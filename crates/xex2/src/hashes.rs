//! Hash computations over XEX regions (header_hash, import_table_hash).
//!
//! The XEX security info's `image_info` stores several SHA-1 digests that the
//! kernel verifies during load:
//! - `image_info.header_hash` covers the bytes from the end of SecurityInfo
//!   through the start of the PE payload (i.e. the optional-header data blobs
//!   and any padding).
//! - `image_info.import_table_hash` covers the `ImportLibraries` optional
//!   header data blob.
//! - `image_info.image_hash` covers the PE payload (chained through page
//!   descriptors; not computed here).
//!
//! Any modification that touches a byte in one of these coverage regions must
//! be accompanied by a matching hash recomputation, or the kernel rejects the
//! XEX.

use sha1::Digest;
use sha1::Sha1;

use crate::header::OptionalHeaderKey;
use crate::header::OptionalHeaderValue;
use crate::header::SecurityInfo;
use crate::header::Sha1Hash;
use crate::header::Xex2Header;

/// Compute `image_info.header_hash` from `source`.
///
/// Per the kernel's XEX loader (`sub_8007bf10`), the hash is SHA-1 over two
/// concatenated regions:
/// 1. `[security_offset + 0x17c, data_offset)` -- the page descriptors plus
///    the optional-header data blobs and any padding.
/// 2. `[0, security_offset + 8)` -- the outer XEX header (including the
///    optional-header index) plus the first two fields of SecurityInfo
///    (`header_size` and `image_size`).
pub fn compute_header_hash(source: &[u8], header: &Xex2Header, _security_info: &SecurityInfo) -> Sha1Hash {
	let sec = header.security_offset as usize;
	let data_off = header.data_offset as usize;
	let mut hasher = Sha1::new();
	hasher.update(&source[sec + 0x17c..data_off]);
	hasher.update(&source[0..sec + 8]);
	Sha1Hash(hasher.finalize().into())
}

/// Compute `image_info.import_table_hash` from the `ImportLibraries` optional
/// header blob.
///
/// The import libraries form a forward-chained hash list: each library's
/// `digest` field stores the SHA-1 of the following library's entry body
/// (skipping the 4-byte `entry_size` prefix). `import_table_hash` is the
/// SHA-1 of the first library's entry body.
///
/// This function assumes the blob's digest chain is consistent with itself
/// (as in an unmodified XEX). For recomputing after edits, use
/// [`rewrite_import_table_hashes`].
///
/// Returns `None` if the XEX has no import table.
pub fn compute_import_table_hash(header: &Xex2Header) -> Option<Sha1Hash> {
	let blob = match header.get_optional_header(OptionalHeaderKey::ImportLibraries)? {
		OptionalHeaderValue::Data(d) => d,
		OptionalHeaderValue::Inline(_) => return None,
	};
	let first = first_library_entry(blob)?;
	Some(hash_entry_body(first))
}

/// Position of one library entry within the ImportLibraries blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LibraryEntrySpan {
	/// Blob-relative offset of the entry's first byte (the entry_size u32).
	pub offset: usize,
	/// Total size of the entry in bytes (including the entry_size prefix).
	pub size: usize,
}

impl LibraryEntrySpan {
	pub fn range(&self) -> std::ops::Range<usize> {
		self.offset..self.offset + self.size
	}
}

/// Walk the ImportLibraries blob and return the span of each library entry
/// in on-disk order.
pub(crate) fn library_entry_offsets(blob: &[u8]) -> Option<Vec<LibraryEntrySpan>> {
	if blob.len() < 12 {
		return None;
	}
	let strtab_size = u32::from_be_bytes(blob[4..8].try_into().ok()?) as usize;
	let lib_count = u32::from_be_bytes(blob[8..12].try_into().ok()?) as usize;
	let mut off = 12 + strtab_size;
	if !off.is_multiple_of(4) {
		off += 4 - (off % 4);
	}

	let mut out = Vec::with_capacity(lib_count);
	for _ in 0..lib_count {
		if off + 4 > blob.len() {
			return None;
		}
		let size = u32::from_be_bytes(blob[off..off + 4].try_into().ok()?) as usize;
		if off + size > blob.len() || size < 4 {
			return None;
		}
		out.push(LibraryEntrySpan { offset: off, size });
		off += size;
	}
	Some(out)
}

fn first_library_entry(blob: &[u8]) -> Option<&[u8]> {
	let spans = library_entry_offsets(blob)?;
	let first = *spans.first()?;
	Some(&blob[first.range()])
}

/// SHA-1 of a library entry's body (bytes 4..entry_size, skipping the size prefix).
pub(crate) fn hash_entry_body(entry: &[u8]) -> Sha1Hash {
	let mut hasher = Sha1::new();
	hasher.update(&entry[4..]);
	Sha1Hash(hasher.finalize().into())
}

/// Rewrite the digest chain of a modified import-libraries blob in place and
/// return the resulting `import_table_hash` (which is the hash of the first
/// library's entry body after rewriting).
///
/// Each library's digest field (bytes +0x04..+0x18 within the entry) is
/// overwritten with the SHA-1 of the *next* library's entry body. The final
/// library's digest is left as-is (it's already consistent with whatever
/// follows it, typically zero). Returns `None` if the blob is malformed.
pub(crate) fn rewrite_import_table_hashes(blob: &mut [u8]) -> Option<Sha1Hash> {
	let spans = library_entry_offsets(blob)?;
	// Walk backwards: compute each library's "next-hash" and store it in the
	// preceding library's digest field.
	for pair in spans.windows(2).rev() {
		let next_hash = hash_entry_body(&blob[pair[1].range()]);
		let digest_off = pair[0].offset + 4;
		blob[digest_off..digest_off + 20].copy_from_slice(&*next_hash);
	}
	let first = *spans.first()?;
	Some(hash_entry_body(&blob[first.range()]))
}
