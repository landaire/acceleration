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

use rootcause::IntoReport;
use sha1::Digest;
use sha1::Sha1;
use xecrypt::ConsoleKind;
use xecrypt::RsaKeyKind;

use crate::Xex2;
use crate::error::Result;
use crate::error::Xex2Error;
use crate::header::OptionalHeaderKey;
use crate::header::OptionalHeaderValue;
use crate::header::SecurityInfo;
use crate::header::Sha1Hash;
use crate::header::Xex2Header;
use crate::opt::ImageFlags;
use crate::writer::TargetMachine;

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

/// Verify the headers the way the console does before it loads a module:
/// the SecurityInfo bounds, the RSA signature over ImageInfo, the header
/// hash and the image size alignment (`XexpVerifyXexHeaders`, kernel 21256
/// `0x800a08d8`), then the load address (`XexpLoadfile`, `0x800a3260`, and
/// hypercall `0x11`, hypervisor 17559 `0x2c990`). Returns the console type
/// whose executable key verifies the signature.
pub fn verify_headers(source: &[u8], xex: &Xex2) -> Result<TargetMachine> {
	const MIN_SECURITY_INFO_SIZE: u32 = 0x184;
	const IMAGE_INFO_OFFSET: usize = 0x108;
	const MIN_IMAGE_INFO_SIZE: u32 = 0x104;
	const MAX_IMAGE_INFO_SIZE: u32 = 0x174;
	const SIGNED_PREFIX: u32 = 0x100;

	let header = &xex.header;
	let security_info = &xex.security_info;
	let header_size = header.data_offset;
	let security_offset = header.security_offset;
	let security_fits = header_size as usize <= source.len()
		&& security_offset <= header_size
		&& header_size - security_offset >= 4
		&& (MIN_SECURITY_INFO_SIZE..=header_size - security_offset).contains(&security_info.header_size);
	if !security_fits {
		return Err(Xex2Error::SecurityInfoOutOfBounds {
			security_offset,
			security_size: security_info.header_size,
			header_size,
		}
		.into_report());
	}

	let info_size = security_info.image_info.info_size;
	if !(MIN_IMAGE_INFO_SIZE..=MAX_IMAGE_INFO_SIZE).contains(&info_size) {
		return Err(Xex2Error::ImageInfoSize { size: info_size }.into_report());
	}

	let image_info_start = security_offset as usize + IMAGE_INFO_OFFSET;
	let image_info = &source[image_info_start..image_info_start + (info_size - SIGNED_PREFIX) as usize];
	let digest = xecrypt::symmetric::xe_crypt_rot_sum_sha(image_info, &[]);
	let image_flags = security_info.image_info.image_flags;
	let (salt, salt_name) = if image_flags.contains(ImageFlags::REVOCATION_SALT) {
		(xecrypt::xex_sig::SALT_XEX_REVOCATION, "XBOXREVXEX")
	} else {
		(xecrypt::xex_sig::SALT_XEX, "XBOX360XEX")
	};

	let mut machine = None;
	for (candidate, console_kind) in
		[(TargetMachine::Retail, ConsoleKind::Retail), (TargetMachine::Devkit, ConsoleKind::Devkit)]
	{
		let key = RsaKeyKind::Executable
			.public_key(console_kind)
			.map_err(|_| Xex2Error::SignatureInvalid { salt: salt_name }.into_report())?;
		if xecrypt::xex_sig::verify(&security_info.rsa_signature, &digest, salt, &key) {
			machine = Some(candidate);
			break;
		}
	}

	let Some(machine) = machine else {
		return Err(Xex2Error::SignatureInvalid { salt: salt_name }.into_report());
	};

	let computed = compute_header_hash(source, header, security_info);
	let expected = security_info.image_info.header_hash;
	if computed != expected {
		return Err(Xex2Error::HeaderHashMismatch { expected, computed }.into_report());
	}

	let small_pages = image_flags.contains(ImageFlags::SMALL_PAGES);
	let page_size = if small_pages { 0x1000 } else { 0x1_0000 };
	let image_size = security_info.image_size;
	if !image_size.is_multiple_of(page_size) {
		return Err(Xex2Error::ImageSizeAlignment { image_size, page_size }.into_report());
	}

	check_load_address(security_info.image_info.load_address.0, image_size, small_pages)?;

	Ok(machine)
}

fn check_load_address(address: u32, image_size: u32, small_pages: bool) -> Result<()> {
	const REGION_SIZE: u32 = 0x1000_0000;
	const SMALL_PAGE_REGION: u32 = 0x9000_0000;

	if address & 0xFFFF != 0 {
		return Err(Xex2Error::LoadAddressAlignment { address }.into_report());
	}

	if address & 0xE000_0000 != 0x8000_0000 {
		return Err(Xex2Error::LoadAddressRegion { address }.into_report());
	}

	if (address & 0xF000_0000 == SMALL_PAGE_REGION) != small_pages {
		return Err(Xex2Error::LoadAddressPageSize { address, small_pages }.into_report());
	}

	if image_size > REGION_SIZE - (address & (REGION_SIZE - 1)) {
		return Err(Xex2Error::ImageCrossesRegion { address, image_size }.into_report());
	}

	Ok(())
}

/// Check the header layout rules the kernel applies to XEX headers it reads
/// from a file (`0x800a2cf8`): the header size is a multiple of 0x800 from
/// 0x800 to 0x10000, and the reserved word at offset 0xC is zero.
pub fn check_file_layout(source: &[u8], xex: &Xex2) -> Result<()> {
	const MAX_HEADER_SIZE: u32 = 0x1_0000;
	const SECTOR_SIZE: u32 = 0x800;

	let size = xex.header.data_offset;
	if !(SECTOR_SIZE..=MAX_HEADER_SIZE).contains(&size) || !size.is_multiple_of(SECTOR_SIZE) {
		return Err(Xex2Error::HeaderSize { size }.into_report());
	}

	let value = u32::from_be_bytes(source[0xC..0x10].try_into().unwrap());
	if value != 0 {
		return Err(Xex2Error::ReservedHeaderWord { value }.into_report());
	}

	Ok(())
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
