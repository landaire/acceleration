use std::fs::File;
use std::io;
use std::io::BufWriter;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use fatx::FatxError;
use fatx::IntegrityPolicy;
use fatx::PartitionKind;
use fatx::VolumeOptions;
use fatx::io::SliceSource;
use fatxtool::FatxCommand;
use fatxtool::FatxToolError;
use fatxtool::InputFile;
use fatxtool::OutputError;
use fatxtool::Session;
use fatxtool::VolumeLocation;
use fatxtool::parse_number;
use memmap2::Mmap;
use rootcause::prelude::ResultExt;
use thiserror::Error;

const PROGRAM: &str = "fatxtool";

#[derive(Debug, Parser)]
#[command(about = "Read-only Xbox 360 FATX volume inspection and recovery")]
struct Arguments {
	/// A partition, memory unit or volume file image
	#[arg(value_name = "IMAGE")]
	image: PathBuf,

	/// Offset of the volume in the image
	#[arg(long, value_parser = parse_number, default_value = "0")]
	offset: u64,

	/// Length of the volume; the rest of the image by default
	#[arg(long, value_parser = parse_number, conflicts_with = "partition_length")]
	length: Option<u64>,

	/// Partition length to mount the volume with, for a volume file shorter than its partition;
	/// inferred from the FAT by default
	#[arg(long, value_parser = parse_number)]
	partition_length: Option<u64>,

	/// Mount the volume as a partition in a growable file, which always has a 32-bit FAT and
	/// 16 KiB clusters
	#[arg(long)]
	alternate_partition: bool,

	/// Treat failed integrity checks, such as unreadable directories, as errors instead of warnings
	#[arg(long)]
	strict: bool,

	#[command(subcommand)]
	command: FatxCommand,
}

#[derive(Debug, Error)]
enum CliError {
	#[error("cannot open image {path:?}")]
	OpenImage {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("cannot map image {path:?}")]
	MapImage {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("the volume at offset {offset:#x} with length {length:#x?} is not inside the {image_length:#x}-byte image")]
	RangeOutsideImage { offset: u64, length: Option<u64>, image_length: u64 },
	#[error(transparent)]
	Fatx(#[from] FatxError),
	#[error(transparent)]
	Output(#[from] OutputError),
	#[error(transparent)]
	Tool(#[from] FatxToolError),
}

type Result<T, E = rootcause::Report<CliError>> = std::result::Result<T, E>;

fn run() -> Result<()> {
	let arguments = Arguments::parse();
	let file = File::open(&arguments.image)
		.map_err(|source| CliError::OpenImage { path: arguments.image.clone(), source })
		.into_report()?;
	// SAFETY: the image is mapped read-only; it is not expected to change while the command runs.
	let map = unsafe { Mmap::map(&file) }
		.map_err(|source| CliError::MapImage { path: arguments.image.clone(), source })
		.into_report()?;
	let input = InputFile::new(&file, &arguments.image).map_err(CliError::from).into_report()?;
	let policy = if arguments.strict { IntegrityPolicy::Deny } else { IntegrityPolicy::Ignore };
	let session = Session::new(PROGRAM, policy, &input);

	let image_length = map.len() as u64;
	let end = match arguments.length {
		Some(length) => arguments.offset.checked_add(length),
		None => Some(image_length),
	};
	let range = match end {
		Some(end) if arguments.offset <= end && end <= image_length => arguments.offset..end,
		_ => {
			return Err(rootcause::Report::new(CliError::RangeOutsideImage {
				offset: arguments.offset,
				length: arguments.length,
				image_length,
			}));
		}
	};

	let source = SliceSource(&map[..]);
	let kind = if arguments.alternate_partition { PartitionKind::Alternate } else { PartitionKind::Standard };
	let location = match arguments.partition_length {
		Some(partition_length) => {
			VolumeLocation::new(range, VolumeOptions::new().with_kind(kind).with_partition_length(partition_length))
		}
		None => VolumeLocation::infer_with(&source, range, kind).map_err(CliError::from).into_report()?,
	};

	let mut out = BufWriter::new(io::stdout().lock());
	fatxtool::run(&arguments.command, &source, &location, &session, &mut out).map_err(CliError::from).into_report()?;
	out.flush().map_err(|error| CliError::from(FatxToolError::WriteListing(error))).into_report()?;

	Ok(())
}

fn main() -> ExitCode {
	match run() {
		Ok(()) => ExitCode::SUCCESS,
		Err(error) => {
			eprintln!("{PROGRAM}: {error:?}");
			ExitCode::FAILURE
		}
	}
}
