use std::io::Write;
use std::path::PathBuf;

use clap::Subcommand;
use clap::ValueEnum;
use fatx::ClusterNumber;
use fatx::Listing;
use fatx::Volume;
use fatx::forensic::recover::RecoveryStrategy;

use crate::error::FatxToolError;
use crate::extract;
use crate::info;
use crate::list;
use crate::location::VolumeLocation;
use crate::number::parse_cluster;
use crate::number::parse_u32;
use crate::recover;
use crate::recover::RecoverTarget;
use crate::scan;
use crate::session::Session;
use crate::source::VolumeSource;

/// A command on one FATX volume.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum FatxCommand {
	/// List the directory tree
	List {
		/// Also list deleted files and directories
		#[arg(long)]
		deleted: bool,
		/// Print the size, first cluster and timestamps of each entry
		#[arg(long)]
		long: bool,
	},
	/// Copy a live file out of the volume
	Extract {
		/// Path of the file in the volume
		path: String,
		/// File to write
		output: PathBuf,
	},
	/// Copy every live file and directory out of the volume
	ExtractAll {
		/// Directory to write the tree into
		directory: PathBuf,
	},
	/// Search free clusters for orphaned directory records and known file signatures, and
	/// assess every deleted file
	Scan,
	/// Recover the data of a deleted file, from its path or from a first cluster and a size
	#[command(allow_missing_positional = true)]
	Recover {
		/// Path of the deleted file
		#[arg(required_unless_present = "cluster", conflicts_with = "cluster")]
		path: Option<String>,
		/// File to write
		output: PathBuf,
		/// First cluster of the data
		#[arg(long, value_parser = parse_cluster, requires = "size")]
		cluster: Option<ClusterNumber>,
		/// Size of the data in bytes
		#[arg(long, value_parser = parse_u32, requires = "cluster")]
		size: Option<u32>,
		#[arg(long, value_enum, default_value_t = StrategyArgument::Contiguous)]
		strategy: StrategyArgument,
	},
	/// Print the volume header, geometry and allocation
	Info,
}

/// How to recover data whose cluster chain the deletion freed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StrategyArgument {
	/// Read the clusters after the first cluster, whether or not live files hold them now
	Contiguous,
	/// Read only those clusters the FAT marks free
	ContiguousFree,
}

impl StrategyArgument {
	pub const fn strategy(self) -> RecoveryStrategy {
		match self {
			Self::Contiguous => RecoveryStrategy::Contiguous,
			Self::ContiguousFree => RecoveryStrategy::ContiguousFree,
		}
	}
}

/// Runs `command` on the volume at `location` of `source`. Listings and reports go to `out`;
/// warnings and notes go to standard error.
pub fn run<S, W>(
	command: &FatxCommand,
	source: &S,
	location: &VolumeLocation,
	session: &Session<'_>,
	out: &mut W,
) -> Result<(), FatxToolError>
where
	S: VolumeSource,
	W: Write,
{
	if let Some(partition_length) = location.extended_partition_length() {
		session.note(format_args!(
			"the {:#x}-byte volume file is mounted as a {partition_length:#x}-byte partition",
			location.image_length(),
		));
	}

	let volume = location.open(source)?;
	session.check_source(source)?;

	match command {
		FatxCommand::List { deleted, long } => {
			list::run(&volume, source, session, list::ListOptions { deleted: *deleted, long: *long }, out)
		}
		FatxCommand::Extract { path, output } => extract::extract(&volume, source, session, path, output),
		FatxCommand::ExtractAll { directory } => extract::extract_all(&volume, source, session, directory, out),
		FatxCommand::Scan => scan::run(&volume, source, session, out),
		FatxCommand::Recover { path, output, cluster, size, strategy } => {
			let target = match (path, cluster, size) {
				(Some(path), None, None) => RecoverTarget::Entry(path),
				(None, Some(first), Some(size)) => RecoverTarget::Clusters { first: *first, size: *size },
				_ => return Err(FatxToolError::MissingRecoveryTarget),
			};
			recover::run(&volume, source, session, target, strategy.strategy(), output, out)
		}
		FatxCommand::Info => info::run(&volume, out),
	}
}

/// `path` as an absolute volume path without a trailing separator.
pub(crate) fn normalize_path(path: &str) -> String {
	let trimmed = path.trim_matches('/');

	format!("/{trimmed}")
}

/// The directory tree, with the failed integrity checks of the walk and the source resolved.
pub(crate) fn walk<S>(volume: &Volume, source: &S, session: &Session<'_>) -> Result<Listing, FatxToolError>
where
	S: VolumeSource,
{
	let listing = volume.walk(source)?;
	session.check_source(source)?;

	Ok(session.resolve(listing)?)
}

/// Whether two volume paths name the same entry. The driver compares names without regard to
/// ASCII case.
pub(crate) fn same_path(left: &str, right: &str) -> bool {
	left.eq_ignore_ascii_case(right)
}
