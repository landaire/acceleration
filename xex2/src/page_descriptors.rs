//! Page-descriptor generation and verification for the inner PE image.
//!
//! The XEX security info stores a table of per-page descriptors that the
//! hypervisor uses to verify each page of the decompressed PE image. Each
//! descriptor is 24 bytes: `u32 (page_count << 4 | flags)` + `[u8; 20]` hash.
//!
//! # Verification chain (reverse-engineered from `HvxLoadImageData` in the HV)
//!
//! The HV maintains a running "expected" state initialized to
//! `image_info.image_hash`. For each descriptor `i`:
//!
//! 1. `H[i] = SHA1(page_data[i] || descriptor[i].bytes(24))`
//! 2. Assert `H[i] == expected[i]`
//! 3. `expected[i+1] = descriptor[i].stored_hash` (bytes 4..24)
//!
//! This is a forward-chained hash that anchors at `image_info.image_hash`.
//!
//! # Generation (inverse)
//!
//! Work backwards from a terminator hash (nothing reads the last descriptor's
//! stored_hash, so any value works -- we use zeros):
//!
//! - `descriptor[N-1].stored_hash = zeros`
//! - `descriptor[i-1].stored_hash = SHA1(page_data[i] || descriptor[i].bytes(24))`
//! - `image_info.image_hash = SHA1(page_data[0] || descriptor[0].bytes(24))`

use sha1::Digest;
use sha1::Sha1;

use crate::error::Result;
use crate::error::Xex2Error;
use crate::header::SecurityInfo;
use crate::header::Sha1Hash;
use crate::header::Xex2Header;
use crate::opt::ImageFlags;
use rootcause::IntoReport;

/// Common flag values observed in production XEXs. The kernel/HV doesn't
/// strictly enforce specific values for most bits; they group descriptors
/// with similar protection/memory attributes.
pub(crate) const FLAG_HASHED: u32 = 0x1;
#[cfg(test)]
pub(crate) const FLAG_EXECUTABLE: u32 = 0x3;

/// A single 24-byte page descriptor entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PageDescriptor {
	/// Number of `page_size` pages this descriptor covers.
	pub page_count: u32,
	/// Low 4 bits of the descriptor info word.
	pub flags: u32,
	/// Hash field. For descriptor `i`, this stores `expected[i+1]` in the
	/// HV's verification chain.
	pub hash: Sha1Hash,
}

impl PageDescriptor {
	pub(crate) fn to_bytes(self) -> [u8; 24] {
		let info = (self.page_count << 4) | (self.flags & 0xF);
		let mut buf = [0u8; 24];
		buf[0..4].copy_from_slice(&info.to_be_bytes());
		buf[4..24].copy_from_slice(&*self.hash);
		buf
	}

	#[cfg(test)]
	pub(crate) fn from_bytes(bytes: &[u8; 24]) -> Self {
		let info = u32::from_be_bytes(bytes[0..4].try_into().unwrap());
		let hash: [u8; 20] = bytes[4..24].try_into().unwrap();
		Self { page_count: info >> 4, flags: info & 0xF, hash: Sha1Hash(hash) }
	}
}

fn page_size_for(flags: ImageFlags) -> u32 {
	if flags.contains(ImageFlags::SMALL_PAGES) { 0x1000 } else { 0x10000 }
}

/// Hash a page of exactly `declared_len` bytes followed by the 24-byte
/// descriptor. If the actual `page` is shorter than `declared_len` (last
/// page of the image), pad with zeros -- the HV sees virtual memory which
/// is zero-initialized past image_size.
fn sha1_page_and_descriptor(page: &[u8], declared_len: usize, descriptor_bytes: &[u8; 24]) -> Sha1Hash {
	let mut h = Sha1::new();
	h.update(page);
	if page.len() < declared_len {
		let padding = vec![0u8; declared_len - page.len()];
		h.update(&padding);
	}
	h.update(descriptor_bytes);
	Sha1Hash(h.finalize().into())
}

/// Shape of one descriptor slot in [`generate`]'s `template`: how many
/// `page_size` pages it covers and what `flags` word to store. Using a named
/// struct rather than a bare `(u32, u32)` rules out the easy-to-miss
/// `(flags, page_count)` swap.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DescriptorSlot {
	pub page_count: u32,
	pub flags: u32,
}

/// Output of [`generate`]: the new `page_descriptors` array plus the
/// `image_info.image_hash` that roots its hash chain.
pub(crate) struct GeneratedDescriptors {
	pub descriptors: Vec<PageDescriptor>,
	pub image_hash: Sha1Hash,
}

