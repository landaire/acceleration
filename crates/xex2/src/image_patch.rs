//! Verified patches for an extracted XEX image.
//!
//! The format is big-endian and self-contained:
//! `XEX2PAT\0`, version, source image base, source image size, source SHA-1,
//! record count, followed by records of virtual address, byte count, and
//! replacement bytes. Records after the source image extend it with zero fill.

use byteorder::BigEndian;
use byteorder::ByteOrder;
use rootcause::IntoReport;
use sha1::Digest;
use sha1::Sha1;
use xenon_types::VirtualAddress;

use crate::error::Result;
use crate::error::Xex2Error;
use crate::header::Sha1Hash;

const MAGIC: &[u8; 8] = b"XEX2PAT\0";
const VERSION: u32 = 1;
const HEADER_SIZE: usize = 44;
const RECORD_HEADER_SIZE: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImagePatchRecord<'a> {
	pub address: VirtualAddress,
	pub bytes: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePatch<'a> {
	pub image_base: VirtualAddress,
	pub image_size: u32,
	pub source_hash: Sha1Hash,
	pub records: Vec<ImagePatchRecord<'a>>,
}

impl<'a> ImagePatch<'a> {
	pub fn parse(data: &'a [u8]) -> Result<Self> {
		if data.len() < HEADER_SIZE {
			return Err(truncated(0, HEADER_SIZE, data.len()));
		}
		if &data[..MAGIC.len()] != MAGIC {
			return Err(Xex2Error::InvalidImagePatchMagic.into_report());
		}
		let version = BigEndian::read_u32(&data[8..12]);
		if version != VERSION {
			return Err(Xex2Error::UnsupportedImagePatchVersion { version }.into_report());
		}

		let image_base = VirtualAddress(BigEndian::read_u32(&data[12..16]));
		let image_size = BigEndian::read_u32(&data[16..20]);
		let source_hash = Sha1Hash(data[20..40].try_into().unwrap());
		let record_count = BigEndian::read_u32(&data[40..44]) as usize;
		let available = data.len() - HEADER_SIZE;
		let record_headers_size = record_count.saturating_mul(RECORD_HEADER_SIZE);
		if record_count > available / RECORD_HEADER_SIZE {
			return Err(truncated(HEADER_SIZE, record_headers_size, available));
		}
		let mut offset = HEADER_SIZE;
		let mut records = Vec::new();
		records.try_reserve(record_count).map_err(|_| Xex2Error::AllocationFailed {
			size: record_count.saturating_mul(size_of::<ImagePatchRecord<'a>>()),
		})?;
		for _ in 0..record_count {
			let header_end = offset.checked_add(RECORD_HEADER_SIZE).ok_or_else(|| {
				Xex2Error::ImagePatchTruncated { offset, needed: RECORD_HEADER_SIZE, available: data.len() - offset }
					.into_report()
			})?;
			if header_end > data.len() {
				return Err(truncated(offset, RECORD_HEADER_SIZE, data.len() - offset));
			}
			let address = VirtualAddress(BigEndian::read_u32(&data[offset..offset + 4]));
			let len = BigEndian::read_u32(&data[offset + 4..header_end]) as usize;
			offset = header_end;
			let end = offset.checked_add(len).ok_or_else(|| truncated(offset, len, data.len() - offset))?;
			if end > data.len() {
				return Err(truncated(offset, len, data.len() - offset));
			}
			records.push(ImagePatchRecord { address, bytes: &data[offset..end] });
			offset = end;
		}
		if offset != data.len() {
			return Err(Xex2Error::ImagePatchTrailingData { len: data.len() - offset }.into_report());
		}

		Ok(Self { image_base, image_size, source_hash, records })
	}

	pub fn apply(&self, image: &mut Vec<u8>, load_address: VirtualAddress) -> Result<()> {
		if self.image_base != load_address {
			return Err(Xex2Error::ImagePatchBase { expected: self.image_base.0, actual: load_address.0 }.into_report());
		}
		if self.image_size as usize != image.len() {
			return Err(
				Xex2Error::ImagePatchSize { expected: self.image_size as usize, actual: image.len() }.into_report()
			);
		}
		let actual_hash = Sha1Hash(Sha1::digest(&*image).into());
		if actual_hash != self.source_hash {
			return Err(Xex2Error::ImagePatchHash { expected: self.source_hash, actual: actual_hash }.into_report());
		}

		let mut spans = Vec::with_capacity(self.records.len());
		let mut target_size = image.len();
		let address_space_size = 0x1000_0000usize - (load_address.0 as usize & 0x0FFF_FFFF);
		for record in &self.records {
			let offset = record.address.0.checked_sub(load_address.0).ok_or_else(|| {
				Xex2Error::ImagePatchAddress { address: record.address.0, len: record.bytes.len() }.into_report()
			})? as usize;
			let end = offset.checked_add(record.bytes.len()).ok_or_else(|| {
				Xex2Error::ImagePatchAddress { address: record.address.0, len: record.bytes.len() }.into_report()
			})?;
			if end > address_space_size {
				return Err(
					Xex2Error::ImagePatchAddress { address: record.address.0, len: record.bytes.len() }.into_report()
				);
			}
			target_size = target_size.max(end);
			spans.push((offset, end));
		}
		spans.sort_unstable();
		if spans.windows(2).any(|pair| pair[0].1 > pair[1].0) {
			return Err(Xex2Error::ImagePatchOverlap.into_report());
		}
		image.try_reserve(target_size - image.len()).map_err(|_| Xex2Error::AllocationFailed { size: target_size })?;
		image.resize(target_size, 0);

		for record in &self.records {
			let offset = (record.address.0 - load_address.0) as usize;
			let end = offset + record.bytes.len();
			image[offset..end].copy_from_slice(record.bytes);
		}
		Ok(())
	}
}

fn truncated(offset: usize, needed: usize, available: usize) -> rootcause::Report<Xex2Error> {
	Xex2Error::ImagePatchTruncated { offset, needed, available }.into_report()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn encoded_patch(image: &[u8], records: &[(u32, &[u8])]) -> Vec<u8> {
		let mut out = Vec::new();
		out.extend_from_slice(MAGIC);
		out.extend_from_slice(&VERSION.to_be_bytes());
		out.extend_from_slice(&0x9200_0000u32.to_be_bytes());
		out.extend_from_slice(&(image.len() as u32).to_be_bytes());
		out.extend_from_slice(&Sha1::digest(image));
		out.extend_from_slice(&(records.len() as u32).to_be_bytes());
		for (address, bytes) in records {
			out.extend_from_slice(&address.to_be_bytes());
			out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
			out.extend_from_slice(bytes);
		}
		out
	}

	#[test]
	fn applies_verified_records() {
		let mut image = vec![0u8; 16];
		let bytes = encoded_patch(&image, &[(0x9200_0004, &[1, 2, 3, 4]), (0x9200_000c, &[5, 6])]);
		ImagePatch::parse(&bytes).unwrap().apply(&mut image, VirtualAddress(0x9200_0000)).unwrap();
		assert_eq!(&image, &[0, 0, 0, 0, 1, 2, 3, 4, 0, 0, 0, 0, 5, 6, 0, 0]);
	}

	#[test]
	fn extends_source_image_with_zero_fill() {
		let mut image = vec![0xAA; 4];
		let bytes = encoded_patch(&image, &[(0x9200_0008, &[1, 2])]);
		ImagePatch::parse(&bytes).unwrap().apply(&mut image, VirtualAddress(0x9200_0000)).unwrap();
		assert_eq!(&image, &[0xAA, 0xAA, 0xAA, 0xAA, 0, 0, 0, 0, 1, 2]);
	}

	#[test]
	fn rejects_wrong_source_image() {
		let source = vec![0u8; 16];
		let bytes = encoded_patch(&source, &[(0x9200_0004, &[1])]);
		let mut different = vec![1u8; 16];
		let error = ImagePatch::parse(&bytes).unwrap().apply(&mut different, VirtualAddress(0x9200_0000)).unwrap_err();
		assert!(matches!(error.current_context(), Xex2Error::ImagePatchHash { .. }));
	}

	#[test]
	fn rejects_record_count_larger_than_the_patch() {
		let image = [0u8; 16];
		let mut bytes = encoded_patch(&image, &[]);
		bytes[40..44].copy_from_slice(&u32::MAX.to_be_bytes());

		let error = ImagePatch::parse(&bytes).unwrap_err();

		assert!(matches!(error.current_context(), Xex2Error::ImagePatchTruncated { .. }));
	}
}
