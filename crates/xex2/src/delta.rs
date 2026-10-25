//! XEX delta patches (XEXP files).
//!
//! A patch file updates a base XEX to a new version. When the kernel loads a
//! module and finds a patch file beside it (`XexpLoadfile` at kernel 21256
//! `0x800a2f90`), it builds the new module in memory:
//!
//! 1. The headers: a range of the base headers is copied into a buffer of
//!    the new header size, then the delta records of the patch descriptor
//!    (optional header `0x5FF`) are applied (`XexDeltaDecompressHeaders`,
//!    `0x800a6430`). The new headers carry their own signature.
//! 2. The image: a range of the base image is loaded into the new image,
//!    then the delta records in the patch file's data are applied
//!    (`XexpLoadDeltaCompressedImage`, `0x800a5ff8`). The new headers' page
//!    descriptors verify the result.
//!
//! The base image key is wrapped under the new module's session key in the
//! descriptor, and the patch data key under the same session key in the
//! patch file's ImageInfo.
//!
//! A delta record (`0x800a5ed8`) is a big-endian source offset, target
//! offset, 16-bit size and 16-bit compressed size, all within one buffer.
//! Compressed size 0 zeroes the target, 1 copies the source to the target,
//! and any other value is that many bytes of LZX data that expand to the
//! target with the source as window history. A record of size 0 ends the
//! list.

use rootcause::IntoReport;
use sha1::Digest;
use sha1::Sha1;
use xenon_lzx::Decoder;

use crate::Xex2;
use crate::basefile;
use crate::basefile::BlockChain;
use crate::crypto;
use crate::crypto::CbcDecryptor;
use crate::error::Result;
use crate::error::Xex2Error;
use crate::header::AesKey;
use crate::header::CompressionType;
use crate::header::EncryptionType;
use crate::header::OptionalHeaderKey;
use crate::header::Sha1Hash;
use crate::opt::ModuleFlags;
use crate::writer::TargetMachine;
use xenon_types::Version;

/// Window and frame limit of the header delta decoder (`0x800a660c`).
const HEADER_WINDOW_SIZE: usize = 0x8000;
const RECORD_HEADER_SIZE: usize = 12;
/// Offset of the records in a delta data block, after the next block's size
/// and SHA-1.
const BLOCK_RECORDS_OFFSET: usize = 24;

/// A byte range of a buffer, as offset and size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Span {
	pub offset: u32,
	pub size: u32,
}

/// The delta patch descriptor, optional header `0x5FF` of a patch file.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DeltaPatchDescriptor {
	/// Version of the patched module.
	pub target_version: Version,
	/// Version of the base module. The kernel stores it as the base version
	/// in the patched module's execution info after loading.
	pub source_version: Version,
	/// SHA-1 of the base file's RSA signature.
	pub base_signature_digest: Sha1Hash,
	/// Session key of the base image data, encrypted with the patched
	/// module's session key.
	pub base_image_key: AesKey,
	/// Size of the patched headers.
	pub target_header_size: u32,
	/// Bytes of the base headers copied into the patched headers.
	pub header_source: Span,
	pub header_target_offset: u32,
	/// Bytes of the base image loaded into the patched image.
	pub image_source: Span,
	pub image_target_offset: u32,
	/// Delta records applied to the patched headers.
	#[cfg_attr(feature = "serde", serde(skip))]
	pub header_records: Vec<u8>,
}

impl DeltaPatchDescriptor {
	/// Parse the optional header data, which begins with its size.
	pub fn parse(data: &[u8]) -> Result<Self> {
		const FIXED_SIZE: usize = 0x4C;

		if data.len() < FIXED_SIZE {
			return Err(Xex2Error::DeltaDescriptorTruncated { len: data.len() }.into_report());
		}

		let word = |offset: usize| u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap());
		let declared = word(0) as usize;
		if declared < FIXED_SIZE || declared > data.len() {
			return Err(Xex2Error::DeltaDescriptorSize { declared: word(0), available: data.len() }.into_report());
		}

		Ok(Self {
			target_version: Version::from(word(0x04)),
			source_version: Version::from(word(0x08)),
			base_signature_digest: Sha1Hash(data[0x0C..0x20].try_into().unwrap()),
			base_image_key: AesKey(data[0x20..0x30].try_into().unwrap()),
			target_header_size: word(0x30),
			header_source: Span { offset: word(0x34), size: word(0x38) },
			header_target_offset: word(0x3C),
			image_source: Span { offset: word(0x40), size: word(0x44) },
			image_target_offset: word(0x48),
			header_records: data[FIXED_SIZE..declared].to_vec(),
		})
	}
}

