use thiserror::Error;

use crate::fat::ClusterNumber;
use crate::volume_file::RejectedCandidate;

pub use xenon_io::ReadError;
pub use xenon_io::ReadErrorKind;

/// A volume header or partition length that the FATX driver would not mount.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum GeometryError {
	#[error("volume signature {signature:02x?} is not XTAF")]
	BadSignature { signature: [u8; 4] },
	#[error("{sectors_per_cluster:#x} sectors per cluster is not a power of two from 1 to 128")]
	InvalidSectorsPerCluster { sectors_per_cluster: u32 },
	#[error("an alternate partition needs {required:#x}-byte clusters, not {bytes_per_cluster:#x}")]
	AlternateClusterSize { bytes_per_cluster: u32, required: u32 },
	#[error("{bytes_per_cluster:#x}-byte clusters are smaller than the {metadata_size:#x}-byte metadata page")]
	ClusterSmallerThanMetadata { bytes_per_cluster: u32, metadata_size: u64 },
	#[error("a partition of {partition_length:#x} bytes cannot hold the metadata page")]
	PartitionTooSmall { partition_length: u64 },
	#[error("the {fat_length:#x}-byte FAT does not fit in the {available:#x} bytes after the metadata page")]
	FatTooLarge { fat_length: u64, available: u64 },
	#[error("{clusters:#x} clusters exceed the FATX maximum")]
	TooManyClusters { clusters: u64 },
	#[error("root directory cluster {cluster:#x} is not a data cluster of the volume")]
	InvalidRootCluster { cluster: u32 },
}

/// A cluster chain that the FAT does not support.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ChainError {
	#[error("a non-empty file has no first cluster")]
	MissingFirstCluster,
	#[error("cluster {cluster} is not a data cluster of the volume")]
	InvalidCluster { cluster: ClusterNumber },
	#[error("cluster {cluster} links to {next:#x}, which is not a data cluster")]
	InvalidLink { cluster: ClusterNumber, next: u32 },
	#[error("the cluster chain from {first} loops")]
	Cycle { first: ClusterNumber },
	#[error("the cluster chain from {first} holds {available} clusters; {needed} are needed")]
	TooShort { first: ClusterNumber, needed: u64, available: u64 },
	#[error("cluster {cluster} lies past the end of the image")]
	OutsideImage { cluster: ClusterNumber },
}

/// A directory that the FATX driver would fail to enumerate.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryError {
	#[error(transparent)]
	Chain(#[from] ChainError),
	#[error("directory at cluster {first} holds more than {maximum:#x} bytes of entries")]
	TooLarge { first: ClusterNumber, maximum: u64 },
	#[error("directory entry has no first cluster")]
	MissingFirstCluster,
}

#[derive(Debug, Error)]
pub enum FatxError {
	#[error(transparent)]
	Geometry(#[from] GeometryError),
	#[error(transparent)]
	Chain(#[from] ChainError),
	#[error(transparent)]
	Directory(#[from] DirectoryError),
	#[error("the volume needs {required:#x} bytes but the image range holds {available:#x}")]
	ImageTooShort { required: u64, available: u64 },
	#[error("entry {name} is a directory")]
	NotAFile { name: String },
	#[error("entry {name} is not a directory")]
	NotADirectory { name: String },
	#[error("the requested data holds more bytes than this host can buffer")]
	ReadTooLarge,
	#[error("no FAT length fits the volume file; {} candidates were rejected", .rejected.len())]
	PartitionLengthNotInferred { rejected: Vec<RejectedCandidate> },
	#[error(transparent)]
	Read(#[from] ReadError),
}
