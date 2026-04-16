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
use crate::header::EncryptionType;
use crate::header::FileSpan;
use crate::header::OptionalHeaderKey;
use crate::opt::AllowedMediaTypes;
use crate::opt::ImageFlags;
use crate::opt::ModuleFlags;
use crate::patch::Patch;
use xenon_types::FileOffset;

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

/// Offset of a field within `image_info`.
///
/// `image_info` starts at `security_info + 0x108`. A field at
/// `ImageInfoOffset(0x70)` thus lives at absolute file offset
/// `security_offset + 0x108 + 0x70`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImageInfoOffset(pub u32);

impl ImageInfoOffset {
	pub const IMAGE_FLAGS: Self = Self(0x04);
	pub const LOAD_ADDRESS: Self = Self(0x08);
	pub const IMPORT_TABLE_HASH: Self = Self(0x24);
	pub const MEDIA_ID: Self = Self(0x38);
	pub const FILE_KEY: Self = Self(0x48);
	pub const HEADER_HASH: Self = Self(0x5C);
	pub const GAME_REGIONS: Self = Self(0x70);
}

/// Offset of a field within `security_info` (from its start, not from the
/// file start).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SecurityOffset(pub u32);

impl SecurityOffset {
	pub const RSA_SIG: Self = Self(0x008);
	pub const IMAGE_INFO: Self = Self(0x108);
	/// allowed_media_types sits immediately after the RSA-signed image_info.
	pub const ALLOWED_MEDIA: Self = Self(0x17C);
}

const RSA_SIG_LEN: usize = 0x100;
const IMAGE_INFO_OFFSET: usize = SecurityOffset::IMAGE_INFO.0 as usize;

/// XEX main header: module_flags at file offset 0x04.
const HEADER_MODULE_FLAGS: xenon_types::FileOffset = xenon_types::FileOffset(0x04);

/// A single blob-edit: replace `len` bytes at `offset` in the source file
/// with `bytes`. Used for optional-header data blobs like DateRange,
/// ConsoleSerialList, ImportLibraries.
#[derive(Debug, Clone)]
struct BlobEdit {
	offset: xenon_types::FileOffset,
	bytes: Vec<u8>,
}

/// A FILETIME-based validity window for a XEX's DateRange optional header.
/// Both ends are 100-ns intervals since 1601-01-01 (Windows FILETIME).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRangeEdit {
	pub not_before: u64,
	pub not_after: u64,
}

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
	pub file_key: Option<xenon_types::AesKey>,
	pub load_address: Option<xenon_types::VirtualAddress>,
	pub date_range: Option<DateRangeEdit>,
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
	let mut patch = Patch::new();
	let sec: FileOffset = xex.header.security_offset.into();

	let mut blob_edits = stage_blob_edits(xex, source, plan)?;
	stage_module_flags(source, plan, &mut patch);
	stage_allowed_media(source, plan, sec, &mut patch);

	// image_info is the RSA-signed portion of SecurityInfo. Some edits live
	// here; the rest of `plan_edits` threads an editor through so we can
	// recompute and sign at the end.
	let Some(image_info_span) = image_info_span(source, sec) else {
		return Ok(patch);
	};
	let mut image_info_buf = source[image_info_span.as_range()].to_vec();
	let mut editor = ImageInfoEditor::new(&mut image_info_buf, sec);

	apply_limits(plan, &mut patch, &mut editor);
	apply_field_overrides(plan, &mut patch, &mut editor);
	apply_encryption_transforms(xex, source, plan, &mut patch, &mut editor, &mut blob_edits)?;
	apply_blob_edits_and_rehash(xex, source, plan, &mut patch, &mut editor, &mut blob_edits);

	if editor.changed {
		sign_and_stage(sec, editor.buffer, &mut patch)?;
	}

	Ok(patch)
}