/// A module rebuilt from a base XEX and a delta patch.
#[derive(Debug)]
pub struct PatchedModule {
	/// The patched XEX headers: the first `data_offset` bytes of the new
	/// module's XEX file, signed by the patch author.
	pub headers: Vec<u8>,
	/// The loaded image (basefile) of the new module.
	pub image: Vec<u8>,
	/// The console type whose keys sign and decrypt the patched module.
	pub machine: TargetMachine,
}

impl PatchedModule {
	/// A patched module produced earlier, checked as [`apply_delta_patch`]
	/// checks its result: the headers' signature and hash, and the image's
	/// page descriptors.
	pub fn from_parts(headers: Vec<u8>, image: Vec<u8>) -> Result<Self> {
		let xex = Xex2::parse(&headers)?;
		let machine = crate::hashes::verify_headers(&headers, &xex)?;
		let expected = xex.security_info.image_size as usize;
		if image.len() != expected {
			return Err(Xex2Error::InvalidBasefileSize { expected, got: image.len() }.into_report());
		}

		crate::page_descriptors::verify_chain(&image, &xex.header, &xex.security_info, &headers)?;

		Ok(Self { headers, image, machine })
	}

	/// Parse the patched headers.
	pub fn xex(&self) -> Result<Xex2> {
		Xex2::parse(&self.headers)
	}
}

/// The largest header size the kernel accepts in a XEX file (kernel 21256
/// `0x800a2e08`). The kernel sets no limit on patched headers; it fails when
/// it cannot allocate them.
pub const FILE_HEADER_LIMIT: u32 = 0x1_0000;

/// Apply the delta patch `patch` (an XEXP file) to the base XEX `base`,
/// with the checks the kernel and hypervisor make: the header layout,
/// signature and hash of both files, the base signature digest, the patched
/// headers' signature and hash, the SHA-1 of every data block of both files,
/// and the page descriptors of the patched image. Patched headers larger
/// than `max_header_size` are rejected before they are allocated.
pub fn apply_delta_patch(base: &[u8], patch: &[u8], max_header_size: u32) -> Result<PatchedModule> {
	let base_xex = Xex2::parse(base)?;
	let patch_xex = Xex2::parse(patch)?;

	// Both files are read through `0x800a2cf8`, which checks their layout
	// and headers.
	for (source, xex) in [(base, &base_xex), (patch, &patch_xex)] {
		crate::hashes::check_file_layout(source, xex)?;
		crate::hashes::verify_headers(source, xex)?;
	}

	// Kernel 21256 `0x800a305c` and `0x800a31bc`.
	let base_flags = base_xex.header.module_flags;
	if base_flags.intersects(ModuleFlags::PATCH | ModuleFlags::PATCH_DELTA) {
		return Err(Xex2Error::BaseHasPatchFlags { module_flags: base_flags.bits() }.into_report());
	}

	let patch_flags = patch_xex.header.module_flags;
	if !patch_flags.contains(ModuleFlags::PATCH) {
		return Err(Xex2Error::PatchFlagMissing { module_flags: patch_flags.bits() }.into_report());
	}

	let descriptor = patch_xex.header.get_optional_data(OptionalHeaderKey::DeltaPatchDescriptor).ok_or_else(|| {
		Xex2Error::MissingOptionalHeader(OptionalHeaderKey::DeltaPatchDescriptor as u32).into_report()
	})?;
	let descriptor = DeltaPatchDescriptor::parse(descriptor)?;

	let headers = patch_headers(base, &base_xex, &descriptor, max_header_size)?;
	let patched = Xex2::parse(&headers)?;
	let machine = crate::hashes::verify_headers(&headers, &patched)?;

	let image_flags = patched.security_info.image_info.image_flags;
	let master = crypto::master_key(machine, image_flags)
		.ok_or_else(|| Xex2Error::UnknownMasterKey { machine, image_flags: image_flags.bits() }.into_report())?;
	let session_key = crypto::unwrap_key(&patched.security_info.image_info.file_key, &master);

	let mut image = basefile::allocate(patched.security_info.image_size as usize)?;
	load_base_image(base, &base_xex, &descriptor, &session_key, &mut image)?;
	apply_image_delta(patch, &patch_xex, &session_key, &mut image)?;

	crate::page_descriptors::verify_chain(&image, &patched.header, &patched.security_info, &headers)?;

	Ok(PatchedModule { headers, image, machine })
}

