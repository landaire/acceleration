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
//   +0x108: info_size          (u32)
//   +0x10C: image_flags        (u32)
//   +0x110: load_address       (u32)
//   +0x114: image_hash         ([u8; 20])
//   +0x128: import_count       (u32)
//   +0x12C: import_hash        ([u8; 20])
//   +0x140: media_id           ([u8; 16])
//   +0x150: file_key           ([u8; 16])
//   +0x160: header_hash        ([u8; 20])
//   +0x174: game_regions       (u32)
//   +0x178: allowed_media      (u32)

const RSA_SIG: usize = 0x008;
const RSA_SIG_LEN: usize = 0x100;
const IMAGE_INFO: usize = 0x108;
const IMAGE_INFO_INFO_SIZE: usize = IMAGE_INFO;
const IMAGE_INFO_MEDIA_ID: usize = IMAGE_INFO + 0x38;
const IMAGE_INFO_GAME_REGIONS: usize = IMAGE_INFO + 0x6C;
const IMAGE_INFO_ALLOWED_MEDIA: usize = IMAGE_INFO + 0x70;

/// Build a [`Patch`] describing the ImageInfo edits + a re-signature.
///
/// Pure function -- takes the source bytes read-only, computes the target
/// ImageInfo image, signs it, emits Write ops for each changed field plus
/// the RSA signature.
pub fn plan_edits(xex: &Xex2, source: &[u8], limits: &RemoveLimits) -> Result<Patch> {
	let mut patch = Patch::new();
	let sec = xex.header.security_offset as usize;

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

	// Build the post-edit ImageInfo by copying the source region and applying
	// each field change. This doubles as the bytes we hash + sign.
	let mut image_info = source[image_info_start..image_info_end].to_vec();
	let mut changed = false;

	if limits.region {
		let local = IMAGE_INFO_GAME_REGIONS - IMAGE_INFO;
		if local + 4 <= image_info.len() {
			image_info[local..local + 4].copy_from_slice(&0xFFFFFFFFu32.to_be_bytes());
			patch.write(
				(sec + IMAGE_INFO_GAME_REGIONS) as u64,
				0xFFFFFFFFu32.to_be_bytes().to_vec(),
			);
			changed = true;
		}
	}

	if limits.media {
		let local = IMAGE_INFO_ALLOWED_MEDIA - IMAGE_INFO;
		if local + 4 <= image_info.len() {
			image_info[local..local + 4].copy_from_slice(&0xFFFFFFFFu32.to_be_bytes());
			patch.write(
				(sec + IMAGE_INFO_ALLOWED_MEDIA) as u64,
				0xFFFFFFFFu32.to_be_bytes().to_vec(),
			);
			changed = true;
		}
	}

	if limits.zero_media_id {
		let local = IMAGE_INFO_MEDIA_ID - IMAGE_INFO;
		if local + 16 <= image_info.len() {
			image_info[local..local + 16].fill(0);
			patch.write((sec + IMAGE_INFO_MEDIA_ID) as u64, vec![0u8; 16]);
			changed = true;
		}
	}

	if !changed {
		return Ok(patch);
	}

	let digest = xecrypt::symmetric::xe_crypt_rot_sum_sha(&image_info, &[]);
	let sig = xecrypt::RsaKeyKind::Pirs
		.sign(xecrypt::ConsoleKind::Devkit, &digest)
		.map_err(|_| Xex2Error::SigningFailed.into_report())?;

	patch.write((sec + RSA_SIG) as u64, sig.to_vec());

	Ok(patch)
}
