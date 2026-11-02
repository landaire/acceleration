//! Sans-io operations. A machine names the byte range of the source it needs next; the caller
//! reads it from any source and feeds the bytes back. [`crate::io::drive`] and
//! [`crate::io::drive_async`] run a machine against a [`fskit::ReadAt`] or [`fskit::AsyncReadAt`]
//! source.

use std::ops::Range;

use crate::error::ReadError;
use crate::error::ReadErrorKind;

/// What a machine needs next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step<T> {
	/// Read this range of the source and pass its bytes to [`Machine::feed`].
	Read(Range<u64>),
	Done(T),
}

pub trait Machine {
	type Output;
	/// The machine's own error. A failed or short read converts into it.
	type Error: From<ReadError>;

	/// The next read, or the output once no read remains. A machine that returned
	/// [`Step::Done`] must not be stepped again.
	fn step(&mut self) -> Result<Step<Self::Output>, Self::Error>;

	/// Supplies the bytes of the range the last [`Machine::step`] requested.
	fn feed(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;
}

/// Rejects bytes that do not cover the requested range.
pub fn expect_len(range: &Range<u64>, bytes: &[u8]) -> Result<(), ReadError> {
	if bytes.len() as u64 != range.end - range.start {
		return Err(ReadError { range: range.clone(), kind: ReadErrorKind::Short { actual: bytes.len() } });
	}

	Ok(())
}