fn patch_headers(
	base: &[u8],
	base_xex: &Xex2,
	descriptor: &DeltaPatchDescriptor,
	max_header_size: u32,
) -> Result<Vec<u8>> {
	let base_signature = &base_xex.security_info.rsa_signature;
	if Sha1Hash(Sha1::digest(base_signature).into()) != descriptor.base_signature_digest {
		return Err(Xex2Error::BaseSignatureDigestMismatch.into_report());
	}

	let base_headers = base.get(..base_xex.header.data_offset as usize).ok_or_else(|| {
		Xex2Error::InvalidHeaderOffset { offset: base_xex.header.data_offset, file_size: base.len() }.into_report()
	})?;

	let target_size = descriptor.target_header_size;
	let source = descriptor.header_source;
	let target = descriptor.header_target_offset;
	let source_fits = source.offset <= base_headers.len() as u32
		&& source.size <= base_headers.len() as u32 - source.offset
		&& target <= target_size
		&& source.size <= target_size - target;
	if !source_fits {
		return Err(Xex2Error::DeltaHeaderRange {
			source_span: source,
			target_offset: target,
			base_header_size: base_headers.len() as u32,
			target_header_size: target_size,
		}
		.into_report());
	}

	if target_size > max_header_size {
		return Err(Xex2Error::DeltaHeaderTooLarge { requested: target_size, limit: max_header_size }.into_report());
	}

	let mut headers = basefile::allocate(target_size as usize)?;
	headers[target as usize..(target + source.size) as usize]
		.copy_from_slice(&base_headers[source.offset as usize..(source.offset + source.size) as usize]);

	let mut decoder =
		Decoder::new(vec![0u8; HEADER_WINDOW_SIZE], HEADER_WINDOW_SIZE).map_err(|e| Xex2Error::Lzx(e).into_report())?;
	apply_records(&mut decoder, &mut headers, &descriptor.header_records)?;

	Ok(headers)
}

/// Load the descriptor's range of the base image into the patched image.
/// The base data key comes from the descriptor, not from the base file.
fn load_base_image(
	base: &[u8],
	base_xex: &Xex2,
	descriptor: &DeltaPatchDescriptor,
	session_key: &AesKey,
	image: &mut [u8],
) -> Result<()> {
	let source = descriptor.image_source;
	if source.size == 0 {
		return Ok(());
	}

	let image_size = image.len() as u32;
	let range_error = || {
		Xex2Error::DeltaImageRange { source_span: source, target_offset: descriptor.image_target_offset, image_size }
			.into_report()
	};
	let target = descriptor.image_target_offset as usize;
	let Some(destination) = image.get_mut(target..target + source.size as usize) else {
		return Err(range_error());
	};

	let format = base_xex.header.file_format_info()?;
	if !matches!(format.compression_type, CompressionType::Basic | CompressionType::Normal) {
		return Err(Xex2Error::InvalidCompressionFormat(format.compression_type as u16).into_report());
	}

	let key = match format.encryption_type {
		EncryptionType::None => None,
		EncryptionType::Normal => Some(crypto::unwrap_key(&descriptor.base_image_key, session_key)),
	};

	let Some(range_end) = source.offset.checked_add(source.size) else {
		return Err(range_error());
	};

	basefile::load_image(
		basefile::image_data(base, &base_xex.header)?,
		&format,
		key,
		base_xex.security_info.image_size,
		source.offset..range_end,
		destination,
	)
}

