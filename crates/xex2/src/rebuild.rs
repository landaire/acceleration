//! Streaming rebuilds of an XEX file.
//!
//! [`Rebuilder`] holds the user's transform intent (target encryption,
//! compression, machine, inner PE replacement, limit removals, arbitrary
//! per-field overrides) and writes a new XEX to a `Write` sink without
//! materializing the whole output in memory when the edits are
//! length-preserving.
//!
//! Edits come in two flavors:
//! - **Length-preserving edits** (limit recipes, per-field overrides of fields
//!   that stay the same size): fast path, expressed as a [`Patch`] that streams
//!   through source bytes.
//! - **Transformative edits** (recompression, re-encryption, PE replacement):
//!   require a full rebuild. These currently return
//!   [`Xex2Error::RebuildTransformNotImplemented`][crate::error::Xex2Error::RebuildTransformNotImplemented]
//!   until Phase 2+ transforms land.

use std::io::Write;

use crate::Xex2;
use crate::error::Result;
use crate::error::Xex2Error;
use crate::header::OptionalHeaderKey;
use crate::opt::AllowedMediaTypes;
use crate::opt::ImageFlags;
use crate::opt::ModuleFlags;
use crate::patch::Patch;
use crate::writer::EditPlan;
use crate::writer::RemoveLimits;
use crate::writer::TargetCompression;
use crate::writer::TargetEncryption;
use crate::writer::TargetMachine;
use rootcause::IntoReport;

pub struct Rebuilder<'a> {
	xex: Xex2,
	source: &'a [u8],
	compression: Option<TargetCompression>,
	plan: EditPlan,
}

impl<'a> Rebuilder<'a> {
	pub fn new(xex: Xex2, source: &'a [u8]) -> Self {
		Self { xex, source, compression: None, plan: EditPlan::default() }
	}

	// Transform setters. Each sets the target state; if it already matches the
	// current XEX, the transform is a no-op. Not calling a setter leaves the
	// corresponding aspect untouched.

	pub fn target_encryption(mut self, target: TargetEncryption) -> Self {
		self.plan.target_encryption = Some(target);
		self
	}

	pub fn target_compression(mut self, target: TargetCompression) -> Self {
		self.compression = Some(target);
		self
	}

	pub fn target_machine(mut self, target: TargetMachine) -> Self {
		self.plan.target_machine = Some(target);
		self
	}

	pub fn replace_pe(mut self, pe: Vec<u8>) -> Self {
		self.plan.replace_pe = Some(pe);
		self
	}

	// Recipe setter.

	pub fn remove_limits(mut self, limits: RemoveLimits) -> Self {
		self.plan.limits = limits;
		self
	}

	// Per-field setters. Values set here override anything a recipe would
	// have computed for the same field.

	pub fn set_module_flags(mut self, flags: ModuleFlags) -> Self {
		self.plan.module_flags = Some(flags);
		self
	}

	pub fn set_image_flags(mut self, flags: ImageFlags) -> Self {
		self.plan.image_flags = Some(flags);
		self
	}

	pub fn set_game_regions(mut self, regions: u32) -> Self {
		self.plan.game_regions = Some(regions);
		self
	}

	pub fn set_allowed_media(mut self, media: AllowedMediaTypes) -> Self {
		self.plan.allowed_media = Some(media);
		self
	}

	pub fn set_media_id(mut self, id: [u8; 16]) -> Self {
		self.plan.media_id = Some(id);
		self
	}

	pub fn set_file_key(mut self, key: xenon_types::AesKey) -> Self {
		self.plan.file_key = Some(key);
		self
	}

	pub fn set_load_address(mut self, address: xenon_types::VirtualAddress) -> Self {
		self.plan.load_address = Some(address);
		self
	}

	pub fn set_date_range(mut self, range: crate::writer::DateRangeEdit) -> Self {
		self.plan.date_range = Some(range);
		self
	}

	pub fn clear_optional_header(mut self, key: OptionalHeaderKey) -> Self {
		self.plan.cleared_optional_headers.push(key);
		self
	}

