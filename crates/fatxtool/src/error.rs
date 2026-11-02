use std::error::Error;
use std::io;
use std::num::ParseIntError;
use std::path::PathBuf;

use fatx::ClusterNumber;
use fatx::FatxError;
use fatx::IntegrityError;
use fatx::ReadError;
use thiserror::Error;

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