fn apply_image_delta(patch: &[u8], patch_xex: &Xex2, session_key: &AesKey, image: &mut [u8]) -> Result<()> {
	let format = patch_xex.header.file_format_info()?;
	if format.compression_type != CompressionType::Delta {
		return Err(Xex2Error::InvalidCompressionFormat(format.compression_type as u16).into_report());
	}

	let window_size = format.window_size.ok_or_else(|| Xex2Error::InvalidFileFormatInfo.into_report())? as usize;
	let mut decoder = Decoder::new(vec![0u8; window_size], window_size).map_err(|e| Xex2Error::Lzx(e).into_report())?;

	let decryptor = match format.encryption_type {
		EncryptionType::None => None,
		EncryptionType::Normal => {
			Some(CbcDecryptor::new(crypto::unwrap_key(&patch_xex.security_info.image_info.file_key, session_key)))
		}
	};

	let mut chain = BlockChain::new(basefile::image_data(patch, &patch_xex.header)?, &format, decryptor)?;
	while let Some((_, block)) = chain.next_block()? {
		apply_records(&mut decoder, image, &block[BLOCK_RECORDS_OFFSET..])?;
	}

	Ok(())
}

/// Apply delta records to `buffer` (kernel 21256 `0x800a5ed8`). The records
/// end at a record of size zero or at the end of `records`.
fn apply_records(decoder: &mut Decoder<Vec<u8>>, buffer: &mut [u8], records: &[u8]) -> Result<()> {
	let len = buffer.len();
	let mut offset = 0;
	while offset < records.len() {
		let Some(header) = records.get(offset..offset + RECORD_HEADER_SIZE) else {
			return Err(Xex2Error::InvalidDeltaRecord { offset }.into_report());
		};

		let source = u32::from_be_bytes(header[0..4].try_into().unwrap()) as usize;
		let target = u32::from_be_bytes(header[4..8].try_into().unwrap()) as usize;
		let size = usize::from(u16::from_be_bytes([header[8], header[9]]));
		let compressed_size = usize::from(u16::from_be_bytes([header[10], header[11]]));
		if size == 0 {
			return Ok(());
		}

		let in_bounds = source < len && size <= len - source && target < len && size <= len - target;
		if !in_bounds {
			return Err(Xex2Error::InvalidDeltaRecord { offset }.into_report());
		}

		let data_offset = offset + RECORD_HEADER_SIZE;
		match compressed_size {
			0 => buffer[target..target + size].fill(0),
			1 => buffer.copy_within(source..source + size, target),
			_ => {
				let Some(compressed) = records.get(data_offset..data_offset + compressed_size) else {
					return Err(Xex2Error::InvalidDeltaRecord { offset }.into_report());
				};

				decoder.set_reference(&buffer[source..source + size]).map_err(|e| Xex2Error::Lzx(e).into_report())?;
				decoder
					.decompress(compressed, &mut buffer[target..target + size])
					.map_err(|e| Xex2Error::Lzx(e).into_report())?;
				decoder.reset();
			}
		}

		offset = data_offset + if compressed_size > 1 { compressed_size } else { 0 };
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn record(source: u32, target: u32, size: u16, compressed_size: u16) -> Vec<u8> {
		let mut bytes = Vec::new();
		bytes.extend_from_slice(&source.to_be_bytes());
		bytes.extend_from_slice(&target.to_be_bytes());
		bytes.extend_from_slice(&size.to_be_bytes());
		bytes.extend_from_slice(&compressed_size.to_be_bytes());
		bytes
	}

	fn decoder() -> Decoder<Vec<u8>> {
		Decoder::new(vec![0u8; HEADER_WINDOW_SIZE], HEADER_WINDOW_SIZE).unwrap()
	}

	#[test]
	fn zero_and_overlapping_copy_records_apply_in_order() {
		let mut buffer: Vec<u8> = (1..=8).collect();
		let mut records = record(0, 6, 2, 0);
		records.extend(record(0, 2, 4, 1));

		apply_records(&mut decoder(), &mut buffer, &records).unwrap();

		assert_eq!(buffer, [1, 2, 1, 2, 3, 4, 0, 0]);
	}

	#[test]
	fn a_record_of_size_zero_ends_the_list() {
		let mut buffer = vec![0xAAu8; 4];
		let mut records = record(0, 0, 0, 0);
		records.extend(record(0, 0, 4, 0));

		apply_records(&mut decoder(), &mut buffer, &records).unwrap();

		assert_eq!(buffer, [0xAA; 4]);
	}

	#[test]
	fn rejects_records_outside_the_buffer() {
		let mut buffer = vec![0u8; 4];
		let records = record(2, 0, 4, 1);

		let error = apply_records(&mut decoder(), &mut buffer, &records).unwrap_err();

		assert!(matches!(error.current_context(), Xex2Error::InvalidDeltaRecord { offset: 0 }));
	}
}
