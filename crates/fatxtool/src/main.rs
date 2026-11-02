use std::fs::File;
use std::io;
use std::io::BufWriter;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;
use fatx::FatxError;
use fatx::IntegrityPolicy;
use fatx::PartitionKind;
use fatx::VolumeOptions;
use fatx::io::SliceSource;
use fatxtool::DiskToolError;
use fatxtool::FatxCommand;
use fatxtool::FatxToolError;
use fatxtool::InputFile;
use fatxtool::OutputError;
use fatxtool::PartitionTableCommand;
use fatxtool::Session;
use fatxtool::VolumeLocation;
use fatxtool::VolumeSource;
use fatxtool::WriteCommand;
use fatxtool::diagnostics::print_error;
use fatxtool::parse_number;
use fatxtool::partition_table;
use fatxtool::partitions;
use fatxtool::select_partition;
use fatxtool::usb_files::open_usb_storage;
use fatxtool::write;
use fatxtool::write::WritableImage;
use memmap2::Mmap;
use rootcause::prelude::ResultExt;
use thiserror::Error;
use xenon_disk::DetectOptions;
use xenon_disk::MemoryDumpSize;

const PROGRAM: &str = "fatxtool";

#[derive(Debug, Parser)]
#[command(about = "Xbox 360 FATX volume inspection, recovery and editing, and disk partition layouts")]
struct Arguments {
	/// A disk, partition, memory unit or volume file image, or a directory of USB storage
	/// Data#### files
	#[arg(value_name = "IMAGE")]
	image: PathBuf,

	/// Offset of the volume in the image
	#[arg(long, value_parser = parse_number, default_value = "0")]
	offset: u64,

	/// Length of the volume; the rest of the image by default
	#[arg(long, value_parser = parse_number, conflicts_with = "partition_length")]
	length: Option<u64>,

	/// The volume of the named partition of the device the image holds, such as Partition1;
	/// `partitions` lists the names
	#[arg(long, value_name = "NAME", conflicts_with_all = ["offset", "length"])]
	partition: Option<String>,

	/// Partition length to mount the volume with, for a volume file the console mounts through a
	/// longer device; the length of the volume's byte range by default
	#[arg(long, value_parser = parse_number)]
	partition_length: Option<u64>,

	/// Mount the volume as a partition in a growable file, which always has a 32-bit FAT and
	/// 16 KiB clusters. A partition selected by name gets this from its layout.
	#[arg(long)]
	alternate_partition: bool,

	/// The devkit has the large memory, which makes its MemDump partition 0x40e30000 bytes
	#[arg(long)]
	large_memory: bool,

	/// Treat failed integrity checks, such as unreadable directories, as errors instead of warnings
	#[arg(long)]
	strict: bool,

	#[command(subcommand)]
	command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
	#[command(flatten)]
	Volume(FatxCommand),
	/// Identify the device the image holds and print its partitions
	Partitions,
	/// Create a devkit partition table on an image file
	#[command(subcommand)]
	PartitionTable(PartitionTableCommand),
	#[command(flatten)]
	Write(WriteCommand),
}

