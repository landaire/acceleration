use crate::error::Result;
use crate::Xex2;

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

pub fn modify_xex(
	xex: &Xex2,
	_encryption: TargetEncryption,
	_compression: TargetCompression,
	_machine: TargetMachine,
	limits: &RemoveLimits,
) -> Result<Vec<u8>> {
	let mut data = xex.raw().to_vec();

	let sec_offset = xex.header.security_offset as usize;
	let image_key_offset = sec_offset + 8;

	if limits.region {
		let region_offset = image_key_offset + 0x16C;
		if region_offset + 4 <= data.len() {
			data[region_offset..region_offset + 4].copy_from_slice(&0xFFFFFFFFu32.to_be_bytes());
		}
	}

	if limits.media {
		let media_offset = image_key_offset + 0x170;
		if media_offset + 4 <= data.len() {
			data[media_offset..media_offset + 4].copy_from_slice(&0xFFFFFFFFu32.to_be_bytes());
		}
	}

	if limits.zero_media_id {
		let media_id_offset = image_key_offset + 0x138;
		if media_id_offset + 16 <= data.len() {
			data[media_id_offset..media_id_offset + 16].fill(0);
		}
	}

	Ok(data)
}