/// Generate descriptors + `image_info.image_hash` for a new PE image.
///
/// `template` controls descriptor grouping (count/flags per entry). Pass
/// `None` to use a single descriptor covering the whole image with
/// [`FLAG_HASHED`].
pub(crate) fn generate(pe: &[u8], page_size: u32, template: Option<&[DescriptorSlot]>) -> GeneratedDescriptors {
	let page_size_usize = page_size as usize;
	let owned_template: Vec<DescriptorSlot>;
	let template = match template {
		Some(t) => t,
		None => {
			let total = pe.len().div_ceil(page_size_usize) as u32;
			owned_template = vec![DescriptorSlot { page_count: total, flags: FLAG_HASHED }];
			&owned_template
		}
	};

	// Slice PE into per-descriptor ranges. Each range's *declared* length is
	// `count * page_size`; the actual PE byte span may be shorter for the
	// last descriptor, in which case hashing pads with zeros.
	struct Range {
		start: usize,
		end: usize,
		declared_len: usize,
	}
	let mut ranges: Vec<Range> = Vec::with_capacity(template.len());
	let mut cursor = 0usize;
	for slot in template {
		let declared = (slot.page_count as usize) * page_size_usize;
		let end = (cursor + declared).min(pe.len());
		ranges.push(Range { start: cursor, end, declared_len: declared });
		cursor = end;
	}

	// Last descriptor's stored_hash is the "terminator" -- nothing reads it
	// after the loop, so any value works. Zeros is simplest.
	let mut descriptors: Vec<PageDescriptor> = template
		.iter()
		.map(|s| PageDescriptor { page_count: s.page_count, flags: s.flags, hash: Sha1Hash::ZERO })
		.collect();

	// Work backwards: descriptor[i-1].stored_hash = SHA1(page_data[i] || descriptor[i].bytes).
	for i in (1..descriptors.len()).rev() {
		let desc_bytes = descriptors[i].to_bytes();
		let r = &ranges[i];
		descriptors[i - 1].hash = sha1_page_and_descriptor(&pe[r.start..r.end], r.declared_len, &desc_bytes);
	}

	// image_info.image_hash = SHA1(page_data[0] || descriptor[0].bytes).
	let desc0_bytes = descriptors[0].to_bytes();
	let r0 = &ranges[0];
	let image_hash = sha1_page_and_descriptor(&pe[r0.start..r0.end], r0.declared_len, &desc0_bytes);

	GeneratedDescriptors { descriptors, image_hash }
}

/// Verify the complete page-descriptor chain against a decompressed PE
/// image and an XEX's `source` bytes.
pub fn verify_chain(pe: &[u8], header: &Xex2Header, security_info: &SecurityInfo, source: &[u8]) -> Result<()> {
	let sec = header.security_offset as usize;
	let count = security_info.page_descriptor_count as usize;
	let pd_off = sec + 0x184;
	let page_size = page_size_for(security_info.image_info.image_flags) as usize;

	let mut expected: Sha1Hash = security_info.image_info.image_hash;
	let mut pe_cursor = 0usize;

	for i in 0..count {
		if pd_off + (i + 1) * 24 > source.len() {
			return Err(Xex2Error::HashMismatch { block_index: i }.into_report());
		}
		let desc_bytes: [u8; 24] = source[pd_off + i * 24..pd_off + (i + 1) * 24].try_into().unwrap();
		let page_count = u32::from_be_bytes(desc_bytes[0..4].try_into().unwrap()) >> 4;
		let declared_len = (page_count as usize) * page_size;
		let range_end = (pe_cursor + declared_len).min(pe.len());

		let computed = sha1_page_and_descriptor(&pe[pe_cursor..range_end], declared_len, &desc_bytes);
		if computed != expected {
			return Err(Xex2Error::HashMismatch { block_index: i }.into_report());
		}
		expected.0.copy_from_slice(&desc_bytes[4..24]);
		pe_cursor = range_end;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn roundtrip_bytes() {
		let d = PageDescriptor { page_count: 16, flags: 0x3, hash: Sha1Hash([0xAB; 20]) };
		let bytes = d.to_bytes();
		let parsed = PageDescriptor::from_bytes(&bytes);
		assert_eq!(d, parsed);
	}

	#[test]
	fn generate_chain_self_verifies() {
		// Build a synthetic PE + template, generate descriptors, confirm the
		// HV-style verification walk accepts them.
		let pe: Vec<u8> = (0..(4 * 0x10000)).map(|i| (i & 0xFF) as u8).collect();
		let template = &[
			DescriptorSlot { page_count: 2, flags: FLAG_EXECUTABLE },
			DescriptorSlot { page_count: 2, flags: FLAG_HASHED },
		];
		let GeneratedDescriptors { descriptors, image_hash } = generate(&pe, 0x10000, Some(template));

		let mut expected = image_hash;
		let mut cursor = 0usize;
		for d in &descriptors {
			let bytes = d.to_bytes();
			let declared = (d.page_count as usize) * 0x10000;
			let end = (cursor + declared).min(pe.len());
			let computed = sha1_page_and_descriptor(&pe[cursor..end], declared, &bytes);
			assert_eq!(computed, expected);
			expected = d.hash;
			cursor = end;
		}
	}

	#[test]
	fn default_template_single_descriptor() {
		let pe = vec![0u8; 64 * 1024 * 3];
		let descs = generate(&pe, 0x10000, None).descriptors;
		assert_eq!(descs.len(), 1);
		assert_eq!(descs[0].page_count, 3);
		assert_eq!(descs[0].flags, FLAG_HASHED);
	}
}
