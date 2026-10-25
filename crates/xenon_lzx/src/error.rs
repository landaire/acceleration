use thiserror::Error;

/// A failure of [`Decoder`][crate::Decoder]. After an error other than
/// [`Error::InvalidWindowSize`], [`Error::WindowBufferSize`],
/// [`Error::FrameTooLarge`] and [`Error::ReferenceTooLarge`], the decoder state
/// is unspecified until [`Decoder::reset`][crate::Decoder::reset].
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum Error {
	#[error("window size {size:#x} is not a power of two from 0x8000 to 0x200000")]
	InvalidWindowSize { size: usize },
	#[error("window buffer holds {got:#x} bytes, the window size is {expected:#x}")]
	WindowBufferSize { expected: usize, got: usize },
	#[error("frame of {requested:#x} bytes exceeds the {max:#x}-byte frame limit")]
	FrameTooLarge { requested: usize, max: usize },
	#[error("frame of {len:#x} bytes at window position {position:#x} crosses the end of the window")]
	FrameCrossesWindowEnd { position: u32, len: usize },
	#[error("reference data of {len:#x} bytes exceeds the {window:#x}-byte window")]
	ReferenceTooLarge { len: usize, window: usize },
	#[error("block type {kind} is not verbatim (1), aligned offset (2) or uncompressed (3)")]
	InvalidBlockType { kind: u32 },
	#[error("compressed input ended inside a frame")]
	InputExhausted,
	/// The kernel decodes with uninitialized stack memory in this case.
	#[error("pretree code lengths are neither a complete code nor all zero")]
	IncompletePretree,
	/// The kernel writes past its length counts in this case.
	#[error("code length {length} exceeds 16")]
	CodeLengthTooLong { length: u8 },
	#[error("decoding table has no room for another node")]
	DecodeTableFull,
	#[error("a match ends {overrun:#x} bytes past the end of the frame")]
	MatchOverrunsFrame { overrun: u32 },
}
