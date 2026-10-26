//! Plan in-place modifications to an XEX file as a [`Patch`].
//!
//! This module handles length-preserving edits: ImageInfo and module flags,
//! media_id, optional-header blobs (date range, import libraries), encryption
//! and retail/devkit file-key changes, and in-place PE replacement on
//! uncompressed sources, followed by hash recomputation and devkit re-signing.
//! The edits are expressed as a [`Patch`] of
//! [`PatchOp::Write`][crate::patch::PatchOp::Write] ops.
//!
//! Changes that alter file length (compression changes) go through
//! [`crate::rebuild::Rebuilder`].
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
use crate::builder::BoundingPath;
use crate::builder::bounding_path_bytes;
use crate::error::Result;
use crate::error::Xex2Error;
use crate::hashes;
use crate::header::EncryptionType;
use crate::header::FileSpan;
use crate::header::OptionalHeaderKey;
use crate::opt::AllowedMediaTypes;
use crate::opt::ImageFlags;
use crate::opt::ModuleFlags;
use crate::opt::ResourceEntry;
use crate::opt::resource_info_bytes;
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
	pub const IMAGE_HASH: Self = Self(0x0C);
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
	pub const PAGE_DESCRIPTOR_COUNT: Self = Self(0x180);
	pub const PAGE_DESCRIPTORS: Self = Self(0x184);
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

impl DateRangeEdit {
	/// Construct from raw FILETIME values.
	pub fn from_filetimes(not_before: u64, not_after: u64) -> Self {
		Self { not_before, not_after }
	}

	/// A validity window that the kernel accepts as "always valid":
	/// `not_before = 0`, `not_after = u64::MAX`.
	pub fn unlimited() -> Self {
		Self { not_before: 0, not_after: u64::MAX }
	}

	/// Construct from `jiff::Timestamp` values. Returns `None` if either
	/// timestamp is before the FILETIME epoch (1601-01-01).
	///
	/// Available with the `jiff` feature.
	#[cfg(feature = "jiff")]
	pub fn from_timestamps(not_before: jiff::Timestamp, not_after: jiff::Timestamp) -> Option<Self> {
		Some(Self {
			not_before: xenon_types::timestamp_to_filetime(not_before)?,
			not_after: xenon_types::timestamp_to_filetime(not_after)?,
		})
	}
}

