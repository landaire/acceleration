//! Streaming rebuilds of an XEX file.
//!
//! [`Rebuilder`] holds the user's transform intent (target encryption,
//! compression, machine, inner PE replacement, limit removals) and writes a
//! new XEX to a `Write` sink without materializing the whole output in memory.
//!
//! Small, length-preserving edits go through [`crate::writer::plan_edits`]
//! instead. The rebuilder's fast path delegates to that when all transforms
//! are [`TargetEncryption::Unchanged`] / [`TargetCompression::Unchanged`] /
//! [`TargetMachine::Unchanged`] and no PE replacement is requested -- it
//! produces a [`Patch`] and streams it forward through the source bytes.
//!
//! The transform paths (recompression, re-encryption, PE replacement) are
//! stubbed out for now; the API shape is in place so they can be filled in
//! incrementally.

use std::io::Write;

use crate::Xex2;
use crate::error::Result;
use crate::error::Xex2Error;
use crate::patch::Patch;
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
	limits: RemoveLimits,
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
			limits: RemoveLimits::default(),
			replace_pe: None,
		}
	}

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

	pub fn remove_limits(mut self, limits: RemoveLimits) -> Self {
		self.limits = limits;
		self
	}

	pub fn replace_pe(mut self, pe: Vec<u8>) -> Self {
		self.replace_pe = Some(pe);
		self
	}

	/// True iff this rebuild can be expressed as a length-preserving [`Patch`]
	/// over the source bytes.
	pub fn is_fast_path(&self) -> bool {
		self.encryption == TargetEncryption::Unchanged
			&& self.compression == TargetCompression::Unchanged
			&& self.machine == TargetMachine::Unchanged
			&& self.replace_pe.is_none()
	}

	/// If the rebuild is on the fast path, return the [`Patch`]; otherwise
	/// `None`. Callers that want streaming for both cases should use
	/// [`write_to`][Self::write_to].
	pub fn as_patch(&self) -> Result<Option<Patch>> {
		if self.is_fast_path() {
			Ok(Some(crate::writer::plan_edits(self.xex, self.source, &self.limits)?))
		} else {
			Ok(None)
		}
	}

	/// Stream the rebuilt XEX to `sink`. Fast path emits source with the
	/// planned edits substituted; full rebuilds are not yet implemented.
	pub fn write_to<W: Write>(self, sink: &mut W) -> Result<()> {
		if !self.is_fast_path() {
			return Err(Xex2Error::RebuildTransformNotImplemented.into_report());
		}
		let patch = crate::writer::plan_edits(self.xex, self.source, &self.limits)?;
		patch.stream_to(self.source, sink)
	}
}