fn stage_blob_edits(xex: &Xex2, source: &[u8], plan: &EditPlan) -> Result<Vec<BlobEdit>> {
	let mut edits = Vec::new();
	let stage = |edits: &mut Vec<BlobEdit>, key: OptionalHeaderKey, bytes_for: &dyn Fn(usize) -> Vec<u8>| {
		if let Some(span) = xex.header.optional_header_source_range(source, key) {
			edits.push(BlobEdit { offset: span.offset, bytes: bytes_for(span.len) });
		}
	};

	if plan.limits.dates {
		stage(&mut edits, OptionalHeaderKey::DateRange, &|len| {
			let mut b = vec![0u8; len];
			if len >= 16 {
				b[8..16].copy_from_slice(&u64::MAX.to_be_bytes());
			}
			b
		});
	}
	if let Some(range) = plan.date_range {
		stage(&mut edits, OptionalHeaderKey::DateRange, &|len| {
			let mut b = vec![0u8; len];
			if len >= 16 {
				b[0..8].copy_from_slice(&range.not_before.to_be_bytes());
				b[8..16].copy_from_slice(&range.not_after.to_be_bytes());
			}
			b
		});
	}
	if plan.limits.console_id {
		stage(&mut edits, OptionalHeaderKey::ConsoleSerialList, &|len| vec![0u8; len]);
	}
	for key in &plan.cleared_optional_headers {
		stage(&mut edits, *key, &|len| vec![0u8; len]);
	}
	if plan.limits.library_versions
		&& let Some(span) = xex.header.optional_header_source_range(source, OptionalHeaderKey::ImportLibraries)
	{
		let mut blob = source[span.as_range()].to_vec();
		let entries = hashes::library_entry_offsets(&blob).ok_or_else(|| Xex2Error::SigningFailed.into_report())?;
		for entry in &entries {
			let version_min = entry.offset + 0x20;
			blob[version_min..version_min + 4].fill(0);
		}
		edits.push(BlobEdit { offset: span.offset, bytes: blob });
	}

	Ok(edits)
}

fn stage_module_flags(source: &[u8], plan: &EditPlan, patch: &mut Patch) {
	let new_flags = resolve_module_flags(source, plan);
	if let Some(flags) = new_flags {
		patch.write(HEADER_MODULE_FLAGS.get(), flags.bits().to_be_bytes().to_vec());
	}
}

fn resolve_module_flags(source: &[u8], plan: &EditPlan) -> Option<ModuleFlags> {
	// Explicit override wins.
	if let Some(flags) = plan.module_flags {
		return Some(flags);
	}
	if !plan.limits.bounding_path && !plan.limits.device_id {
		return None;
	}
	let current = ModuleFlags::from_bits_retain(BigEndian::read_u32(&source[HEADER_MODULE_FLAGS.as_usize()..]));
	let mut clear = ModuleFlags::empty();
	if plan.limits.bounding_path {
		clear |= ModuleFlags::BOUND_PATH;
	}
	if plan.limits.device_id {
		clear |= ModuleFlags::DEVICE_ID;
	}
	let new = current - clear;
	(new != current).then_some(new)
}

fn stage_allowed_media(source: &[u8], plan: &EditPlan, sec: FileOffset, patch: &mut Patch) {
	let _ = source;
	let value = match (plan.limits.media, plan.allowed_media) {
		(_, Some(media)) => Some(media.bits()),
		(true, None) => Some(0xFFFFFFFFu32),
		_ => None,
	};
	if let Some(v) = value {
		let file_off = sec + SecurityOffset::ALLOWED_MEDIA.0 as usize;
		patch.write(file_off.get(), v.to_be_bytes().to_vec());
	}
}

