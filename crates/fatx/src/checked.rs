use thiserror::Error;

use crate::entry::EntryLocation;
use crate::entry::InvalidEntryReason;
use crate::error::ChainError;
use crate::error::DirectoryError;
use crate::fat::ClusterNumber;
use crate::fat::ClusterRun;

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
	/// Clusters the FAT allocates that no live file or directory chain reaches: chkdsk's lost
	/// clusters. An interrupted update leaves them, and so does a volume opened with a partition
	/// length other than the one it was formatted for. `runs` are the first of them.
	#[error("{count} allocated clusters are reached by no live file or directory, from {}", describe_runs(.runs))]
	LostClusters { count: u64, runs: Vec<ClusterRun> },
}

/// A value read from a volume and the integrity checks it failed.
pub type Checked<T> = xenon_io::Checked<T, Warning>;

pub type IntegrityError = xenon_io::IntegrityError<Warning>;

fn describe_runs(runs: &[ClusterRun]) -> String {
	runs.iter()
		.map(|run| match run.count() {
			1 => run.first().to_string(),
			count => format!("{}..={:#x}", run.first(), u64::from(run.first().get()) + u64::from(count) - 1),
		})
		.collect::<Vec<_>>()
		.join(", ")
}