	/// True iff this rebuild is supported by the current implementation.
	///
	/// `Basic` compression transforms aren't implemented; everything else is.
	pub fn is_supported(&self) -> bool {
		// Only Basic is currently unsupported -- either as a target or if the
		// source is Basic-compressed and the target differs.
		if matches!(self.compression, Some(TargetCompression::Basic)) {
			return false;
		}
		if let Ok(ff) = self.xex.header.file_format_info() {
			if ff.compression_type == crate::header::CompressionType::Basic
				&& self.compression.is_some_and(|c| c != TargetCompression::Basic)
			{
				return false;
			}
			// PE replacement on Basic-compressed sources also isn't handled.
			if ff.compression_type == crate::header::CompressionType::Basic && self.plan.replace_pe.is_some() {
				return false;
			}
		}
		true
	}

	/// Produce the [`Patch`] representing this rebuild, if supported and
	/// length-preserving. Compression changes require a full rebuild and
	/// return `Ok(None)`; callers should use [`Self::write_to`] instead.
	pub fn as_patch(&self) -> Result<Option<Patch>> {
		if !self.is_supported() {
			return Ok(None);
		}
		if self.needs_full_rebuild() {
			return Ok(None);
		}
		Ok(Some(crate::writer::plan_edits(&self.xex, self.source, &self.plan)?))
	}

	/// Stream the rebuilt XEX to `sink`. For length-preserving edits this
	/// streams the source through a [`Patch`]; compression changes (or PE
	/// replacement on a compressed source) trigger a full assemble-from-parts
	/// path in [`crate::assemble::rebuild_with_compression`], after which any
	/// remaining per-field edits run through the normal patch path.
	pub fn write_to<W: Write>(self, sink: &mut W) -> Result<()> {
		if !self.is_supported() {
			return Err(Xex2Error::RebuildTransformNotImplemented.into_report());
		}
		if self.needs_full_rebuild() {
			let bytes = self.full_rebuild()?;
			sink.write_all(&bytes).map_err(|e| Xex2Error::Io(e).into_report())?;
			return Ok(());
		}
		let patch = crate::writer::plan_edits(&self.xex, self.source, &self.plan)?;
		patch.stream_to(self.source, sink)
	}

	fn needs_full_rebuild(&self) -> bool {
		let Ok(ff) = self.xex.header.file_format_info() else { return false };
		let current = ff.compression_type;
		let target_changes_compression = self.compression.is_some_and(|c| {
			!matches!(
				(current, c),
				(crate::header::CompressionType::None, TargetCompression::Uncompressed)
					| (crate::header::CompressionType::Basic, TargetCompression::Basic)
					| (crate::header::CompressionType::Normal, TargetCompression::Normal)
			)
		});
		// PE replacement on a compressed source has to go through the full
		// path: the source's data region holds compressed bytes we need to
		// rewrite end-to-end (either by re-compressing the replacement or by
		// switching the stream to None).
		let pe_needs_decompress_flow =
			self.plan.replace_pe.is_some() && current != crate::header::CompressionType::None;
		target_changes_compression || pe_needs_decompress_flow
	}

	fn full_rebuild(mut self) -> Result<Vec<u8>> {
		let current = self.xex.header.file_format_info().map(|ff| ff.compression_type).unwrap_or(
			// Fallback -- the `is_supported` gate already checked file_format_info parses.
			crate::header::CompressionType::None,
		);
		// Resolve target compression: the caller's request wins, else keep current.
		let target = self.compression.unwrap_or(match current {
			crate::header::CompressionType::None => TargetCompression::Uncompressed,
			crate::header::CompressionType::Normal => TargetCompression::Normal,
			crate::header::CompressionType::Basic => TargetCompression::Basic,
			crate::header::CompressionType::Delta => {
				return Err(Xex2Error::RebuildTransformNotImplemented.into_report());
			}
		});

		// Move the PE replacement into the assembler (no clone). `self.plan`
		// is left with `replace_pe = None`, so the follow-up `plan_edits` pass
		// naturally skips the PE-replacement branch.
		let pe_replacement = self.plan.replace_pe.take();
		let assembled = crate::assemble::rebuild_with_compression(&self.xex, self.source, target, pe_replacement)?;

		// Apply remaining per-field edits (limits, flags, encryption, etc.)
		// to the freshly-assembled file via the patch path.
		if self.plan.is_empty() {
			return Ok(assembled);
		}
		let parsed = crate::Xex2::parse(&assembled)?;
		let patch = crate::writer::plan_edits(&parsed, &assembled, &self.plan)?;
		let mut buf = assembled;
		patch.apply_to_vec(&mut buf)?;
		Ok(buf)
	}
}
