//! Modify a XEX file: remove restrictions, convert format, re-sign.
//!
//! All modifications happen on a copy of the raw XEX bytes; the original
//! [`Xex2`] value is left unchanged.
//!
//! Modifying any ImageInfo field (media, region, media_id) invalidates the
//! kernel's RSA signature over the security info. After modifying, this
//! module recomputes the `XeCryptRotSumSha` hash and re-signs with the
//! devkit PIRS private key. Retail consoles will still reject the modified
//! XEX (since we don't have the retail PIRS private key), but devkit
//! consoles and JTAG/RGH-modded retail consoles will accept it.
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
//! // Remove all region and media restrictions
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

#[derive(Debug, Default)]
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
//
// The kernel hashes ImageInfo with XeCryptRotSumSha and verifies the RSA-PKCS#1
// signature over that hash via XeKeysVerifyPIRSSignature. After modifying any
// ImageInfo field, the hash and signature must be recomputed.

const RSA_SIG: usize = 0x008;
const RSA_SIG_LEN: usize = 0x100;
const IMAGE_INFO: usize = 0x108;
const IMAGE_INFO_INFO_SIZE: usize = IMAGE_INFO;
const IMAGE_INFO_MEDIA_ID: usize = IMAGE_INFO + 0x38;
const IMAGE_INFO_GAME_REGIONS: usize = IMAGE_INFO + 0x6C;
const IMAGE_INFO_ALLOWED_MEDIA: usize = IMAGE_INFO + 0x70;

pub fn modify_xex(
	xex: &Xex2,
	input: &[u8],
	_encryption: TargetEncryption,
	_compression: TargetCompression,
	_machine: TargetMachine,
	limits: &RemoveLimits,
) -> Result<Vec<u8>> {
	let mut data = input.to_vec();
	let sec = xex.header.security_offset as usize;

	let mut modified = false;

	if limits.region {
		let off = sec + IMAGE_INFO_GAME_REGIONS;
		if off + 4 <= data.len() {
			data[off..off + 4].copy_from_slice(&0xFFFFFFFFu32.to_be_bytes());
			modified = true;
		}
	}

	if limits.media {
		let off = sec + IMAGE_INFO_ALLOWED_MEDIA;
		if off + 4 <= data.len() {
			data[off..off + 4].copy_from_slice(&0xFFFFFFFFu32.to_be_bytes());
			modified = true;
		}
	}

	if limits.zero_media_id {
		let off = sec + IMAGE_INFO_MEDIA_ID;
		if off + 16 <= data.len() {
			data[off..off + 16].fill(0);
			modified = true;
		}
	}

	if modified {
		resign_security_info(&mut data, sec)?;
	}

	Ok(data)
}

fn resign_security_info(data: &mut [u8], sec: usize) -> Result<()> {
	let info_size = BigEndian::read_u32(&data[sec + IMAGE_INFO_INFO_SIZE..]) as usize;
	let image_info_len = info_size.saturating_sub(RSA_SIG_LEN);
	if image_info_len == 0 {
		return Ok(());
	}

	let image_info_start = sec + IMAGE_INFO;
	let image_info_end = image_info_start + image_info_len;
	if image_info_end > data.len() {
		return Ok(());
	}

	let digest = xecrypt::symmetric::xe_crypt_rot_sum_sha(&data[image_info_start..image_info_end], &[]);

	let sig = xecrypt::RsaKeyKind::Pirs
		.sign(xecrypt::ConsoleKind::Devkit, &digest)
		.map_err(|_| Xex2Error::SigningFailed.into_report())?;

	let sig_start = sec + RSA_SIG;
	let sig_end = sig_start + RSA_SIG_LEN;
	data[sig_start..sig_end].copy_from_slice(&sig);

	Ok(())
}
