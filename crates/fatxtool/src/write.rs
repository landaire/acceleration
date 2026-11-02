//! Commands that change a volume in an image file: format, mkdir, put, rm and mv. They refuse
//! devices and other files that are not regular files, and refuse to change a volume whose
//! listing has failed integrity checks unless `--force` is given.

use std::fs;
use std::io::Write;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use clap::Args;
use clap::Subcommand;
use fatx::Change;
use fatx::FormatOptions;
use fatx::ReadAt;
use fatx::Request;
use fatx::SerialNumber;
use fatx::Timestamp;
use fatx::Volume;
use fatx::VolumeOptions;
use fatx::WriteAt;
use fatx::WriteMode;
use fatx::format::DEFAULT_CLUSTER_SIZE;
use xenon_io::FileSource;
use xenon_io::IntegrityPolicy;

use crate::error::FatxToolError;
use crate::location::VolumeLocation;
use crate::number::parse_u32;
use crate::output::InputFile;
use crate::output::open_writable_image;
use crate::session::Session;
use crate::source::VolumeSource;

/// A command that writes to the volume.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum WriteCommand {
	/// Create an empty volume over the selected range, as the console formats one
	Format(FormatArguments),
	/// Create a directory
	Mkdir {
		/// Path of the new directory in the volume
		path: String,
		#[command(flatten)]
		force: Force,
	},
	/// Copy a host file into the volume, replacing a file of the same name
	Put {
		/// File to copy
		host_file: PathBuf,
		/// Path of the file in the volume
		path: String,
		#[command(flatten)]
		force: Force,
	},
	/// Delete a file or an empty directory; its record stays recoverable until reused
	Rm {
		/// Path of the entry in the volume
		path: String,
		#[command(flatten)]
		force: Force,
	},
	/// Rename or move a file or directory
	Mv {
		/// Path of the entry in the volume
		from: String,
		/// New path; it must not exist
		to: String,
		#[command(flatten)]
		force: Force,
	},
}

#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct FormatArguments {
	/// Volume serial number; by default the low 32 bits of the current time, as XAM sets it
	#[arg(long, value_parser = parse_u32)]
	pub serial: Option<u32>,
	/// Cluster size in bytes: a power of two from 0x1000 to 0x10000. An alternate partition
	/// needs 0x4000
	#[arg(long, value_parser = parse_u32, default_value_t = DEFAULT_CLUSTER_SIZE)]
	pub cluster_size: u32,
	/// Format over an existing volume
	#[arg(long)]
	pub force: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Args)]
pub struct Force {
	/// Change the volume even though its listing failed integrity checks
	#[arg(long)]
	pub force: bool,
}

/// An image file opened for reading and writing.
#[derive(Debug)]
pub struct WritableImage {
	source: FileSource,
	input: InputFile,
	length: u64,
}

impl WritableImage {
	/// Opens `path`, which must be a regular file reached without a symbolic link.
	pub fn open(path: &Path) -> Result<Self, FatxToolError> {
		let file = open_writable_image(path)?;
		let input = InputFile::new(&file, path)?;
		let length =
			file.metadata().map_err(|source| FatxToolError::InspectImage { path: path.to_owned(), source })?.len();

		Ok(Self { source: FileSource::new(file), input, length })
	}

	pub fn source(&self) -> &FileSource {
		&self.source
	}

	pub fn input(&self) -> &InputFile {
		&self.input
	}

	pub fn length(&self) -> u64 {
		self.length
	}
}

/// Formats the partition at `range` of `source` with the kind and the partition length of
/// `mount`; a partition length longer than the range sizes the FAT for the whole partition,
/// and the range must still hold that FAT and the root directory's cluster. An existing volume
/// is kept unless `--force` is given.
pub fn format<S, W>(
	arguments: &FormatArguments,
	source: &S,
	range: Range<u64>,
	mount: VolumeOptions,
	out: &mut W,
) -> Result<(), FatxToolError>
where
	S: ReadAt + WriteAt,
	W: Write,
{
	let signature_end = range.start + 4;
	if !arguments.force && signature_end <= range.end {
		let signature = ReadAt::read_at(source, &(), range.start..signature_end).map_err(|source| {
			FatxToolError::Read(fatx::ReadError {
				range: range.start..signature_end,
				kind: fatx::ReadErrorKind::Io(source),
			})
		})?;
		if signature.as_ref() == b"XTAF" {
			return Err(FatxToolError::VolumeExists { offset: range.start });
		}
	}

	let serial = match arguments.serial {
		Some(serial) => serial,
		None => system_time_serial()?,
	};
	let mut options =
		FormatOptions::new(SerialNumber::new(serial)).with_cluster_size(arguments.cluster_size).with_kind(mount.kind());
	if let Some(partition_length) = mount.partition_length() {
		options = options.with_partition_length(partition_length);
	}
	let volume = Volume::format(source, range, options)?;

	let geometry = volume.geometry();
	writeln!(
		out,
		"formatted serial {} with {} clusters of {:#x} bytes and a {:?} FAT",
		geometry.serial_number(),
		geometry.cluster_count(),
		geometry.bytes_per_cluster(),
		geometry.fat_width(),
	)
	.map_err(FatxToolError::WriteListing)
}

