//! Page-descriptor generation for the inner PE image.
//!
//! The XEX security info stores a table of per-page SHA-1 hashes that the
//! kernel uses to verify the decompressed PE image. Each descriptor is 24
//! bytes: `u32 (page_count << 4 | flags)` followed by 20 bytes of SHA-1.
//!
//! **Caveat**: the exact hash-chain formula the kernel expects is not fully
//! reverse-engineered. Empirically neither plain SHA-1 of each page range nor
//! a simple Merkle chain matches the stored values in production XEXs -- the
//! kernel appears to use HMAC-SHA1 with a key derived from the XEX's session
//! key (per the `XexTitleHash*` routines in `xboxkrnl.exe`). The values this
//! module produces are sufficient for tools/emulators that don't strictly
//! enforce page integrity (Xenia, devkit/JTAG consoles with patched verifiers)
//! but a retail Xbox 360 is unlikely to accept them even with a valid RSA
//! signature.

use sha1::Digest;
use sha1::Sha1;

/// Flag value the kernel uses on descriptors for regular hashed pages.
pub const FLAG_HASHED: u32 = 0x1;
/// Flag value the kernel uses on descriptors for executable pages.
pub const FLAG_EXECUTABLE: u32 = 0x3;

/// A single 24-byte page descriptor entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageDescriptor {
	/// Number of `page_size` pages this descriptor covers.
	pub page_count: u32,
	/// Low 4 bits of the descriptor info word.
	pub flags: u32,
	/// SHA-1 hash of the covered page range.
	pub hash: [u8; 20],
}

impl PageDescriptor {
	/// 24-byte on-disk representation: info word (big-endian) + hash.
	pub fn to_bytes(&self) -> [u8; 24] {
		let info = (self.page_count << 4) | (self.flags & 0xF);
		let mut buf = [0u8; 24];
		buf[0..4].copy_from_slice(&info.to_be_bytes());
		buf[4..24].copy_from_slice(&self.hash);
		buf
	}

	pub fn from_bytes(bytes: &[u8; 24]) -> Self {
		let info = u32::from_be_bytes(bytes[0..4].try_into().unwrap());
		let hash: [u8; 20] = bytes[4..24].try_into().unwrap();
		Self { page_count: info >> 4, flags: info & 0xF, hash }
	}
}

/// Best-guess page descriptor generation: SHA-1 of each page range, 64KB
/// per descriptor.
///
/// Takes an optional template of `(page_count, flags)` groupings from the
/// original XEX. If provided, the output preserves the grouping and flags;
/// otherwise a single [`FLAG_HASHED`] descriptor covers the whole image.
pub fn generate(pe: &[u8], page_size: u32, template: Option<&[(u32, u32)]>) -> (Vec<PageDescriptor>, [u8; 20]) {
	let page_size = page_size as usize;
	let descriptors = match template {
		Some(t) => hash_with_template(pe, page_size, t),
		None => hash_whole_image(pe, page_size),
	};
	let image_hash = compute_image_hash(pe);
	(descriptors, image_hash)
}

fn hash_with_template(pe: &[u8], page_size: usize, template: &[(u32, u32)]) -> Vec<PageDescriptor> {
	let mut offset = 0usize;
	let mut out = Vec::with_capacity(template.len());
	for &(page_count, flags) in template {
		let len = (page_count as usize) * page_size;
		let end = (offset + len).min(pe.len());
		out.push(PageDescriptor { page_count, flags, hash: sha1_of(&pe[offset..end]) });
		offset = end;
	}
	out
}

fn hash_whole_image(pe: &[u8], page_size: usize) -> Vec<PageDescriptor> {
	let total_pages = pe.len().div_ceil(page_size) as u32;
	vec![PageDescriptor { page_count: total_pages, flags: FLAG_HASHED, hash: sha1_of(pe) }]
}

fn sha1_of(bytes: &[u8]) -> [u8; 20] {
	let mut h = Sha1::new();
	h.update(bytes);
	h.finalize().into()
}

/// Best-guess `image_info.image_hash`: SHA-1 over the whole PE image.
pub fn compute_image_hash(pe: &[u8]) -> [u8; 20] {
	sha1_of(pe)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn roundtrip_bytes() {
		let d = PageDescriptor { page_count: 16, flags: 0x3, hash: [0xAB; 20] };
		let bytes = d.to_bytes();
		let parsed = PageDescriptor::from_bytes(&bytes);
		assert_eq!(d, parsed);
	}

	#[test]
	fn single_descriptor_without_template() {
		let pe = vec![0u8; 64 * 1024 * 3];
		let (descs, _) = generate(&pe, 0x10000, None);
		assert_eq!(descs.len(), 1);
		assert_eq!(descs[0].page_count, 3);
		assert_eq!(descs[0].flags, FLAG_HASHED);
	}

	#[test]
	fn template_preserves_shape() {
		let pe = vec![0xAAu8; 4 * 0x10000];
		let template = &[(2, 0x3), (2, 0x1)];
		let (descs, _) = generate(&pe, 0x10000, Some(template));
		assert_eq!(descs.len(), 2);
		assert_eq!(descs[0].page_count, 2);
		assert_eq!(descs[0].flags, 0x3);
		assert_eq!(descs[1].page_count, 2);
		assert_eq!(descs[1].flags, 0x1);
	}
}
