use std::error::Error;
use std::io;
use std::num::ParseIntError;
use std::path::PathBuf;

use fatx::ClusterNumber;
use fatx::FatxError;
use fatx::IntegrityError;
use fatx::ReadError;
use thiserror::Error;
use xenon_disk::DetectError;
use xenon_disk::DeviceKind;
use xenon_disk::DevkitTableError;
use xenon_disk::DiskWarning;
use xenon_disk::UsbStorageError;

/// A command-line number that does not parse.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum NumberError {
	#[error("{text:?} is not a decimal or 0x-prefixed hexadecimal number")]
	Invalid {
		text: String,
		#[source]
		source: ParseIntError,
	},
	#[error("{value:#x} does not fit in 32 bits")]
	TooLarge { value: u64 },
	#[error("cluster 0 is not a data cluster")]
	ZeroCluster,
}

/// An output file that cannot be written.
#[derive(Debug, Error)]
pub enum OutputError {
	#[error("cannot identify file {path:?}")]
	IdentifyFile {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("cannot create output {path:?}")]
	CreateOutput {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("output {output:?} aliases input {input:?}")]
	OutputAliasesInput { input: PathBuf, output: PathBuf },
	#[error("cannot truncate output {path:?}")]
	TruncateOutput {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("cannot write output {path:?}")]
	WriteOutput {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("cannot create directory {path:?}")]
	CreateDirectory {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("{path:?} is a symbolic link; outputs are not written through links")]
	SymbolicLink { path: PathBuf },
	#[error("{path:?} is not a regular file; only image files are written, never devices")]
	NotARegularFile { path: PathBuf },
	#[error("cannot open image {path:?} for writing")]
	OpenWritableImage {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("output {path:?} names no file")]
	NoFileName { path: PathBuf },
	#[error("cannot rename {from:?} to {to:?}")]
	RenameOutput {
		from: PathBuf,
		to: PathBuf,
		#[source]
		source: io::Error,
	},
}

/// A deleted entry that a recovery path names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeletedCandidate {
	pub first_cluster: Option<ClusterNumber>,
	pub size: u32,
}

#[derive(Debug, Error)]
pub enum FatxToolError {
	#[error(transparent)]
	Fatx(#[from] FatxError),
	#[error(transparent)]
	Integrity(#[from] IntegrityError),
	#[error("reads of the volume source failed integrity checks")]
	SourceIntegrity(#[source] Box<dyn Error + Send + Sync>),
	#[error(transparent)]
	Read(#[from] ReadError),
	#[error(transparent)]
	Output(#[from] OutputError),
	#[error("cannot write to standard output")]
	WriteListing(#[source] io::Error),
	#[error("{path} is not a live entry of the volume")]
	NotFound { path: String },
	#[error("{path} is a directory")]
	IsDirectory { path: String },
	#[error("no deleted file has the path {path}")]
	DeletedNotFound { path: String },
	#[error(
		"{} deleted files have the path {path} ({}); select one with --cluster and --size",
		candidates.len(),
		describe_candidates(candidates)
	)]
	AmbiguousDeleted { path: String, candidates: Vec<DeletedCandidate> },
	#[error("recover needs the path of a deleted file, or --cluster and --size")]
	MissingRecoveryTarget,
	#[error("cannot read the size of image {path:?}")]
	InspectImage {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("cannot read host file {path:?}")]
	ReadHostFile {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("the volume failed {count} integrity checks; pass --force to change it anyway")]
	PendingWarnings { count: usize },
	#[error("a volume already starts at {offset:#x}; pass --force to format over it")]
	VolumeExists { offset: u64 },
	#[error("the host clock is outside the years 1980 to 2107 that a FATX timestamp holds")]
	ClockOutOfRange,
}

/// A failure of a command on the partitions of a whole device.
#[derive(Debug, Error)]
pub enum DiskToolError {
	#[error(transparent)]
	Detect(#[from] DetectError),
	#[error("the disk metadata failed integrity checks")]
	Integrity(#[from] xenon_io::IntegrityError<DiskWarning>),
	#[error(transparent)]
	Tool(#[from] FatxToolError),
	#[error(transparent)]
	Output(#[from] OutputError),
	#[error("cannot write to standard output")]
	WriteListing(#[source] io::Error),
	#[error("the {device} has no partition {name}; its partitions are {}", available.join(", "))]
	UnknownPartition { name: String, device: DeviceKind, available: Vec<&'static str> },
	#[error("{name} starts at {start:#x}, past the end of the {image_length:#x}-byte image")]
	PartitionPastImage { name: &'static str, start: u64, image_length: u64 },
	#[error("cannot open {path:?}")]
	OpenImage {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("cannot map {path:?}")]
	MapFile {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("the files in {path:?} are not a USB storage layout")]
	UsbStorage {
		path: PathBuf,
		#[source]
		source: UsbStorageError,
	},
	#[error("the {length:#x}-byte image is not a whole number of sectors that a 32-bit count holds")]
	ImageNotWholeSectors { length: u64 },
	#[error("an alignment of {alignment:#x} bytes is not a power-of-two number of sectors")]
	InvalidAlignment { alignment: u64 },
	#[error(transparent)]
	DevkitTable(#[from] DevkitTableError),
	#[error("cannot write the partition table to {path:?}")]
	WriteTable {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
}

fn describe_candidates(candidates: &[DeletedCandidate]) -> String {
	candidates
		.iter()
		.map(|candidate| match candidate.first_cluster {
			Some(cluster) => format!("cluster {cluster} size {}", candidate.size),
			None => format!("no cluster size {}", candidate.size),
		})
		.collect::<Vec<_>>()
		.join(", ")
}
