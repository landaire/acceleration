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
	replace_pe: Option<Vec<u8>>,
}

impl<'a> Rebuilder<'a> {
	pub fn new(xex: Xex2, source: &'a [u8]) -> Self {
		Self {
			xex,
			source,
			compression: None,
			plan: EditPlan::default(),
			replace_pe: None,
		}
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
		self.replace_pe = Some(pe);
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

	pub fn set_file_key(mut self, key: [u8; 16]) -> Self {
		self.plan.file_key = Some(key);
		self
	}

	pub fn set_load_address(mut self, address: u32) -> Self {
		self.plan.load_address = Some(address);
		self
	}

	pub fn set_date_range(mut self, not_before: u64, not_after: u64) -> Self {
		self.plan.date_range = Some((not_before, not_after));
		self
	}

	pub fn clear_optional_header(mut self, key: OptionalHeaderKey) -> Self {
		self.plan.cleared_optional_headers.push(key);
		self
	}

	/// True iff this rebuild doesn't require compression changes or PE
	/// replacement. Setting a target that already matches is still supported.
	pub fn is_supported(&self) -> bool {
		let compression_changes = self.compression.is_some_and(|c| {
			// A no-op if the current XEX already matches.
			self.xex.header.file_format_info().is_ok_and(|ff| match (ff.compression_type, c) {
				(crate::header::CompressionType::None, TargetCompression::Uncompressed) => false,
				(crate::header::CompressionType::Basic, TargetCompression::Basic) => false,
				(crate::header::CompressionType::Normal, TargetCompression::Normal) => false,
				_ => true,
			})
		});
		!compression_changes && self.replace_pe.is_none()
	}

	/// Produce the [`Patch`] representing this rebuild, if supported.
	pub fn as_patch(&self) -> Result<Option<Patch>> {
		if self.is_supported() {
			Ok(Some(crate::writer::plan_edits(&self.xex, self.source, &self.plan)?))
		} else {
			Ok(None)
		}
	}

	/// Stream the rebuilt XEX to `sink`. Compression changes (other than
	/// no-op ones) and PE replacement aren't implemented yet.
	pub fn write_to<W: Write>(self, sink: &mut W) -> Result<()> {
		if !self.is_supported() {
			return Err(Xex2Error::RebuildTransformNotImplemented.into_report());
		}
		let patch = crate::writer::plan_edits(&self.xex, self.source, &self.plan)?;
		patch.stream_to(self.source, sink)
	}
}
