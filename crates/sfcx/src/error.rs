use std::ops::Range;

use thiserror::Error;

use crate::raw::BlockIndex;
use crate::raw::RawKind;

pub use xenon_io::ReadError;
pub use xenon_io::ReadErrorKind;

/// A malformed or unsupported flash header, boot chain, or file system.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
	#[error("image is shorter than the fixed image header")]
	HeaderTruncated,
	#[error("unsupported image magic {magic:02x?}")]
	UnsupportedMagic { magic: [u8; 2] },
	#[error("the observed devkit profile requires a 0x3c00000-byte logical device, got {actual:#x}")]
	UnsupportedImageSize { actual: u64 },
	#[error("unsupported observed-profile value for {field}: expected {expected:#x}, got {actual:#x}")]
	UnsupportedProfileValue { field: &'static str, expected: u64, actual: u64 },
	#[error("boot stage header at {offset:#x} is truncated")]
	BootStageHeaderTruncated { offset: usize },
	#[error("expected boot stage {expected:02x?} at {offset:#x}, got {actual:02x?}")]
	UnexpectedBootStage { offset: usize, expected: [u8; 2], actual: [u8; 2] },
	#[error("boot stage {name:02x?} has an invalid size {size:#x}")]
	InvalidBootStageSize { name: [u8; 2], size: usize },
	#[error("boot stage {name:02x?} extends past the boot region")]
	BootStageOutOfBounds { name: [u8; 2] },
	#[error("image of {actual:#x} bytes is shorter than its declared {declared:#x}-byte boot region")]
	BootRegionTruncated { declared: u64, actual: u64 },
	#[error("root directory cluster holds {actual:#x} bytes; expected {expected:#x}")]
	RootDirectorySize { expected: usize, actual: usize },
	#[error("the observed profile requires a non-empty root directory")]
	EmptyRootDirectory,
	#[error("directory entry at {offset:#x} has an invalid name")]
	InvalidDirectoryName { offset: usize },
	#[error("directory entry at {offset:#x} has zero length")]
	EmptyFile { offset: usize },
	#[error("file cluster {cluster:#x} is outside the observed data region")]
	FileClusterOutOfRange { cluster: u16 },
	#[error("cluster {cluster:#x} has no allocation entry")]
	AllocationEntryOutOfBounds { cluster: u16 },
	#[error("cluster chain entered free cluster {cluster:#x}")]
	FreeClusterInChain { cluster: u16 },
	#[error("cluster chain ended before the declared file size")]
	TruncatedClusterChain,
	#[error("cluster chain contains a cycle")]
	ClusterChainCycle,
	#[error("cluster chain continues from final cluster {cluster:#x} to {next:#x}")]
	ClusterChainNotTerminated { cluster: u16, next: u16 },
}

/// A raw NAND image that does not fit a supported geometry.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum RawError {
	#[error("raw image size {size:#x} is not a whole number of 0x210-byte sectors")]
	PartialSector { size: u64 },
	#[error("{data_size:#x} bytes of flash data do not form a supported {kind} device")]
	UnsupportedGeometry { kind: RawKind, data_size: u64 },
	#[error("the raw image matches no supported flash geometry")]
	UnknownGeometry,
	#[error("the raw image matches the {first} and {second} geometries equally")]
	AmbiguousGeometry { first: RawKind, second: RawKind },
	#[error("logical block {block:?} is bad and no spare block replaces it")]
	UntranslatedBlock { block: BlockIndex },
}

/// A flash image that has no memory unit the kernel would mount, or a request outside it.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MemoryUnitError {
	#[error("a logical image has no spare areas, so its memory unit cannot be read")]
	LogicalImage,
	#[error("the kernel mounts a memory unit only on big-block flash, not {kind} flash")]
	UnsupportedKind { kind: RawKind },
	#[error("{usable_blocks} usable blocks leave no memory unit data blocks")]
	NoDataBlocks { usable_blocks: u64 },
	#[error("the {size:#x}-byte memory unit is smaller than the 0xc00000-byte minimum")]
	TooSmall { size: u64 },
	#[error("range {range:#x?} extends past the {size:#x}-byte memory unit")]
	RangeOutOfBounds { range: Range<u64>, size: u64 },
}

#[derive(Debug, Error)]
pub enum SfcxError {
	#[error(transparent)]
	Parse(#[from] ParseError),
	#[error(transparent)]
	Raw(#[from] RawError),
	#[error(transparent)]
	MemoryUnit(#[from] MemoryUnitError),
	#[error("the image is neither a logical image ({logical}) nor a raw image ({raw})")]
	Unrecognized { logical: ParseError, raw: RawError },
	#[error("the requested ranges hold more bytes than this host can buffer")]
	ReadTooLarge,
	#[error("logical range {range:#x?} extends past the {size:#x}-byte logical device")]
	LogicalRangeOutOfBounds { range: Range<u64>, size: u64 },
	#[error(transparent)]
	Read(#[from] ReadError),
}
