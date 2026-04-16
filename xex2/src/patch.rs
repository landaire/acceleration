//! Binary patches for describing modifications to an XEX file.
//!
//! A [`Patch`] is a list of byte-level edits: [`Write`][PatchOp::Write] replaces
//! a same-length region, [`Splice`][PatchOp::Splice] replaces a region with
//! bytes of a different length. Patches are storage-agnostic -- the same
//! `Patch` can be applied to a `Vec<u8>`, a slice, or streamed through any
//! `Write` sink alongside the source bytes.
//!
//! Patches are the small-edit branch of the modification story. Full rebuilds
//! (recompression, re-encryption, replacing the inner PE) go through
//! [`crate::rebuild::Rebuilder`] instead.

use std::io::Write;

use crate::error::Result;
use crate::error::Xex2Error;
use rootcause::IntoReport;

/// A single byte-level edit.
#[derive(Debug, Clone)]
pub enum PatchOp {
	/// Overwrite `bytes.len()` bytes starting at `offset`. Length-preserving.
	Write { offset: u64, bytes: Vec<u8> },
	/// Remove `remove_len` bytes at `offset`, insert `insert` in their place.
	/// Shifts everything after.
	Splice { offset: u64, remove_len: u64, insert: Vec<u8> },
}

impl PatchOp {
	fn offset(&self) -> u64 {
		match self {
			PatchOp::Write { offset, .. } => *offset,
			PatchOp::Splice { offset, .. } => *offset,
		}
	}
}

#[derive(Debug, Default, Clone)]
pub struct Patch {
	ops: Vec<PatchOp>,
}

impl Patch {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn is_empty(&self) -> bool {
		self.ops.is_empty()
	}

	pub fn ops(&self) -> &[PatchOp] {
		&self.ops
	}

	pub fn write(&mut self, offset: u64, bytes: impl Into<Vec<u8>>) {
		self.ops.push(PatchOp::Write { offset, bytes: bytes.into() });
	}

	pub fn splice(&mut self, offset: u64, remove_len: u64, insert: impl Into<Vec<u8>>) {
		self.ops.push(PatchOp::Splice { offset, remove_len, insert: insert.into() });
	}

	/// Apply all ops (Write + Splice) to an owned buffer.
	pub fn apply_to_vec(&self, buf: &mut Vec<u8>) -> Result<()> {
		let mut sorted = self.sorted_ops()?;
		// Apply in reverse offset order so earlier splices don't shift later offsets.
		sorted.sort_by_key(|op| std::cmp::Reverse(op.offset()));
		for op in sorted {
			match op {
				PatchOp::Write { offset, bytes } => {
					let start = offset as usize;
					let end = start + bytes.len();
					if end > buf.len() {
						return Err(Xex2Error::PatchOutOfBounds {
							offset,
							len: bytes.len(),
							buf_len: buf.len(),
						}
						.into_report());
					}
					buf[start..end].copy_from_slice(&bytes);
				}
				PatchOp::Splice { offset, remove_len, insert } => {
					let start = offset as usize;
					let end = start + remove_len as usize;
					if end > buf.len() {
						return Err(Xex2Error::PatchOutOfBounds {
							offset,
							len: remove_len as usize,
							buf_len: buf.len(),
						}
						.into_report());
					}
					buf.splice(start..end, insert.into_iter());
				}
			}
		}
		Ok(())
	}

	/// Apply Write ops to a fixed-size slice. Errors on Splice.
	pub fn apply_to_slice(&self, buf: &mut [u8]) -> Result<()> {
		for op in &self.ops {
			match op {
				PatchOp::Write { offset, bytes } => {
					let start = *offset as usize;
					let end = start + bytes.len();
					if end > buf.len() {
						return Err(Xex2Error::PatchOutOfBounds {
							offset: *offset,
							len: bytes.len(),
							buf_len: buf.len(),
						}
						.into_report());
					}
					buf[start..end].copy_from_slice(bytes);
				}
				PatchOp::Splice { .. } => {
					return Err(Xex2Error::PatchHasSplice.into_report());
				}
			}
		}
		Ok(())
	}

	/// Stream `source` to `sink` in one forward pass, substituting bytes from
	/// Write ops as they pass under the cursor. Length-preserving only --
	/// errors on Splice.
	pub fn stream_to<W: Write>(&self, source: &[u8], sink: &mut W) -> Result<()> {
		let ops = self.sorted_writes_only()?;
		let mut cursor = 0usize;
		for (offset, bytes) in &ops {
			let start = *offset as usize;
			let end = start + bytes.len();
			if end > source.len() {
				return Err(Xex2Error::PatchOutOfBounds {
					offset: *offset,
					len: bytes.len(),
					buf_len: source.len(),
				}
				.into_report());
			}
			if cursor < start {
				sink.write_all(&source[cursor..start]).map_err(|e| Xex2Error::Io(e).into_report())?;
			}
			sink.write_all(bytes).map_err(|e| Xex2Error::Io(e).into_report())?;
			cursor = end;
		}
		if cursor < source.len() {
			sink.write_all(&source[cursor..]).map_err(|e| Xex2Error::Io(e).into_report())?;
		}
		Ok(())
	}

	fn sorted_ops(&self) -> Result<Vec<PatchOp>> {
		let mut sorted = self.ops.clone();
		sorted.sort_by_key(|op| op.offset());
		Ok(sorted)
	}

	fn sorted_writes_only(&self) -> Result<Vec<(u64, &[u8])>> {
		let mut ops: Vec<(u64, &[u8])> = Vec::with_capacity(self.ops.len());
		for op in &self.ops {
			match op {
				PatchOp::Write { offset, bytes } => ops.push((*offset, bytes.as_slice())),
				PatchOp::Splice { .. } => {
					return Err(Xex2Error::PatchHasSplice.into_report());
				}
			}
		}
		ops.sort_by_key(|(o, _)| *o);
		for pair in ops.windows(2) {
			let (a_off, a_bytes) = &pair[0];
			let (b_off, _) = &pair[1];
			if a_off + a_bytes.len() as u64 > *b_off {
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
	fn apply_to_vec_splice_grows() {
		let mut buf = vec![1, 2, 3, 4];
		let mut p = Patch::new();
		p.splice(1, 2, vec![9, 9, 9]);
		p.apply_to_vec(&mut buf).unwrap();
		assert_eq!(buf, vec![1, 9, 9, 9, 4]);
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
