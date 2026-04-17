//! Binary patches for describing modifications to an XEX file.
//!
//! A [`Patch`] is a list of length-preserving byte writes (each
//! [`PatchOp::Write`] overwrites `bytes.len()` bytes at a fixed offset).
//! Patches are storage-agnostic: the same patch can be applied to a
//! `Vec<u8>`, a slice, or streamed through any `Write` sink alongside the
//! source bytes.
//!
//! Patches are the small-edit branch of the modification story. Full
//! rebuilds (recompression, re-encryption, replacing the inner PE) go
//! through [`crate::rebuild::Rebuilder`] and produce a whole new file.

use std::io::Write;

use crate::error::Result;
use crate::error::Xex2Error;
use rootcause::IntoReport;

/// A single byte-level edit. Always length-preserving.
#[derive(Debug, Clone)]
pub(crate) struct PatchOp {
	pub offset: u64,
	pub bytes: Vec<u8>,
}

#[derive(Debug, Default, Clone)]
pub struct Patch {
	ops: Vec<PatchOp>,
}

impl Patch {
	pub(crate) fn new() -> Self {
		Self::default()
	}

	pub fn is_empty(&self) -> bool {
		self.ops.is_empty()
	}

	pub(crate) fn write(&mut self, offset: u64, bytes: impl Into<Vec<u8>>) {
		self.ops.push(PatchOp { offset, bytes: bytes.into() });
	}

	/// Apply the patch to an owned buffer.
	pub fn apply_to_vec(&self, buf: &mut Vec<u8>) -> Result<()> {
		self.apply_to_slice(buf.as_mut_slice())
	}

	/// Apply the patch to a fixed-size slice.
	pub fn apply_to_slice(&self, buf: &mut [u8]) -> Result<()> {
		for op in &self.ops {
			let start = op.offset as usize;
			let end = start + op.bytes.len();
			if end > buf.len() {
				return Err(Xex2Error::PatchOutOfBounds {
					offset: op.offset,
					len: op.bytes.len(),
					buf_len: buf.len(),
				}
				.into_report());
			}
			buf[start..end].copy_from_slice(&op.bytes);
		}
		Ok(())
	}

	/// Stream `source` to `sink` in one forward pass, substituting bytes
	/// from the patch as they pass under the cursor.
	pub fn stream_to<W: Write>(&self, source: &[u8], sink: &mut W) -> Result<()> {
		let ops = self.sorted_non_overlapping()?;
		let mut cursor = 0usize;
		for op in &ops {
			let start = op.offset as usize;
			let end = start + op.bytes.len();
			if end > source.len() {
				return Err(
					Xex2Error::PatchOutOfBounds { offset: op.offset, len: op.bytes.len(), buf_len: source.len() }
						.into_report(),
				);
			}
			if cursor < start {
				sink.write_all(&source[cursor..start]).map_err(|e| Xex2Error::Io(e).into_report())?;
			}
			sink.write_all(&op.bytes).map_err(|e| Xex2Error::Io(e).into_report())?;
			cursor = end;
		}
		if cursor < source.len() {
			sink.write_all(&source[cursor..]).map_err(|e| Xex2Error::Io(e).into_report())?;
		}
		Ok(())
	}

	fn sorted_non_overlapping(&self) -> Result<Vec<&PatchOp>> {
		let mut ops: Vec<&PatchOp> = self.ops.iter().collect();
		ops.sort_by_key(|op| op.offset);
		for pair in ops.windows(2) {
			let a = pair[0];
			let b = pair[1];
			if a.offset + a.bytes.len() as u64 > b.offset {
				return Err(Xex2Error::PatchOverlap.into_report());
			}
		}
		Ok(ops)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn apply_to_vec_write() {
		let mut buf = vec![0u8; 8];
		let mut p = Patch::new();
		p.write(2, vec![0xAA, 0xBB]);
		p.apply_to_vec(&mut buf).unwrap();
		assert_eq!(buf, vec![0, 0, 0xAA, 0xBB, 0, 0, 0, 0]);
	}

	#[test]
	fn stream_to_matches_apply_to_vec() {
		let source = (0u8..16).collect::<Vec<_>>();
		let mut p = Patch::new();
		p.write(4, vec![0xAA, 0xBB]);
		p.write(10, vec![0xCC]);

		let mut via_vec = source.clone();
		p.apply_to_vec(&mut via_vec).unwrap();

		let mut via_stream = Vec::new();
		p.stream_to(&source, &mut via_stream).unwrap();

		assert_eq!(via_vec, via_stream);
	}

	#[test]
	fn stream_to_rejects_overlap() {
		let source = vec![0u8; 16];
		let mut p = Patch::new();
		p.write(4, vec![0xAA, 0xBB, 0xCC]);
		p.write(5, vec![0xDD]);
		let mut sink = Vec::new();
		assert!(p.stream_to(&source, &mut sink).is_err());
	}
}
