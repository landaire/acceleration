use thiserror::Error;

use crate::entry::EntryLocation;
use crate::entry::InvalidEntryReason;
use crate::error::ChainError;
use crate::error::DirectoryError;
use crate::fat::ClusterNumber;

pub use xenon_io::IntegrityPolicy;

/// A failed integrity check. The read that found it continued the way the FATX driver would.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum Warning {
	#[error("record at {location} skipped: {reason}")]
	InvalidEntry { location: EntryLocation, reason: InvalidEntryReason },
	#[error("directory {path} is unreadable: {error}")]
	UnreadableDirectory { path: String, error: DirectoryError },
	#[error("directory {path} at cluster {cluster} was already listed")]
	DirectoryLoop { path: String, cluster: ClusterNumber },
	#[error("file {path} has a broken cluster chain: {error}")]
	BrokenChain { path: String, error: ChainError },
	#[error("cluster {cluster} of {path} also belongs to another file or directory")]
	CrossLinkedCluster { path: String, cluster: ClusterNumber },
}

/// A value read from a volume and the integrity checks it failed.
pub type Checked<T> = xenon_io::Checked<T, Warning>;

pub type IntegrityError = xenon_io::IntegrityError<Warning>;