/// A command that reads the image.
#[derive(Debug, Clone, Copy)]
enum SourceCommand<'a> {
	Partitions,
	Volume(&'a FatxCommand),
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
	#[error(transparent)]
	Disk(#[from] DiskToolError),
}

type Result<T, E = rootcause::Report<CliError>> = std::result::Result<T, E>;

fn run() -> Result<()> {
	let arguments = Arguments::parse();
	let memory_dump = detect_options(&arguments).memory_dump;
	let mut out = BufWriter::new(io::stdout().lock());

	let command = match &arguments.command {
		Command::PartitionTable(command) => {
			partition_table::run(command, &arguments.image, memory_dump, &mut out)
				.map_err(CliError::from)
				.into_report()?;
			return flush(&mut out);
		}
		Command::Write(command) => {
			run_write(&arguments, command, &mut out)?;
			return flush(&mut out);
		}
		Command::Partitions => SourceCommand::Partitions,
		Command::Volume(command) => SourceCommand::Volume(command),
	};

	if arguments.image.is_dir() {
		let (source, input) = open_usb_storage(&arguments.image).map_err(CliError::from).into_report()?;
		let image_length = source.layout().logical_length().get();
		run_on(&arguments, command, &source, image_length, &input, &mut out)?;
		return flush(&mut out);
	}

	let file = File::open(&arguments.image)
		.map_err(|source| CliError::OpenImage { path: arguments.image.clone(), source })
		.into_report()?;
	// SAFETY: the image is mapped read-only; it is not expected to change while the command runs.
	let map = unsafe { Mmap::map(&file) }
		.map_err(|source| CliError::MapImage { path: arguments.image.clone(), source })
		.into_report()?;
	let input = InputFile::new(&file, &arguments.image).map_err(CliError::from).into_report()?;
	run_on(&arguments, command, &SliceSource(&map[..]), map.len() as u64, &input, &mut out)?;

	flush(&mut out)
}

fn run_on<S, W>(
	arguments: &Arguments,
	command: SourceCommand<'_>,
	source: &S,
	image_length: u64,
	input: &InputFile,
	out: &mut W,
) -> Result<()>
where
	S: VolumeSource,
	W: Write,
{
	let policy = if arguments.strict { IntegrityPolicy::Deny } else { IntegrityPolicy::Ignore };
	let session = Session::new(PROGRAM, policy, input);

	match command {
		SourceCommand::Partitions => {
			partitions::run(source, image_length, detect_options(arguments), &session, out)
				.map_err(CliError::from)
				.into_report()?;
		}
		SourceCommand::Volume(command) => {
			let location = select(arguments, source, image_length, &session)?;
			fatxtool::run(command, source, &location, &session, out).map_err(CliError::from).into_report()?;
		}
	}

	Ok(())
}

/// Runs a write command on the image file, which is opened for writing only when it is a
/// regular file.
fn run_write<W>(arguments: &Arguments, command: &WriteCommand, out: &mut W) -> Result<()>
where
	W: Write,
{
	let image = WritableImage::open(&arguments.image).map_err(CliError::from).into_report()?;
	let policy = if arguments.strict { IntegrityPolicy::Deny } else { IntegrityPolicy::Ignore };
	let session = Session::new(PROGRAM, policy, image.input());
	let location = select(arguments, image.source(), image.length(), &session)?;

	let result = match command {
		WriteCommand::Format(format) => {
			write::format(format, image.source(), location.range(), location.options(), out)
		}
		command => write::run(command, image.source(), &location, &session, out),
	};
	result.map_err(CliError::from).into_report()?;

	Ok(())
}

/// The volume that `--partition`, or `--offset` and `--length`, select, and the partition length
/// to mount it with: the partition table's, `--partition-length`, or the byte range's.
fn select<S>(arguments: &Arguments, source: &S, image_length: u64, session: &Session<'_>) -> Result<VolumeLocation>
where
	S: VolumeSource,
{
	let requested_kind = arguments.alternate_partition.then_some(PartitionKind::Alternate);

	if let Some(name) = &arguments.partition {
		let selected = select_partition(source, image_length, name, detect_options(arguments), session)
			.map_err(CliError::from)
			.into_report()?;
		let kind = requested_kind.unwrap_or(selected.kind);
		let mut options = VolumeOptions::new().with_kind(kind);
		if let Some(partition_length) = arguments.partition_length.or(selected.truncated_length) {
			options = options.with_partition_length(partition_length);
		}
		if let Some(partition_length) = selected.truncated_length {
			session.note(format_args!(
				"the image ends {:#x} bytes into the {partition_length:#x}-byte {} partition",
				selected.range.end - selected.range.start,
				selected.name
			));
		}

		return Ok(VolumeLocation::new(selected.range, options));
	}

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

	let mut options = VolumeOptions::new().with_kind(requested_kind.unwrap_or(PartitionKind::Standard));
	if let Some(partition_length) = arguments.partition_length {
		options = options.with_partition_length(partition_length);
	}

	Ok(VolumeLocation::new(range, options))
}

fn detect_options(arguments: &Arguments) -> DetectOptions {
	let memory_dump = if arguments.large_memory { MemoryDumpSize::Large } else { MemoryDumpSize::Standard };

	DetectOptions { memory_dump }
}

fn flush<W>(out: &mut W) -> Result<()>
where
	W: Write,
{
	out.flush().map_err(|error| CliError::from(FatxToolError::WriteListing(error))).into_report()?;

	Ok(())
}

fn main() -> ExitCode {
	match run() {
		Ok(()) => ExitCode::SUCCESS,
		Err(error) => {
			print_error(PROGRAM, error.current_context());
			ExitCode::FAILURE
		}
	}
}
