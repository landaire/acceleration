//! Sans-io operations. A machine names the byte range of the source it needs next; the caller
//! reads it from any source and feeds the bytes back. [`crate::io::drive`] and
//! [`crate::io::drive_async`] run a machine against a [`fskit::ReadAt`] or [`fskit::AsyncReadAt`]
//! source.
//!
//! A [`WriteMachine`] also names bytes to write, in the order the machine needs them to reach
//! the source, and [`Effect::Flush`] barriers that make the writes before them durable before
//! any write after them is issued. A machine can so document what a failure or a power loss
//! part way through leaves behind. [`crate::io::drive_write`] and
//! [`crate::io::drive_write_async`] run one against a source that implements both the read and
//! the write traits.

use std::ops::Range;

use crate::error::ReadError;
use crate::error::ReadErrorKind;
use crate::error::WriteError;

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

/// What a write machine needs next.
#[derive(Debug, PartialEq, Eq)]
pub enum Effect<'a, T> {
	/// Read this range of the source and pass its bytes to [`WriteMachine::feed`].
	Read(Range<u64>),
	/// Write `data` at `offset` of the source, then step again. The write must complete before
	/// the next step.
	Write {
		offset: u64,
		data: &'a [u8],
	},
	/// Make every write so far durable, then step again.
	Flush,
	Done(T),
}

/// A machine that reads and writes a source.
pub trait WriteMachine {
	type Output;
	/// The machine's own error. A failed or short read and a failed write convert into it.
	type Error: From<ReadError> + From<WriteError>;

	/// The next read or write, or the output once nothing remains. A machine that returned
	/// [`Effect::Done`] must not be stepped again.
	fn step(&mut self) -> Result<Effect<'_, Self::Output>, Self::Error>;

	/// Supplies the bytes of the range the last [`WriteMachine::step`] requested.
	fn feed(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;
}

/// Rejects bytes that do not cover the requested range.
pub fn expect_len(range: &Range<u64>, bytes: &[u8]) -> Result<(), ReadError> {
	if bytes.len() as u64 != range.end - range.start {
		return Err(ReadError { range: range.clone(), kind: ReadErrorKind::Short { actual: bytes.len() } });
	}

	Ok(())
}