fn image_info_span(source: &[u8], sec: FileOffset) -> Option<FileSpan> {
	let info_size_off = sec + IMAGE_INFO_OFFSET;
	let info_size = BigEndian::read_u32(&source[info_size_off.as_usize()..]) as usize;
	let image_info_len = info_size.checked_sub(RSA_SIG_LEN)?;
	if image_info_len == 0 {
		return None;
	}
	let start = sec + IMAGE_INFO_OFFSET;
	(start.as_usize() + image_info_len <= source.len()).then_some(FileSpan { offset: start, len: image_info_len })
}

fn apply_limits(plan: &EditPlan, patch: &mut Patch, editor: &mut ImageInfoEditor<'_>) {
	if plan.limits.region {
		editor.overwrite(patch, ImageInfoOffset::GAME_REGIONS, 0xFFFFFFFFu32.to_be_bytes().to_vec());
	}
	if plan.limits.zero_media_id {
		editor.overwrite(patch, ImageInfoOffset::MEDIA_ID, vec![0u8; 16]);
	}
	if plan.limits.keyvault_privileges || plan.limits.signed_keyvault_only {
		editor.update_u32(patch, ImageInfoOffset::IMAGE_FLAGS, |current| {
			let flags = ImageFlags::from_bits_retain(current);
			let mut clear = ImageFlags::empty();
			if plan.limits.keyvault_privileges {
				clear |= ImageFlags::KV_PRIVILEGES_REQUIRED;
			}
			if plan.limits.signed_keyvault_only {
				clear |= ImageFlags::SIGNED_KEYVAULT_REQUIRED;
			}
			(flags - clear).bits()
		});
	}
}

fn apply_field_overrides(plan: &EditPlan, patch: &mut Patch, editor: &mut ImageInfoEditor<'_>) {
	if let Some(v) = plan.game_regions {
		editor.overwrite(patch, ImageInfoOffset::GAME_REGIONS, v.to_be_bytes().to_vec());
	}
	if let Some(id) = plan.media_id {
		editor.overwrite(patch, ImageInfoOffset::MEDIA_ID, id.to_vec());
	}
	if let Some(k) = plan.file_key {
		editor.overwrite(patch, ImageInfoOffset::FILE_KEY, k.0.to_vec());
	}
	if let Some(addr) = plan.load_address {
		editor.overwrite(patch, ImageInfoOffset::LOAD_ADDRESS, addr.0.to_be_bytes().to_vec());
	}
	if let Some(flags) = plan.image_flags {
		editor.overwrite(patch, ImageInfoOffset::IMAGE_FLAGS, flags.bits().to_be_bytes().to_vec());
	}
}

fn apply_blob_edits_and_rehash(
	xex: &Xex2,
	source: &[u8],
	plan: &EditPlan,
	patch: &mut Patch,
	editor: &mut ImageInfoEditor<'_>,
	blob_edits: &mut [BlobEdit],
) {
	if blob_edits.is_empty() {
		return;
	}

	// Apply blob edits to a working copy so hash recomputations see the new bytes.
	let mut working = source.to_vec();
	for edit in blob_edits.iter() {
		let start = edit.offset.as_usize();
		working[start..start + edit.bytes.len()].copy_from_slice(&edit.bytes);
		patch.write(edit.offset.get(), edit.bytes.clone());
	}

	// If the import table changed, rebuild its digest chain and stage the
	// new import_table_hash alongside the updated blob.
	if plan.limits.library_versions
		&& let Some(span) = xex.header.optional_header_source_range(source, OptionalHeaderKey::ImportLibraries)
	{
		let blob = &mut working[span.as_range()];
		if let Some(new_table_hash) = hashes::rewrite_import_table_hashes(blob) {
			let updated = blob.to_vec();
			if let Some(existing) = blob_edits.iter_mut().find(|e| e.offset == span.offset) {
				existing.bytes = updated.clone();
			}
			patch.write(span.offset.get(), updated);
			editor.overwrite(patch, ImageInfoOffset::IMPORT_TABLE_HASH, new_table_hash.to_vec());
		}
	}

	let new_header_hash = hashes::compute_header_hash(&working, &xex.header, &xex.security_info);
	editor.overwrite(patch, ImageInfoOffset::HEADER_HASH, new_header_hash.to_vec());
}

