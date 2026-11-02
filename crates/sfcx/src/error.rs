use std::ops::Range;

use thiserror::Error;

use crate::boot::ChainKind;
use crate::boot::StageName;
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
	#[error(
		"the root directory of a {kind} flash cannot be located without the block metadata in the spare areas; \
		 read a raw NAND image instead of a logical one"
	)]
	RootDirectoryNeedsRawImage { kind: ChainKind },
	#[error("the {kind} flash holds no complete root directory copy")]
	RootDirectoryNotFound { kind: RawKind },
	#[error("boot stage header at {offset:#x} is truncated")]
	BootStageHeaderTruncated { offset: usize },
	#[error("expected boot stage {expected} at {offset:#x}, got {actual}")]
	UnexpectedBootStage { offset: usize, expected: StageName, actual: StageName },
	#[error("boot stage {name} at {offset:#x} starts neither a devkit chain (\"SB\") nor a retail chain (\"CB\")")]
	UnknownBootChain { offset: usize, name: StageName },
	#[error("boot stage {name} has an invalid size {size:#x}")]
	InvalidBootStageSize { name: StageName, size: usize },
	#[error("boot stage {name} extends past the boot region")]
	BootStageOutOfBounds { name: StageName },
	#[error("patch slots of {size:#x} bytes at {offset:#x} are not a valid flash layout")]
	InvalidPatchSlots { offset: usize, size: usize },
	#[error("image of {actual:#x} bytes is shorter than its declared {declared:#x}-byte boot region")]
	BootRegionTruncated { declared: u64, actual: u64 },
	#[error(
		"SMC firmware of {size:#x} bytes at {offset:#x} is not between the flash header and the end of the boot region"
	)]
	SmcOutOfBounds { offset: usize, size: usize },
	#[error("root directory cluster holds {actual:#x} bytes; expected {expected:#x}")]
	RootDirectorySize { expected: usize, actual: usize },
	#[error("the observed profile requires a non-empty root directory")]
	EmptyRootDirectory,
	#[error("directory entry at {offset:#x} has an invalid name")]
	InvalidDirectoryName { offset: usize },
	#[error("directory entry at {offset:#x} has zero length")]
	EmptyFile { offset: usize },
	#[error("file cluster {cluster:#x} is outside the data blocks of the file system")]
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
