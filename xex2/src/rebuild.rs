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
	xex: &'a Xex2,
	source: &'a [u8],
	encryption: TargetEncryption,
	compression: TargetCompression,
	machine: TargetMachine,
	plan: EditPlan,
	replace_pe: Option<Vec<u8>>,
}

impl<'a> Rebuilder<'a> {
	pub fn new(xex: &'a Xex2, source: &'a [u8]) -> Self {
		Self {
			xex,
			source,
			encryption: TargetEncryption::Unchanged,
			compression: TargetCompression::Unchanged,
			machine: TargetMachine::Unchanged,
			plan: EditPlan::default(),
			replace_pe: None,
		}
	}

	// Transform setters (require a full rebuild).

	pub fn target_encryption(mut self, target: TargetEncryption) -> Self {
		self.encryption = target;
		self
	}

	pub fn target_compression(mut self, target: TargetCompression) -> Self {
		self.compression = target;
		self
	}

	pub fn target_machine(mut self, target: TargetMachine) -> Self {
		self.machine = target;
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

	/// True iff this rebuild can be expressed as a length-preserving [`Patch`].
	pub fn is_fast_path(&self) -> bool {
		self.encryption == TargetEncryption::Unchanged
			&& self.compression == TargetCompression::Unchanged
			&& self.machine == TargetMachine::Unchanged
			&& self.replace_pe.is_none()
	}

	/// If the rebuild is on the fast path, return the [`Patch`]; otherwise `None`.
	pub fn as_patch(&self) -> Result<Option<Patch>> {
		if self.is_fast_path() {
			Ok(Some(crate::writer::plan_edits(self.xex, self.source, &self.plan)?))
		} else {
			Ok(None)
		}
	}

	/// Stream the rebuilt XEX to `sink`. Fast path streams source + patch;
	/// transform paths aren't implemented yet.
	pub fn write_to<W: Write>(self, sink: &mut W) -> Result<()> {
		if !self.is_fast_path() {
			return Err(Xex2Error::RebuildTransformNotImplemented.into_report());
		}
		let patch = crate::writer::plan_edits(self.xex, self.source, &self.plan)?;
		patch.stream_to(self.source, sink)
	}
}
