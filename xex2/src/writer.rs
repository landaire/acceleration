//! Plan in-place modifications to an XEX file as a [`Patch`].
//!
//! This module handles the small-edit branch of the modification story:
//! flipping ImageInfo bits, zeroing media_id, and re-signing. The edits are
//! length-preserving and local to SecurityInfo, so they're expressed as a
//! [`Patch`] of [`PatchOp::Write`][crate::patch::PatchOp::Write] ops.
//!
//! For full rebuilds (recompression, re-encryption, replacing the inner PE),
//! see [`crate::rebuild::Rebuilder`].
//!
//! # Example
//!
//! ```no_run
//! use xex2::Xex2;
//! use xex2::writer::RemoveLimits;
//!
//! let data = std::fs::read("game.xex").unwrap();
//! let xex = Xex2::parse(&data).unwrap();
//!
//! let mut limits = RemoveLimits::default();
//! limits.region = true;
//! limits.media = true;
//!
//! let patched = xex.modify(&data, &limits).unwrap();
//! std::fs::write("game_patched.xex", patched).unwrap();
//! ```

use byteorder::BigEndian;
use byteorder::ByteOrder;
use rootcause::IntoReport;

use crate::Xex2;
use crate::error::Result;
use crate::error::Xex2Error;
use crate::hashes;
use crate::header::OptionalHeaderKey;
use crate::patch::Patch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetEncryption {
	Unchanged,
	Encrypted,
	Decrypted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetCompression {
	Unchanged,
	Uncompressed,
	Basic,
	Normal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetMachine {
	Unchanged,
	Devkit,
	Retail,
}

#[derive(Debug, Default, Clone)]
pub struct RemoveLimits {
	pub media: bool,
	pub region: bool,
	pub bounding_path: bool,
	pub device_id: bool,
	pub console_id: bool,
	pub dates: bool,
	pub keyvault_privileges: bool,
	pub signed_keyvault_only: bool,
	pub library_versions: bool,
	pub revocation_check: bool,
	pub zero_media_id: bool,
}

impl RemoveLimits {
	pub fn all() -> Self {
		RemoveLimits {
			media: true,
			region: true,
			bounding_path: true,
			device_id: true,
			console_id: true,
			dates: true,
			keyvault_privileges: true,
			signed_keyvault_only: true,
			library_versions: true,
			revocation_check: true,
			zero_media_id: true,
		}
	}

	pub fn any_set(&self) -> bool {
		self.media
			|| self.region
			|| self.bounding_path
			|| self.device_id
			|| self.console_id
			|| self.dates
			|| self.keyvault_privileges
			|| self.signed_keyvault_only
			|| self.library_versions
			|| self.revocation_check
			|| self.zero_media_id
	}
}

// SecurityInfo layout (all fields big-endian):
//
//   +0x000: header_size        (u32)
//   +0x004: image_size         (u32)
//   +0x008: rsa_signature      ([u8; 0x100])
//   +0x108: image_info         (variable, up to 0x74 bytes)
//   +0x17C: page_descriptor_count (u32)
//   +0x180: page_descriptors   (variable)
//
// ImageInfo field offsets (relative to SecurityInfo start):
//   +0x108: info_size            (u32)
//   +0x10C: image_flags          (u32)
//   +0x110: load_address         (u32)
//   +0x114: image_hash           ([u8; 20])
//   +0x128: import_count         (u32)
//   +0x12C: import_hash          ([u8; 20])
//   +0x140: media_id             ([u8; 16])
//   +0x150: file_key             ([u8; 16])
//   +0x160: export_table_address (u32)
//   +0x164: header_hash          ([u8; 20])
//   +0x178: game_regions         (u32)
//   +0x17C: allowed_media        (u32)

const RSA_SIG: usize = 0x008;
const RSA_SIG_LEN: usize = 0x100;
pub(crate) const IMAGE_INFO: usize = 0x108;
pub(crate) const IMAGE_INFO_INFO_SIZE: usize = IMAGE_INFO;
pub(crate) const IMAGE_INFO_IMAGE_FLAGS: usize = IMAGE_INFO + 0x04;
pub(crate) const IMAGE_INFO_IMPORT_TABLE_HASH: usize = IMAGE_INFO + 0x24;
pub(crate) const IMAGE_INFO_MEDIA_ID: usize = IMAGE_INFO + 0x38;
pub(crate) const IMAGE_INFO_HEADER_HASH: usize = IMAGE_INFO + 0x5C;
pub(crate) const IMAGE_INFO_GAME_REGIONS: usize = IMAGE_INFO + 0x70;
pub(crate) const IMAGE_INFO_ALLOWED_MEDIA: usize = IMAGE_INFO + 0x74;

// XEX main header: module_flags at offset 0x04.
const HEADER_MODULE_FLAGS: usize = 0x04;

/// Build a [`Patch`] describing the requested [`RemoveLimits`] edits plus a
/// re-signature.
///
/// Pure function -- reads `source` only. Returns an error for limits that
/// require hash recomputations we haven't implemented yet (header_hash and
/// import_table_hash) or whose semantics aren't pinned down.
///
/// Layout:
/// - Module flag bits (`bounding_path`, `device_id`) live in the XEX header at
///   offset 0x04 and are outside the signed region, so they're plain Write ops
///   with no re-sign.
/// - Image-info edits (region, media, media_id, image_flags bits for
///   `keyvault_privileges` / `signed_keyvault_only`) live inside the RSA-signed
///   image_info; we rebuild the target image_info in a local buffer, hash it,
///   sign it, and emit Write ops for each changed field + the new signature.
pub fn plan_edits(xex: &Xex2, source: &[u8], limits: &RemoveLimits) -> Result<Patch> {
	let mut patch = Patch::new();
	let sec = xex.header.security_offset as usize;

	if limits.revocation_check {
		return Err(Xex2Error::LimitRemovalNotImplemented {
			limit: "revocation_check",
			reason: "flag location in the XEX header is not currently known",
		}
		.into_report());
	}

	// Blob edits (optional-header data region, covered by header_hash).
	// Tracked as `(file_offset, new_bytes)` so we can both emit Write ops and
	// apply the edits to a working copy of source for hash recomputation.
	let mut blob_edits: Vec<(usize, Vec<u8>)> = Vec::new();

	if limits.dates {
		if let Some((off, len)) = xex.header.optional_header_source_range(source, OptionalHeaderKey::DateRange) {
			if len >= 16 {
				// not_before=0, not_after=MAX_FILETIME -> effectively always valid.
				let mut blob = vec![0u8; len];
				blob[8..16].copy_from_slice(&u64::MAX.to_be_bytes());
				blob_edits.push((off, blob));
			}
		}
	}
	if limits.console_id {
		if let Some((off, len)) = xex.header.optional_header_source_range(source, OptionalHeaderKey::ConsoleSerialList) {
			// Zero the entire serial list body -> empty whitelist. Kernel
			// treats an empty list as "no restriction."
			blob_edits.push((off, vec![0u8; len]));
		}
	}
	if limits.library_versions {
		if let Some((off, len)) = xex.header.optional_header_source_range(source, OptionalHeaderKey::ImportLibraries) {
			let mut blob = source[off..off + len].to_vec();
			let offsets = hashes::library_entry_offsets(&blob)
				.ok_or_else(|| Xex2Error::SigningFailed.into_report())?;
			// version_min at entry_offset + 0x20, 4 bytes. Zero it.
			for (entry_off, _entry_size) in &offsets {
				blob[entry_off + 0x20..entry_off + 0x24].fill(0);
			}
			blob_edits.push((off, blob));
		}
	}

	// Module flag edits (outside the signed region).
	if limits.bounding_path || limits.device_id {
		let current = BigEndian::read_u32(&source[HEADER_MODULE_FLAGS..]);
		let mut new = current;
		if limits.bounding_path {
			new &= !crate::opt::ModuleFlags::BOUND_PATH.bits();
		}
		if limits.device_id {
			new &= !crate::opt::ModuleFlags::DEVICE_ID.bits();
		}
		if new != current {
			patch.write(HEADER_MODULE_FLAGS as u64, new.to_be_bytes().to_vec());
		}
	}

	let info_size = BigEndian::read_u32(&source[sec + IMAGE_INFO_INFO_SIZE..]) as usize;
	let image_info_len = info_size.saturating_sub(RSA_SIG_LEN);
	if image_info_len == 0 {
		return Ok(patch);
	}

	let image_info_start = sec + IMAGE_INFO;
	let image_info_end = image_info_start + image_info_len;
	if image_info_end > source.len() {
		return Ok(patch);
	}

	// Build the post-edit ImageInfo in a local buffer. This doubles as the
	// bytes we hash + sign.
	let mut image_info = source[image_info_start..image_info_end].to_vec();
	let mut image_info_changed = false;

	let mut overwrite_image_info = |patch: &mut Patch,
	                                 image_info: &mut [u8],
	                                 changed: &mut bool,
	                                 abs_offset: usize,
	                                 bytes: Vec<u8>| {
		let local = abs_offset - IMAGE_INFO;
		if local + bytes.len() <= image_info.len() {
			image_info[local..local + bytes.len()].copy_from_slice(&bytes);
			patch.write((sec + abs_offset) as u64, bytes);
			*changed = true;
		}
	};

	if limits.region {
		overwrite_image_info(
			&mut patch,
			&mut image_info,
			&mut image_info_changed,
			IMAGE_INFO_GAME_REGIONS,
			0xFFFFFFFFu32.to_be_bytes().to_vec(),
		);
	}
	if limits.media {
		overwrite_image_info(
			&mut patch,
			&mut image_info,
			&mut image_info_changed,
			IMAGE_INFO_ALLOWED_MEDIA,
			0xFFFFFFFFu32.to_be_bytes().to_vec(),
		);
	}
	if limits.zero_media_id {
		overwrite_image_info(
			&mut patch,
			&mut image_info,
			&mut image_info_changed,
			IMAGE_INFO_MEDIA_ID,
			vec![0u8; 16],
		);
	}

	if limits.keyvault_privileges || limits.signed_keyvault_only {
		let local = IMAGE_INFO_IMAGE_FLAGS - IMAGE_INFO;
		if local + 4 <= image_info.len() {
			let current = BigEndian::read_u32(&image_info[local..]);
			let mut new = current;
			if limits.keyvault_privileges {
				new &= !crate::opt::ImageFlags::KV_PRIVILEGES_REQUIRED.bits();
			}
			if limits.signed_keyvault_only {
				new &= !crate::opt::ImageFlags::SIGNED_KEYVAULT_REQUIRED.bits();
			}
			if new != current {
				overwrite_image_info(
					&mut patch,
					&mut image_info,
					&mut image_info_changed,
					IMAGE_INFO_IMAGE_FLAGS,
					new.to_be_bytes().to_vec(),
				);
			}
		}
	}

	// Apply blob edits + hash recomputations.
	if !blob_edits.is_empty() {
		// Build a working copy of source with the blob edits applied so we can
		// rehash over the modified bytes.
		let mut working = source.to_vec();
		for (off, bytes) in &blob_edits {
			working[*off..*off + bytes.len()].copy_from_slice(bytes);
			patch.write(*off as u64, bytes.clone());
		}

		// If the import table was edited, rebuild its digest chain and
		// compute the new import_table_hash, write both to the working copy
		// and stage an image_info update.
		if limits.library_versions {
			if let Some((off, len)) = xex.header.optional_header_source_range(source, OptionalHeaderKey::ImportLibraries) {
				let blob = &mut working[off..off + len];
				if let Some(new_table_hash) = hashes::rewrite_import_table_hashes(blob) {
					// Overwrite the blob Write op with the digest-chain-fixed version.
					let updated = blob.to_vec();
					for edit in &mut blob_edits {
						if edit.0 == off {
							edit.1 = updated.clone();
						}
					}
					patch.write(off as u64, updated);
					overwrite_image_info(
						&mut patch,
						&mut image_info,
						&mut image_info_changed,
						IMAGE_INFO_IMPORT_TABLE_HASH,
						new_table_hash.to_vec(),
					);
				}
			}
		}

		// Recompute header_hash over the modified source and stage image_info update.
		let new_header_hash = hashes::compute_header_hash(&working, &xex.header, &xex.security_info);
		overwrite_image_info(
			&mut patch,
			&mut image_info,
			&mut image_info_changed,
			IMAGE_INFO_HEADER_HASH,
			new_header_hash.to_vec(),
		);
	}

	if !image_info_changed {
		return Ok(patch);
	}

	let digest = xecrypt::symmetric::xe_crypt_rot_sum_sha(&image_info, &[]);
	let sig = xecrypt::RsaKeyKind::Pirs
		.sign(xecrypt::ConsoleKind::Devkit, &digest)
		.map_err(|_| Xex2Error::SigningFailed.into_report())?;

	patch.write((sec + RSA_SIG) as u64, sig.to_vec());

	Ok(patch)
}