fn sign_and_stage(sec: FileOffset, image_info: &[u8], patch: &mut Patch) -> Result<()> {
	let digest = xecrypt::symmetric::xe_crypt_rot_sum_sha(image_info, &[]);
	let sig = xecrypt::RsaKeyKind::Pirs
		.sign(xecrypt::ConsoleKind::Devkit, &digest)
		.map_err(|_| Xex2Error::SigningFailed.into_report())?;
	let sig_off = sec + SecurityOffset::RSA_SIG.0 as usize;
	patch.write(sig_off.get(), sig.to_vec());
	Ok(())
}

/// Mutable state for staging image_info edits across the various helpers.
struct ImageInfoEditor<'a> {
	/// Working copy of the image_info region. Reused to compute the
	/// post-edit RotSumSha digest that gets signed.
	buffer: &'a mut [u8],
	/// Whether any overwrite was staged (drives the re-sign decision).
	changed: bool,
	/// Absolute offset of security_info in the source file.
	sec: xenon_types::FileOffset,
}

impl<'a> ImageInfoEditor<'a> {
	fn new(buffer: &'a mut [u8], sec: FileOffset) -> Self {
		Self { buffer, changed: false, sec }
	}

	/// Overwrite `bytes.len()` bytes at `field` within the image_info region,
	/// staging both the local buffer update and a Write op on `patch`.
	fn overwrite(&mut self, patch: &mut Patch, field: ImageInfoOffset, bytes: Vec<u8>) {
		let local = field.0 as usize;
		if local + bytes.len() > self.buffer.len() {
			return;
		}
		self.buffer[local..local + bytes.len()].copy_from_slice(&bytes);
		let file_off = self.sec + IMAGE_INFO_OFFSET + local;
		patch.write(file_off.get(), bytes);
		self.changed = true;
	}

	/// Read the big-endian u32 at `field`, apply `f` to produce a new value,
	/// and overwrite only if it actually changed.
	fn update_u32(&mut self, patch: &mut Patch, field: ImageInfoOffset, f: impl FnOnce(u32) -> u32) {
		let local = field.0 as usize;
		if local + 4 > self.buffer.len() {
			return;
		}
		let current = BigEndian::read_u32(&self.buffer[local..]);
		let new = f(current);
		if new != current {
			self.overwrite(patch, field, new.to_be_bytes().to_vec());
		}
	}
}

fn apply_encryption_transforms(
	xex: &Xex2,
	source: &[u8],
	plan: &EditPlan,
	patch: &mut Patch,
	editor: &mut ImageInfoEditor<'_>,
	blob_edits: &mut Vec<BlobEdit>,
) -> Result<()> {
	if plan.target_machine.is_none() && plan.target_encryption.is_none() {
		return Ok(());
	}

	let file_format = xex.header.file_format_info()?;
	let current_file_key = &xex.security_info.image_info.file_key;
	let keys = crate::crypto::decrypt_file_key(current_file_key);
	let current_machine = detect_current_machine(source, xex, &file_format, &keys);
	let session_key = match current_machine {
		TargetMachine::Retail => keys.retail,
		TargetMachine::Devkit => keys.devkit,
	};
	let resolved_machine = plan.target_machine.unwrap_or(current_machine);

	// Re-wrap under target machine's master key, but only when it would change.
	if let Some(target) = plan.target_machine
		&& target != current_machine
	{
		let master = master_key_for(resolved_machine);
		let new_file_key = crate::crypto::wrap_file_key(&session_key, &master);
		editor.overwrite(patch, ImageInfoOffset::FILE_KEY, new_file_key.0.to_vec());
	}

	if let Some(target) = plan.target_encryption {
		let data_off = FileOffset::from(xex.header.data_offset);
		let data_range = data_off.as_usize()..source.len();
		let region = &source[data_range];

		match (file_format.encryption_type, target) {
			(EncryptionType::Normal, TargetEncryption::Decrypted) => {
				let plaintext = crate::crypto::decrypt_data(region, &session_key);
				patch.write(data_off.get(), plaintext);
				stage_file_format_encryption(xex, source, EncryptionType::None, blob_edits)?;
				// file_key is ignored when encryption is None; zero it out.
				editor.overwrite(patch, ImageInfoOffset::FILE_KEY, vec![0u8; 16]);
			}
			(EncryptionType::None, TargetEncryption::Encrypted) => {
				let ciphertext = crate::crypto::encrypt_data(region, &session_key);
				patch.write(data_off.get(), ciphertext);
				let master = master_key_for(resolved_machine);
				let wrapped = crate::crypto::wrap_file_key(&session_key, &master);
				editor.overwrite(patch, ImageInfoOffset::FILE_KEY, wrapped.0.to_vec());
				stage_file_format_encryption(xex, source, EncryptionType::Normal, blob_edits)?;
			}
			// Already in target state.
			(EncryptionType::Normal, TargetEncryption::Encrypted)
			| (EncryptionType::None, TargetEncryption::Decrypted) => {}
		}
	}

	Ok(())
}

