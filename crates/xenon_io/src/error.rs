use std::io;
use std::ops::Range;

use thiserror::Error;

/// A read of `range` that did not produce its bytes.
#[derive(Debug, Error)]
#[error("reading {range:#x?} failed")]
pub struct ReadError {
	pub range: Range<u64>,
	#[source]
	pub kind: ReadErrorKind,
}

#[derive(Debug, Error)]
pub enum ReadErrorKind {
	/// The source failed the read.
	#[error(transparent)]
	Io(io::Error),
	/// The source returned a different number of bytes than the range holds.
	#[error("the source returned {actual:#x} bytes")]
	Short { actual: usize },
}

/// A write or flush that the source failed.
#[derive(Debug, Error)]
pub enum WriteError {
	/// Any prefix of the bytes may have reached the source.
	#[error("writing {length:#x} bytes at {offset:#x} failed")]
	Write {
		offset: u64,
		length: u64,
		#[source]
		source: io::Error,
	},
	/// The earlier writes may not be durable.
	#[error("making the writes durable failed")]
	Flush(#[source] io::Error),
}