/// A bundle of per-field edits and transforms to apply to a XEX.
///
/// [`RemoveLimits`] lowers into this via [`From`]; [`crate::rebuild::Rebuilder`]
/// exposes per-field setters that also populate it directly.
#[derive(Debug, Default, Clone)]
pub(crate) struct EditPlan {
	pub limits: RemoveLimits,
	pub module_flags: Option<ModuleFlags>,
	pub image_flags: Option<ImageFlags>,
	pub game_regions: Option<u32>,
	pub allowed_media: Option<AllowedMediaTypes>,
	pub media_id: Option<[u8; 16]>,
	pub file_key: Option<xenon_types::AesKey>,
	pub load_address: Option<xenon_types::VirtualAddress>,
	pub date_range: Option<DateRangeEdit>,
	pub bounding_path: Option<BoundingPath>,
	pub added_resources: Vec<ResourceEntry>,
	pub cleared_optional_headers: Vec<OptionalHeaderKey>,
	/// Target machine for the file key wrap. `None` = keep current. If the
	/// request matches the detected current machine, the transform is a no-op.
	pub target_machine: Option<TargetMachine>,
	/// Target encryption state. `None` = keep current. If the request matches
	/// the current `file_format_info.encryption_type`, the transform is a no-op.
	pub target_encryption: Option<TargetEncryption>,
	/// Replace the inner PE image with these bytes. `plan_edits` handles only
	/// uncompressed sources (`CompressionType::None`); `Rebuilder` routes
	/// compressed sources through `assemble`. Triggers
	/// regeneration of page descriptors and `image_hash` using the kernel-
	/// verified chain formula in [`crate::page_descriptors`].
	pub replace_pe: Option<Vec<u8>>,
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
			&& self.bounding_path.is_none()
			&& self.added_resources.is_empty()
			&& self.cleared_optional_headers.is_empty()
			&& self.target_machine.is_none()
			&& self.target_encryption.is_none()
			&& self.replace_pe.is_none()
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
pub(crate) fn plan_edits(xex: &Xex2, source: &[u8], plan: &EditPlan) -> Result<Patch> {
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

	apply_limits(plan, &mut editor);
	apply_field_overrides(plan, &mut editor);
	apply_encryption_transforms(xex, source, plan, &mut patch, &mut editor, &mut blob_edits)?;
	prepare_blob_edits(xex, source, plan, &mut editor, &mut blob_edits);

	let inserts_resource_header =
		!plan.added_resources.is_empty() && xex.header.get_optional_header(OptionalHeaderKey::ResourceInfo).is_none();
	if inserts_resource_header {
		stage_added_resources(xex, source, plan, &mut blob_edits, &mut patch)?;
	} else {
		stage_bounding_path(xex, source, plan, &mut patch)?;
		stage_added_resources(xex, source, plan, &mut blob_edits, &mut patch)?;
	}

	apply_pe_replacement(xex, source, plan, &mut patch, &mut editor)?;
	stage_blob_writes(&mut patch, &blob_edits);

	let header_end = xex.header.data_offset as usize;
	if header_end <= source.len() {
		let mut header = source[..header_end].to_vec();
		patch.apply_to_prefix(&mut header)?;
		let header_hash = hashes::compute_header_hash(&header, &xex.header, &xex.security_info)?;
		editor.overwrite(ImageInfoOffset::HEADER_HASH, header_hash.to_vec());
	}

	if editor.changed {
		sign_and_stage(sec, editor.buffer, &mut patch)?;
	}

	Ok(patch)
}

fn stage_bounding_path(xex: &Xex2, source: &[u8], plan: &EditPlan, patch: &mut Patch) -> Result<()> {
	let Some(path) = plan.bounding_path.as_ref() else {
		return Ok(());
	};
	stage_relocated_optional_header(xex, source, OptionalHeaderKey::BoundingPath, bounding_path_bytes(path), patch)
}

fn stage_added_resources(
	xex: &Xex2,
	source: &[u8],
	plan: &EditPlan,
	blob_edits: &mut Vec<BlobEdit>,
	patch: &mut Patch,
) -> Result<()> {
	if plan.added_resources.is_empty() {
		return Ok(());
	}
	let has_resource_header = xex.header.get_optional_header(OptionalHeaderKey::ResourceInfo).is_some();
	let mut resources = match xex.header.resource_info() {
		Some(info) => info.resources,
		None if !has_resource_header => Vec::new(),
		None => return Err(Xex2Error::InvalidResourceInfo.into_report()),
	};
	let image_start = u64::from(plan.load_address.unwrap_or(xex.security_info.image_info.load_address).0);
	let image_end = image_start + u64::from(xex.security_info.image_size);
	for resource in &plan.added_resources {
		if resources.iter().any(|entry| entry.name == resource.name) {
			return Err(Xex2Error::DuplicateResourceName { name: resource.name }.into_report());
		}
		let start = u64::from(resource.address.0);
		if start < image_start || start + u64::from(resource.size) > image_end {
			return Err(Xex2Error::ResourceOutsideImage {
				name: resource.name,
				address: resource.address.0,
				size: resource.size,
			}
			.into_report());
		}
		resources.push(*resource);
	}
	let bytes = resource_info_bytes(&resources);
	if has_resource_header {
		stage_relocated_optional_header(xex, source, OptionalHeaderKey::ResourceInfo, bytes, patch)?;
	} else {
		stage_inserted_resource_header(xex, source, bytes, plan.bounding_path.as_ref(), blob_edits, patch)?;
	}
	Ok(())
}

fn stage_relocated_optional_header(
	xex: &Xex2,
	source: &[u8],
	key: OptionalHeaderKey,
	bytes: Vec<u8>,
	patch: &mut Patch,
) -> Result<()> {
	let old_span = xex
		.header
		.optional_header_source_range(source, key)
		.ok_or_else(|| Xex2Error::MissingOptionalHeader(key as u32).into_report())?;
	let target = if bytes.len() <= old_span.len {
		old_span.offset.get() as usize
	} else {
		find_free_header_region(xex, source, bytes.len(), patch, None)
			.ok_or_else(|| Xex2Error::NoOptionalHeaderSpace { size: bytes.len() }.into_report())?
	};

	let index_end = 0x18usize + xex.header.optional_header_count as usize * 8;
	let value_offset = (0x18usize..index_end)
		.step_by(8)
		.find_map(|offset| (BigEndian::read_u32(&source[offset..offset + 4]) == key as u32).then_some(offset + 4))
		.ok_or_else(|| Xex2Error::MissingOptionalHeader(key as u32).into_report())?;
	patch.write(value_offset as u64, (target as u32).to_be_bytes().to_vec());
	patch.write(target as u64, bytes);
	Ok(())
}

#[derive(Clone, Copy)]
struct OptionalHeaderIndexEntry {
	key: u32,
	value: u32,
}

fn stage_inserted_resource_header(
	xex: &Xex2,
	source: &[u8],
	resource_bytes: Vec<u8>,
	bounding_path: Option<&BoundingPath>,
	blob_edits: &mut Vec<BlobEdit>,
	patch: &mut Patch,
) -> Result<()> {
	let old_index_end = 0x18usize
		.checked_add(xex.header.optional_header_count as usize * 8)
		.ok_or_else(|| Xex2Error::NoOptionalHeaderSpace { size: resource_bytes.len() + 8 }.into_report())?;
	let new_index_end = old_index_end
		.checked_add(8)
		.ok_or_else(|| Xex2Error::NoOptionalHeaderSpace { size: resource_bytes.len() + 8 }.into_report())?;
	let tail_range = old_index_end..new_index_end;
	let tail = source
		.get(tail_range.clone())
		.ok_or_else(|| Xex2Error::NoOptionalHeaderSpace { size: resource_bytes.len() + 8 }.into_report())?;
	let security_start = xex.header.security_offset as usize;
	let security_end = security_start
		.checked_add(xex.security_info.header_size as usize)
		.ok_or_else(|| Xex2Error::NoOptionalHeaderSpace { size: resource_bytes.len() + 8 }.into_report())?;
	if new_index_end > xex.header.data_offset as usize
		|| ranges_overlap(&tail_range, &(security_start..security_end))
		|| patch.overlaps(tail_range.clone())
	{
		return Err(Xex2Error::NoOptionalHeaderSpace { size: resource_bytes.len() + 8 }.into_report());
	}

	let mut entries = Vec::with_capacity(xex.header.optional_header_count as usize + 1);
	for offset in (0x18usize..old_index_end).step_by(8) {
		entries.push(OptionalHeaderIndexEntry {
			key: BigEndian::read_u32(&source[offset..offset + 4]),
			value: BigEndian::read_u32(&source[offset + 4..offset + 8]),
		});
	}
	if let Some(pair) = entries.windows(2).find(|pair| pair[0].key >= pair[1].key) {
		return Err(Xex2Error::OptionalHeaderOrder { previous: pair[0].key, current: pair[1].key }.into_report());
	}

	let bounding_bytes = bounding_path.map(bounding_path_bytes);
	if bounding_bytes.is_some() && !entries.iter().any(|entry| entry.key == OptionalHeaderKey::BoundingPath as u32) {
		return Err(Xex2Error::MissingOptionalHeader(OptionalHeaderKey::BoundingPath as u32).into_report());
	}
	let blobs = optional_header_blobs(xex, source)
		.ok_or_else(|| Xex2Error::NoOptionalHeaderSpace { size: resource_bytes.len() + 8 }.into_report())?;
	let mut covered = [false; 8];
	for blob in &blobs {
		let pending = blob_edits
			.iter()
			.rposition(|edit| edit.offset.as_usize() == blob.range.start)
			.map(|index| blob_edits.remove(index).bytes);
		blob_edits.retain(|edit| edit.offset.as_usize() != blob.range.start);
		let replacement = if blob.key == OptionalHeaderKey::BoundingPath as u32 {
			bounding_bytes.as_deref().or(pending.as_deref())
		} else {
			pending.as_deref()
		};
		let bytes = replacement.unwrap_or(&source[blob.range.clone()]);
		let relocate = ranges_overlap(&tail_range, &blob.range) || bytes.len() > blob.range.len();
		if relocate {
			let target = find_free_header_region(xex, source, bytes.len(), patch, Some(&tail_range))
				.ok_or_else(|| Xex2Error::NoOptionalHeaderSpace { size: bytes.len() }.into_report())?;
			patch.write(target as u64, bytes.to_vec());
			entries
				.iter_mut()
				.find(|entry| entry.key == blob.key)
				.ok_or_else(|| Xex2Error::MissingOptionalHeader(blob.key).into_report())?
				.value = target as u32;
		} else if replacement.is_some() {
			patch.write(blob.range.start as u64, bytes.to_vec());
		}
		if ranges_overlap(&tail_range, &blob.range) {
			let start = blob.range.start.max(old_index_end);
			let end = blob.range.end.min(new_index_end);
			covered[start - old_index_end..end - old_index_end].fill(true);
		}
	}
	if tail.iter().zip(covered).any(|(byte, covered)| *byte != 0 && !covered) {
		return Err(Xex2Error::NoOptionalHeaderSpace { size: resource_bytes.len() + 8 }.into_report());
	}

	let resource_target = find_free_header_region(xex, source, resource_bytes.len(), patch, Some(&tail_range))
		.ok_or_else(|| Xex2Error::NoOptionalHeaderSpace { size: resource_bytes.len() }.into_report())?;
	let resource_key = OptionalHeaderKey::ResourceInfo as u32;
	let insert_at = entries.partition_point(|entry| entry.key < resource_key);
	if entries.get(insert_at).is_some_and(|entry| entry.key == resource_key) {
		return Err(Xex2Error::InvalidResourceInfo.into_report());
	}
	entries.insert(insert_at, OptionalHeaderIndexEntry { key: resource_key, value: resource_target as u32 });
	let mut index_bytes = Vec::with_capacity(entries.len() * 8);
	for entry in entries {
		index_bytes.extend_from_slice(&entry.key.to_be_bytes());
		index_bytes.extend_from_slice(&entry.value.to_be_bytes());
	}
	patch.write(0x18, index_bytes);
	patch.write(0x14, (xex.header.optional_header_count + 1).to_be_bytes().to_vec());
	patch.write(resource_target as u64, resource_bytes);
	Ok(())
}

fn ranges_overlap(first: &std::ops::Range<usize>, second: &std::ops::Range<usize>) -> bool {
	first.start < second.end && second.start < first.end
}

struct HeaderBlob {
	key: u32,
	range: std::ops::Range<usize>,
}

fn optional_header_blobs(xex: &Xex2, source: &[u8]) -> Option<Vec<HeaderBlob>> {
	let header_end = xex.header.data_offset as usize;
	let index_end = 0x18usize.checked_add(xex.header.optional_header_count as usize * 8)?;
	if index_end > header_end || header_end > source.len() {
		return None;
	}
	let mut blobs = Vec::with_capacity(xex.header.optional_header_count as usize);
	for offset in (0x18usize..index_end).step_by(8) {
		let key = BigEndian::read_u32(&source[offset..offset + 4]);
		let pointer = BigEndian::read_u32(&source[offset + 4..offset + 8]) as usize;
		let words = (key & 0xff) as usize;
		let data_len = match words {
			0 | 1 => continue,
			0xff => {
				if pointer.checked_add(4)? > header_end {
					return None;
				}
				BigEndian::read_u32(&source[pointer..pointer + 4]) as usize
			}
			_ => words.checked_mul(4)?,
		};
		let end = pointer.checked_add(data_len)?;
		if end > header_end {
			return None;
		}
		blobs.push(HeaderBlob { key, range: pointer..end });
	}
	Some(blobs)
}

fn occupied_header_regions(xex: &Xex2, source: &[u8]) -> Option<Vec<std::ops::Range<usize>>> {
	let header_end = xex.header.data_offset as usize;
	let index_end = 0x18usize.checked_add(xex.header.optional_header_count as usize * 8)?;
	if index_end > header_end || header_end > source.len() {
		return None;
	}

	let blobs = optional_header_blobs(xex, source)?;
	let mut occupied = Vec::with_capacity(blobs.len() + 2);
	occupied.push(0..index_end);
	let security_start = xex.header.security_offset as usize;
	let security_end = security_start.checked_add(xex.security_info.header_size as usize)?;
	if security_end > header_end {
		return None;
	}
	occupied.push(security_start..security_end);
	occupied.extend(blobs.into_iter().map(|blob| blob.range));
	Some(occupied)
}

fn find_free_header_region(
	xex: &Xex2,
	source: &[u8],
	len: usize,
	patch: &Patch,
	reserved: Option<&std::ops::Range<usize>>,
) -> Option<usize> {
	let header_end = xex.header.data_offset as usize;
	if len > header_end {
		return None;
	}
	let occupied = occupied_header_regions(xex, source)?;
	let index_end = occupied.first()?.end;
	for candidate in (index_end..=header_end - len).step_by(4) {
		let range = candidate..candidate + len;
		let overlaps_reserved = reserved.is_some_and(|reserved| ranges_overlap(&range, reserved));
		if overlaps_reserved
			|| occupied.iter().any(|occupied| ranges_overlap(&range, occupied))
			|| patch.overlaps(range.clone())
		{
			continue;
		}
		if source[range].iter().all(|byte| *byte == 0) {
			return Some(candidate);
		}
	}
	None
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
	if !plan.limits.bounding_path && !plan.limits.device_id && plan.bounding_path.is_none() {
		return None;
	}
	let current = ModuleFlags::from_bits_retain(BigEndian::read_u32(&source[HEADER_MODULE_FLAGS.as_usize()..]));
	let mut new = current;
	if plan.limits.bounding_path {
		new.remove(ModuleFlags::BOUND_PATH);
	}
	if plan.limits.device_id {
		new.remove(ModuleFlags::DEVICE_ID);
	}
	if plan.bounding_path.is_some() {
		new.insert(ModuleFlags::BOUND_PATH);
	}
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

fn apply_limits(plan: &EditPlan, editor: &mut ImageInfoEditor<'_>) {
	if plan.limits.region {
		editor.overwrite(ImageInfoOffset::GAME_REGIONS, 0xFFFFFFFFu32.to_be_bytes().to_vec());
	}
	if plan.limits.zero_media_id {
		editor.overwrite(ImageInfoOffset::MEDIA_ID, vec![0u8; 16]);
	}
	if plan.limits.keyvault_privileges || plan.limits.signed_keyvault_only {
		editor.update_u32(ImageInfoOffset::IMAGE_FLAGS, |current| {
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

fn apply_field_overrides(plan: &EditPlan, editor: &mut ImageInfoEditor<'_>) {
	if let Some(v) = plan.game_regions {
		editor.overwrite(ImageInfoOffset::GAME_REGIONS, v.to_be_bytes().to_vec());
	}
	if let Some(id) = plan.media_id {
		editor.overwrite(ImageInfoOffset::MEDIA_ID, id.to_vec());
	}
	if let Some(k) = plan.file_key {
		editor.overwrite(ImageInfoOffset::FILE_KEY, k.to_vec());
	}
	if let Some(addr) = plan.load_address {
		editor.overwrite(ImageInfoOffset::LOAD_ADDRESS, addr.0.to_be_bytes().to_vec());
	}
	if let Some(flags) = plan.image_flags {
		editor.overwrite(ImageInfoOffset::IMAGE_FLAGS, flags.bits().to_be_bytes().to_vec());
	}
}

fn prepare_blob_edits(
	xex: &Xex2,
	source: &[u8],
	plan: &EditPlan,
	editor: &mut ImageInfoEditor<'_>,
	blob_edits: &mut [BlobEdit],
) {
	if blob_edits.is_empty() {
		return;
	}

	// The import digest chain covers the final library records, including
	// their cleared minimum versions.
	if plan.limits.library_versions
		&& let Some(span) = xex.header.optional_header_source_range(source, OptionalHeaderKey::ImportLibraries)
		&& let Some(edit) = blob_edits.iter_mut().rev().find(|edit| edit.offset == span.offset)
		&& let Some(table_hash) = hashes::rewrite_import_table_hashes(&mut edit.bytes)
	{
		editor.overwrite(ImageInfoOffset::IMPORT_TABLE_HASH, table_hash.to_vec());
	}
}

fn stage_blob_writes(patch: &mut Patch, blob_edits: &[BlobEdit]) {
	for edit in blob_edits {
		patch.write(edit.offset.get(), edit.bytes.clone());
	}
}

/// Replace the PE image and regenerate page descriptors + image_hash.
///
/// Requires the source XEX to be uncompressed, and either unencrypted or
/// decrypted by this plan, since we only rewrite data bytes verbatim. Attempts to preserve the existing
/// descriptor grouping as a template so the new table has the same shape.
fn apply_pe_replacement(
	xex: &Xex2,
	source: &[u8],
	plan: &EditPlan,
	patch: &mut Patch,
	editor: &mut ImageInfoEditor<'_>,
) -> Result<()> {
	let Some(new_pe) = plan.replace_pe.as_deref() else {
		return Ok(());
	};

	let file_format = xex.header.file_format_info()?;
	if file_format.compression_type != crate::header::CompressionType::None {
		return Err(Xex2Error::RebuildTransformNotImplemented.into_report());
	}
	if file_format.encryption_type != EncryptionType::None
		&& plan.target_encryption != Some(TargetEncryption::Decrypted)
	{
		return Err(Xex2Error::RebuildTransformNotImplemented.into_report());
	}

	// Write the new PE bytes in place of the old data region.
	let data_off = FileOffset::from(xex.header.data_offset);
	patch.write(data_off.get(), new_pe.to_vec());

	// Regenerate page descriptors using the existing grouping as a template
	// (so count/flags match the original layout even if hashes differ).
	let page_size =
		if xex.security_info.image_info.image_flags.contains(ImageFlags::SMALL_PAGES) { 0x1000 } else { 0x10000 };
	let template = read_page_descriptor_template(source, editor.sec, xex.security_info.page_descriptor_count);
	let crate::page_descriptors::GeneratedDescriptors { descriptors, image_hash } =
		crate::page_descriptors::generate(new_pe, page_size, template.as_deref());

	// Stage the new image_hash.
	editor.overwrite(ImageInfoOffset::IMAGE_HASH, image_hash.to_vec());

	// Stage the new image_size in security_info.
	let new_image_size = new_pe.len() as u32;
	let image_size_off = editor.sec + 0x04usize;
	patch.write(image_size_off.get(), new_image_size.to_be_bytes().to_vec());

	// Stage the descriptor table. count at sec+0x180, entries at sec+0x184.
	let count_off = editor.sec + SecurityOffset::PAGE_DESCRIPTOR_COUNT.0 as usize;
	patch.write(count_off.get(), (descriptors.len() as u32).to_be_bytes().to_vec());
	let desc_off = editor.sec + SecurityOffset::PAGE_DESCRIPTORS.0 as usize;
	let mut desc_bytes = Vec::with_capacity(descriptors.len() * 24);
	for d in &descriptors {
		desc_bytes.extend_from_slice(&d.to_bytes());
	}
	patch.write(desc_off.get(), desc_bytes);

	Ok(())
}

fn read_page_descriptor_template(
	source: &[u8],
	sec: FileOffset,
	count: u32,
) -> Option<Vec<crate::page_descriptors::DescriptorSlot>> {
	let base = (sec + SecurityOffset::PAGE_DESCRIPTORS.0 as usize).as_usize();
	let mut out = Vec::with_capacity(count as usize);
	for i in 0..count as usize {
		let off = base + i * 24;
		let bytes: &[u8; 4] = source.get(off..off + 4)?.try_into().ok()?;
		let info = u32::from_be_bytes(*bytes);
		out.push(crate::page_descriptors::DescriptorSlot {
			page_count: info >> 4,
			section_type: crate::page_descriptors::SectionType::from_nibble(info),
		});
	}
	Some(out)
}

fn sign_and_stage(sec: FileOffset, image_info: &[u8], patch: &mut Patch) -> Result<()> {
	let digest = xecrypt::symmetric::xe_crypt_rot_sum_sha(image_info, &[]);
	let sig = xecrypt::xex_sig::sign_devkit(&digest).map_err(|_| Xex2Error::SigningFailed.into_report())?;
	let sig_off = sec + SecurityOffset::RSA_SIG.0 as usize;
	patch.write(sig_off.get(), sig.to_vec());
	let info_off = sec + SecurityOffset::IMAGE_INFO.0 as usize;
	patch.write(info_off.get(), image_info.to_vec());
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

	/// Overwrite `bytes.len()` bytes at `field` within the image_info region.
	fn overwrite(&mut self, field: ImageInfoOffset, bytes: Vec<u8>) {
		let local = field.0 as usize;
		if local + bytes.len() > self.buffer.len() {
			return;
		}
		if self.buffer[local..local + bytes.len()] == bytes {
			return;
		}
		self.buffer[local..local + bytes.len()].copy_from_slice(&bytes);
		self.changed = true;
	}

	/// Read the big-endian u32 at `field`, apply `f` to produce a new value,
	/// and overwrite only if it actually changed.
	fn update_u32(&mut self, field: ImageInfoOffset, f: impl FnOnce(u32) -> u32) {
		let local = field.0 as usize;
		if local + 4 > self.buffer.len() {
			return;
		}
		let current = BigEndian::read_u32(&self.buffer[local..]);
		let new = f(current);
		if new != current {
			self.overwrite(field, new.to_be_bytes().to_vec());
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
	let image_flags = xex.security_info.image_info.image_flags;
	let current_machine = crate::hashes::verify_headers(source, xex)?;
	let master = master_key_for(current_machine, image_flags)?;
	let session_key = crate::crypto::unwrap_key(&xex.security_info.image_info.file_key, &master);
	let resolved_machine = plan.target_machine.unwrap_or(current_machine);

	// Re-wrap under target machine's master key, but only when it would change.
	if let Some(target) = plan.target_machine
		&& target != current_machine
	{
		let master = master_key_for(resolved_machine, image_flags)?;
		let new_file_key = crate::crypto::wrap_file_key(&session_key, &master);
		editor.overwrite(ImageInfoOffset::FILE_KEY, new_file_key.to_vec());
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
				editor.overwrite(ImageInfoOffset::FILE_KEY, vec![0u8; 16]);
			}
			(EncryptionType::None, TargetEncryption::Encrypted) => {
				let ciphertext = crate::crypto::encrypt_data(region, &session_key);
				patch.write(data_off.get(), ciphertext);
				let master = master_key_for(resolved_machine, image_flags)?;
				let wrapped = crate::crypto::wrap_file_key(&session_key, &master);
				editor.overwrite(ImageInfoOffset::FILE_KEY, wrapped.to_vec());
				stage_file_format_encryption(xex, source, EncryptionType::Normal, blob_edits)?;
			}
			// Already in target state.
			(EncryptionType::Normal, TargetEncryption::Encrypted)
			| (EncryptionType::None, TargetEncryption::Decrypted) => {}
		}
	}

	Ok(())
}

fn master_key_for(machine: TargetMachine, image_flags: crate::opt::ImageFlags) -> Result<crate::header::AesKey> {
	crate::crypto::master_key(machine, image_flags)
		.ok_or_else(|| Xex2Error::UnknownMasterKey { machine, image_flags: image_flags.bits() }.into_report())
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

#[cfg(test)]
mod date_range_tests {
	use super::*;

	#[test]
	fn unlimited_window() {
		let r = DateRangeEdit::unlimited();
		assert_eq!(r.not_before, 0);
		assert_eq!(r.not_after, u64::MAX);
	}

	#[cfg(feature = "jiff")]
	#[test]
	fn from_timestamps_round_trips_through_filetime() {
		let nb = jiff::Timestamp::from_second(1_700_000_000).unwrap();
		let na = jiff::Timestamp::from_second(1_800_000_000).unwrap();
		let r = DateRangeEdit::from_timestamps(nb, na).unwrap();
		let parsed_nb = xenon_types::filetime_to_timestamp(r.not_before).unwrap();
		let parsed_na = xenon_types::filetime_to_timestamp(r.not_after).unwrap();
		assert_eq!(parsed_nb.as_second(), 1_700_000_000);
		assert_eq!(parsed_na.as_second(), 1_800_000_000);
	}

	#[cfg(feature = "jiff")]
	#[test]
	fn from_timestamps_rejects_pre_unix_epoch() {
		let nb = jiff::Timestamp::from_second(-1).unwrap();
		let na = jiff::Timestamp::from_second(100).unwrap();
		assert!(DateRangeEdit::from_timestamps(nb, na).is_none());
	}
}