fn master_key_for(machine: TargetMachine) -> crate::header::AesKey {
	match machine {
		TargetMachine::Retail => crate::crypto::RETAIL_KEY,
		TargetMachine::Devkit => crate::crypto::DEVKIT_KEY,
	}
}

fn detect_current_machine(
	source: &[u8],
	xex: &Xex2,
	file_format: &crate::header::FileFormatInfo,
	keys: &crate::crypto::DecryptedKeys,
) -> TargetMachine {
	use crate::header::CompressionType;
	if file_format.encryption_type == EncryptionType::None {
		// Unencrypted: master key unused; pick retail by convention.
		return TargetMachine::Retail;
	}
	let data_off = xex.header.data_offset as usize;
	let Some(probe) = source.get(data_off..data_off + 16) else {
		return TargetMachine::Retail;
	};
	match file_format.compression_type {
		CompressionType::Basic | CompressionType::None => {
			let decrypted = crate::crypto::decrypt_data(probe, &keys.retail);
			if decrypted.starts_with(b"MZ") { TargetMachine::Retail } else { TargetMachine::Devkit }
		}
		// Normal compression: both keys produce similar-looking first blocks;
		// the heuristic is fuzzier, so default to retail.
		_ => TargetMachine::Retail,
	}
}

/// Stage a blob edit that flips the encryption_type field in FileFormatInfo.
fn stage_file_format_encryption(
	xex: &Xex2,
	source: &[u8],
	target: EncryptionType,
	blob_edits: &mut Vec<BlobEdit>,
) -> Result<()> {
	// FileFormatInfo blob layout:
	//   u32: info_size
	//   u16: encryption_type
	//   u16: compression_type
	//   ... (compression-type-specific trailing bytes)
	let span = xex
		.header
		.optional_header_source_range(source, OptionalHeaderKey::FileFormatInfo)
		.ok_or_else(|| Xex2Error::MissingOptionalHeader(OptionalHeaderKey::FileFormatInfo as u32).into_report())?;
	let mut blob = source[span.as_range()].to_vec();
	if blob.len() < 8 {
		return Err(Xex2Error::InvalidOptionalHeaderSize {
			key: OptionalHeaderKey::FileFormatInfo as u32,
			size: blob.len(),
		}
		.into_report());
	}
	blob[4..6].copy_from_slice(&(target as u16).to_be_bytes());

	if let Some(existing) = blob_edits.iter_mut().find(|e| e.offset == span.offset) {
		existing.bytes = blob;
	} else {
		blob_edits.push(BlobEdit { offset: span.offset, bytes: blob });
	}
	Ok(())
}