/// Runs a change other than format on the volume at `location`.
pub fn run<S, W>(
	command: &WriteCommand,
	source: &S,
	location: &VolumeLocation,
	session: &Session<'_>,
	out: &mut W,
) -> Result<(), FatxToolError>
where
	S: VolumeSource + WriteAt,
	W: Write,
{
	let (request_force, host_data) = match command {
		WriteCommand::Format(_) => unreachable!("format runs without a volume"),
		WriteCommand::Mkdir { force, .. } | WriteCommand::Rm { force, .. } | WriteCommand::Mv { force, .. } => {
			(*force, None)
		}
		WriteCommand::Put { host_file, force, .. } => {
			let data = fs::read(host_file)
				.map_err(|source| FatxToolError::ReadHostFile { path: host_file.clone(), source })?;
			(*force, Some(data))
		}
	};

	if let Some(partition_length) = location.extended_partition_length() {
		session.note(format_args!(
			"the {:#x}-byte volume file has a FAT for a {partition_length:#x}-byte partition; new clusters come only \
			 from the file",
			location.image_length(),
		));
	}

	let mut volume = location.open(source)?;
	check_integrity(&volume, source, session, request_force.force)?;

	let now = Timestamp::from_system_time(SystemTime::now()).ok_or(FatxToolError::ClockOutOfRange)?;
	let request = match command {
		WriteCommand::Format(_) => unreachable!("format runs without a volume"),
		WriteCommand::Mkdir { path, .. } => Request::create_directory(path, now),
		WriteCommand::Put { path, .. } => {
			let data = host_data.as_deref().expect("put reads its host file");
			Request::write_file(path, data, WriteMode::Replace, now)
		}
		WriteCommand::Rm { path, .. } => Request::remove(path),
		WriteCommand::Mv { from, to, .. } => Request::rename(from, to),
	}
	.map_err(fatx::FatxError::from)?;

	let change = volume.update(source, request)?;
	session.check_source(source)?;

	describe(&change, out).map_err(FatxToolError::WriteListing)
}

/// Refuses a change while the volume's listing or the source reads have failed integrity
/// checks, unless forced. With `--strict` a failed check is always an error.
fn check_integrity<S>(volume: &Volume, source: &S, session: &Session<'_>, force: bool) -> Result<(), FatxToolError>
where
	S: VolumeSource,
{
	let listing = volume.walk(source)?;
	let source_warnings = source.take_warnings();
	let count = listing.warnings().len() + source_warnings.len();
	session.report(listing.warnings());
	session.report(&source_warnings);
	if count == 0 {
		return Ok(());
	}

	if session.policy() == IntegrityPolicy::Deny {
		listing.deny_warnings()?;
		return Err(FatxToolError::PendingWarnings { count });
	}
	if !force {
		return Err(FatxToolError::PendingWarnings { count });
	}

	session.note(format_args!("changing the volume despite {count} failed integrity checks"));

	Ok(())
}

fn describe<W>(change: &Change, out: &mut W) -> std::io::Result<()>
where
	W: Write,
{
	match change {
		Change::Created { path, record } => writeln!(out, "created {path} ({} bytes)", record.size()),
		Change::Modified { path, record } => writeln!(out, "replaced {path} ({} bytes)", record.size()),
		Change::Removed { path, record } => {
			let first = record.first_cluster().map_or_else(|| "none".to_owned(), |cluster| cluster.to_string());
			writeln!(out, "removed {path} (first cluster {first}, {} bytes)", record.size())
		}
		Change::Renamed { from, to, .. } => writeln!(out, "moved {from} to {to}"),
	}
}

/// The serial number XAM gives a new volume: the low 32 bits of KeQuerySystemTime, 100
/// nanosecond ticks since 1601.
fn system_time_serial() -> Result<u32, FatxToolError> {
	const TICKS_FROM_1601_TO_1970: u128 = 116_444_736_000_000_000;

	let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| FatxToolError::ClockOutOfRange)?;
	let ticks = since_epoch.as_nanos() / 100 + TICKS_FROM_1601_TO_1970;

	Ok(ticks as u32)
}
