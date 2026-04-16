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
//! // `modify` consumes xex -- reparse the output if you need further access.
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
use crate::opt::AllowedMediaTypes;
use crate::opt::ImageFlags;
use crate::opt::ModuleFlags;
use crate::patch::Patch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetEncryption {
	Encrypted,
	Decrypted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetCompression {
	Uncompressed,
	Basic,
	Normal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetMachine {
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
// SecurityInfo layout detail:
//
//   +0x108: image_info (info_size - 0x100 = 0x74 bytes, RSA-signed)
//       +0x000 (0x108): info_size
//       +0x004 (0x10C): image_flags
//       +0x008 (0x110): load_address
//       +0x00C (0x114): image_hash (20)
//       +0x020 (0x128): import_count
//       +0x024 (0x12C): import_hash (20)
//       +0x038 (0x140): media_id (16)
//       +0x048 (0x150): file_key (16)
//       +0x058 (0x160): export_table_address
//       +0x05C (0x164): header_hash (20)
//       +0x070 (0x178): game_regions
//   +0x17C: allowed_media_types        <- OUTSIDE image_info, NOT RSA-signed
//   +0x180: page_descriptor_count
//   +0x184: page_descriptors

const RSA_SIG: usize = 0x008;
const RSA_SIG_LEN: usize = 0x100;
pub(crate) const IMAGE_INFO: usize = 0x108;
pub(crate) const IMAGE_INFO_INFO_SIZE: usize = IMAGE_INFO;
pub(crate) const IMAGE_INFO_IMAGE_FLAGS: usize = IMAGE_INFO + 0x04;
pub(crate) const IMAGE_INFO_IMPORT_TABLE_HASH: usize = IMAGE_INFO + 0x24;
pub(crate) const IMAGE_INFO_MEDIA_ID: usize = IMAGE_INFO + 0x38;
pub(crate) const IMAGE_INFO_HEADER_HASH: usize = IMAGE_INFO + 0x5C;
pub(crate) const IMAGE_INFO_GAME_REGIONS: usize = IMAGE_INFO + 0x70;
// allowed_media_types sits at security_info + 0x17C, immediately after the
// RSA-signed image_info region. Writes here don't invalidate the signature.
pub(crate) const SECURITY_ALLOWED_MEDIA: usize = 0x17C;

// XEX main header: module_flags at offset 0x04.
const HEADER_MODULE_FLAGS: usize = 0x04;

/// A bundle of per-field edits and transforms to apply to a XEX.
///
/// [`RemoveLimits`] lowers into this via [`From`]; [`crate::rebuild::Rebuilder`]
/// exposes per-field setters that also populate it directly.
#[derive(Debug, Default, Clone)]
pub struct EditPlan {
	pub limits: RemoveLimits,
	pub module_flags: Option<ModuleFlags>,
	pub image_flags: Option<ImageFlags>,
	pub game_regions: Option<u32>,
	pub allowed_media: Option<AllowedMediaTypes>,
	pub media_id: Option<[u8; 16]>,
	pub file_key: Option<[u8; 16]>,
	pub load_address: Option<u32>,
	pub date_range: Option<(u64, u64)>,
	pub cleared_optional_headers: Vec<OptionalHeaderKey>,
	/// Target machine for the file key wrap. `None` = keep current. If the
	/// request matches the detected current machine, the transform is a no-op.
	pub target_machine: Option<TargetMachine>,
	/// Target encryption state. `None` = keep current. If the request matches
	/// the current `file_format_info.encryption_type`, the transform is a no-op.
	pub target_encryption: Option<TargetEncryption>,
}

impl EditPlan {
	pub fn is_empty(&self) -> bool {
		!self.limits.any_set()
			&& self.module_flags.is_none()
			&& self.image_flags.is_none()
			&& self.game_regions.is_none()
			&& self.allowed_media.is_none()
			&& self.media_id.is_none()
			&& self.file_key.is_none()
			&& self.load_address.is_none()
			&& self.date_range.is_none()
			&& self.cleared_optional_headers.is_empty()
			&& self.target_machine.is_none()
			&& self.target_encryption.is_none()
	}
}

impl From<RemoveLimits> for EditPlan {
	fn from(limits: RemoveLimits) -> Self {
		EditPlan { limits, ..Self::default() }
	}
}

impl From<&RemoveLimits> for EditPlan {
	fn from(limits: &RemoveLimits) -> Self {
		EditPlan { limits: limits.clone(), ..Self::default() }
	}
}

/// Build a [`Patch`] describing the requested edits plus re-hashing/re-signing.
///
/// Pure function -- reads `source` only. Covers every edit the writer knows
/// how to make: [`RemoveLimits`] recipes, per-field overrides via
/// [`EditPlan`], date-range, optional-header clearing. Image-info edits
/// trigger a RotSumSha re-sign with the devkit PIRS key; blob edits inside
/// the header-hash coverage region trigger a `header_hash` recomputation;
/// import-table edits also recompute `import_table_hash` and the digest
/// chain.
pub fn plan_edits(xex: &Xex2, source: &[u8], plan: &EditPlan) -> Result<Patch> {
	let limits = &plan.limits;
	let mut patch = Patch::new();
	let sec = xex.header.security_offset as usize;

	// Blob edits (optional-header data region, covered by header_hash).
	let mut blob_edits: Vec<(usize, Vec<u8>)> = Vec::new();

	let stage_blob_edit = |k: OptionalHeaderKey, bytes_for: &dyn Fn(usize) -> Vec<u8>, edits: &mut Vec<(usize, Vec<u8>)>| {
		if let Some((off, len)) = xex.header.optional_header_source_range(source, k) {
			edits.push((off, bytes_for(len)));
		}
	};

	if limits.dates {
		stage_blob_edit(
			OptionalHeaderKey::DateRange,
			&|len| {
				let mut b = vec![0u8; len];
				if len >= 16 {
					b[8..16].copy_from_slice(&u64::MAX.to_be_bytes());
				}
				b
			},
			&mut blob_edits,
		);
	}
	if let Some((not_before, not_after)) = plan.date_range {
		stage_blob_edit(
			OptionalHeaderKey::DateRange,
			&|len| {
				let mut b = vec![0u8; len];
				if len >= 16 {
					b[0..8].copy_from_slice(&not_before.to_be_bytes());
					b[8..16].copy_from_slice(&not_after.to_be_bytes());
				}
				b
			},
			&mut blob_edits,
		);
	}
	if limits.console_id {
		stage_blob_edit(OptionalHeaderKey::ConsoleSerialList, &|len| vec![0u8; len], &mut blob_edits);
	}
	for key in &plan.cleared_optional_headers {
		stage_blob_edit(*key, &|len| vec![0u8; len], &mut blob_edits);
	}
	if limits.library_versions {
		if let Some((off, len)) = xex.header.optional_header_source_range(source, OptionalHeaderKey::ImportLibraries) {
			let mut blob = source[off..off + len].to_vec();
			let offsets = hashes::library_entry_offsets(&blob)
				.ok_or_else(|| Xex2Error::SigningFailed.into_report())?;
			for (entry_off, _entry_size) in &offsets {
				blob[entry_off + 0x20..entry_off + 0x24].fill(0);
			}
			blob_edits.push((off, blob));
		}
	}

	// Module flag edits (outside the signed region).
	let mut module_flags_edit: Option<u32> = None;
	if limits.bounding_path || limits.device_id {
		let current = BigEndian::read_u32(&source[HEADER_MODULE_FLAGS..]);
		let mut new = current;
		if limits.bounding_path {
			new &= !ModuleFlags::BOUND_PATH.bits();
		}
		if limits.device_id {
			new &= !ModuleFlags::DEVICE_ID.bits();
		}
		if new != current {
			module_flags_edit = Some(new);
		}
	}
	if let Some(flags) = plan.module_flags {
		module_flags_edit = Some(flags.bits());
	}
	if let Some(new) = module_flags_edit {
		patch.write(HEADER_MODULE_FLAGS as u64, new.to_be_bytes().to_vec());
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

	// RemoveLimits recipes.
	if limits.region {
		overwrite_image_info(
			&mut patch,
			&mut image_info,
			&mut image_info_changed,
			sec,
			IMAGE_INFO_GAME_REGIONS,
			0xFFFFFFFFu32.to_be_bytes().to_vec(),
		);
	}
	// `media` edits allowed_media_types which is *outside* the signed image_info,
	// so it's a plain Write op with no re-sign.
	let mut allowed_media_value: Option<u32> = None;
	if limits.media {
		allowed_media_value = Some(0xFFFFFFFFu32);
	}
	if limits.zero_media_id {
		overwrite_image_info(
			&mut patch,
			&mut image_info,
			&mut image_info_changed,
			sec,
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
				new &= !ImageFlags::KV_PRIVILEGES_REQUIRED.bits();
			}
			if limits.signed_keyvault_only {
				new &= !ImageFlags::SIGNED_KEYVAULT_REQUIRED.bits();
			}
			if new != current {
				overwrite_image_info(
					&mut patch,
					&mut image_info,
					&mut image_info_changed,
					sec,
					IMAGE_INFO_IMAGE_FLAGS,
					new.to_be_bytes().to_vec(),
				);
			}
		}
	}

	// Per-field overrides (apply after recipes so explicit user values win).
	if let Some(v) = plan.game_regions {
		overwrite_image_info(&mut patch, &mut image_info, &mut image_info_changed, sec, IMAGE_INFO_GAME_REGIONS, v.to_be_bytes().to_vec());
	}
	if let Some(v) = plan.allowed_media {
		allowed_media_value = Some(v.bits());
	}
	if let Some(v) = allowed_media_value {
		patch.write((sec + SECURITY_ALLOWED_MEDIA) as u64, v.to_be_bytes().to_vec());
	}
	if let Some(id) = plan.media_id {
		overwrite_image_info(&mut patch, &mut image_info, &mut image_info_changed, sec, IMAGE_INFO_MEDIA_ID, id.to_vec());
	}
	if let Some(k) = plan.file_key {
		overwrite_image_info(&mut patch, &mut image_info, &mut image_info_changed, sec, IMAGE_INFO + 0x48, k.to_vec());
	}
	if let Some(addr) = plan.load_address {
		overwrite_image_info(&mut patch, &mut image_info, &mut image_info_changed, sec, IMAGE_INFO + 0x08, addr.to_be_bytes().to_vec());
	}
	if let Some(flags) = plan.image_flags {
		overwrite_image_info(&mut patch, &mut image_info, &mut image_info_changed, sec, IMAGE_INFO_IMAGE_FLAGS, flags.bits().to_be_bytes().to_vec());
	}

	// Encryption + machine transforms. These touch:
	// - file_key in image_info (re-wrapped or zeroed)
	// - the data region at [data_offset, data_offset + encrypted_size)
	// - the FileFormatInfo optional header (encryption_type field)
	apply_encryption_transforms(
		xex,
		source,
		plan,
		sec,
		&mut patch,
		&mut image_info,
		&mut image_info_changed,
		&mut blob_edits,
	)?;

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
						sec,
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
			sec,
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

fn overwrite_image_info(
	patch: &mut Patch,
	image_info: &mut [u8],
	changed: &mut bool,
	sec_base: usize,
	abs_offset: usize,
	bytes: Vec<u8>,
) {
	let local = abs_offset - IMAGE_INFO;
	if local + bytes.len() <= image_info.len() {
		image_info[local..local + bytes.len()].copy_from_slice(&bytes);
		patch.write((sec_base + abs_offset) as u64, bytes);
		*changed = true;
	}
}

fn apply_encryption_transforms(
	xex: &Xex2,
	source: &[u8],
	plan: &EditPlan,
	sec: usize,
	patch: &mut Patch,
	image_info: &mut [u8],
	image_info_changed: &mut bool,
	blob_edits: &mut Vec<(usize, Vec<u8>)>,
) -> Result<()> {
	if plan.target_machine.is_none() && plan.target_encryption.is_none() {
		return Ok(());
	}

	let file_format = xex.header.file_format_info()?;

	// Determine current session key + which master key currently wraps it.
	let current_file_key = &xex.security_info.image_info.file_key;
	let keys = crate::crypto::decrypt_file_key(current_file_key);
	let current_machine = detect_current_machine(source, xex, &file_format, &keys);
	let session_key = match current_machine {
		TargetMachine::Retail => keys.retail.clone(),
		TargetMachine::Devkit => keys.devkit.clone(),
	};

	let resolved_machine = plan.target_machine.unwrap_or(current_machine);

	// Re-wrap file_key under the resolved target machine's master key, but
	// only if it actually changes.
	if let Some(target_machine) = plan.target_machine {
		if target_machine != current_machine {
			let master = match resolved_machine {
				TargetMachine::Retail => crate::crypto::RETAIL_KEY.clone(),
				TargetMachine::Devkit => crate::crypto::DEVKIT_KEY.clone(),
			};
			let new_file_key = crate::crypto::wrap_file_key(&session_key, &master);
			overwrite_image_info(
				patch,
				image_info,
				image_info_changed,
				sec,
				IMAGE_INFO + 0x48,
				new_file_key.0.to_vec(),
			);
		}
	}

	if let Some(target_enc) = plan.target_encryption {
		let data_off = xex.header.data_offset as usize;
		let data_len = source.len() - data_off;
		let encrypted_region = &source[data_off..data_off + data_len];

		match (file_format.encryption_type, target_enc) {
			(crate::header::EncryptionType::Normal, TargetEncryption::Decrypted) => {
				let plaintext = crate::crypto::decrypt_data(encrypted_region, &session_key);
				patch.write(data_off as u64, plaintext);
				rewrite_file_format_encryption(xex, source, crate::header::EncryptionType::None, patch, blob_edits)?;
				// Zero file_key since it's unused when encryption is None.
				overwrite_image_info(
					patch,
					image_info,
					image_info_changed,
					sec,
					IMAGE_INFO + 0x48,
					vec![0u8; 16],
				);
			}
			(crate::header::EncryptionType::None, TargetEncryption::Encrypted) => {
				let master = match resolved_machine {
					TargetMachine::Retail => crate::crypto::RETAIL_KEY.clone(),
					TargetMachine::Devkit => crate::crypto::DEVKIT_KEY.clone(),
				};
				let ciphertext = crate::crypto::encrypt_data(encrypted_region, &session_key);
				patch.write(data_off as u64, ciphertext);
				let wrapped = crate::crypto::wrap_file_key(&session_key, &master);
				overwrite_image_info(
					patch,
					image_info,
					image_info_changed,
					sec,
					IMAGE_INFO + 0x48,
					wrapped.0.to_vec(),
				);
				rewrite_file_format_encryption(xex, source, crate::header::EncryptionType::Normal, patch, blob_edits)?;
			}
			// Already in target state; no-op.
			(crate::header::EncryptionType::Normal, TargetEncryption::Encrypted)
			| (crate::header::EncryptionType::None, TargetEncryption::Decrypted) => {}
		}
	}

	Ok(())
}

fn detect_current_machine(
	source: &[u8],
	xex: &Xex2,
	file_format: &crate::header::FileFormatInfo,
	keys: &crate::crypto::DecryptedKeys,
) -> TargetMachine {
	use crate::header::{CompressionType, EncryptionType};
	if file_format.encryption_type == EncryptionType::None {
		// When unencrypted, neither master key is used; treat as retail by convention.
		return TargetMachine::Retail;
	}
	let data_off = xex.header.data_offset as usize;
	if data_off + 32 > source.len() {
		return TargetMachine::Retail;
	}
	let probe = &source[data_off..data_off + 32];
	let retail_decrypt = crate::crypto::decrypt_data(&probe[..16], &keys.retail);
	let devkit_decrypt = crate::crypto::decrypt_data(&probe[..16], &keys.devkit);
	let _ = devkit_decrypt;
	match file_format.compression_type {
		CompressionType::Basic | CompressionType::None => {
			if retail_decrypt[0] == b'M' && retail_decrypt[1] == b'Z' {
				TargetMachine::Retail
			} else {
				TargetMachine::Devkit
			}
		}
		// For normal compression, both keys will produce ~similar-looking
		// first blocks; heuristic is fuzzier. Default to retail.
		_ => TargetMachine::Retail,
	}
}

fn rewrite_file_format_encryption(
	xex: &Xex2,
	source: &[u8],
	target: crate::header::EncryptionType,
	_patch: &mut Patch,
	blob_edits: &mut Vec<(usize, Vec<u8>)>,
) -> Result<()> {
	// FileFormatInfo blob layout:
	//   u32: info_size
	//   u16: encryption_type
	//   u16: compression_type
	//   ... (compression-type-specific trailing bytes)
	let (off, len) = xex
		.header
		.optional_header_source_range(source, OptionalHeaderKey::FileFormatInfo)
		.ok_or_else(|| Xex2Error::MissingOptionalHeader(OptionalHeaderKey::FileFormatInfo as u32).into_report())?;
	let mut blob = source[off..off + len].to_vec();
	if blob.len() < 8 {
		return Err(Xex2Error::InvalidOptionalHeaderSize { key: OptionalHeaderKey::FileFormatInfo as u32, size: blob.len() }.into_report());
	}
	let enc = target as u16;
	blob[4..6].copy_from_slice(&enc.to_be_bytes());
	// Replace or add to blob_edits; the main blob_edits loop emits the patch write.
	let mut replaced = false;
	for edit in blob_edits.iter_mut() {
		if edit.0 == off {
			edit.1 = blob.clone();
			replaced = true;
		}
	}
	if !replaced {
		blob_edits.push((off, blob));
	}
	Ok(())
}
