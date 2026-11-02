use std::convert::Infallible;
use std::fs::File;
use std::io;
use std::io::BufWriter;
use std::io::Write;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;
use fatxtool::FatxCommand;
use fatxtool::FatxToolError;
use fatxtool::InputFile;
use fatxtool::OutputError;
use fatxtool::Session;
use fatxtool::VolumeLocation;
use fatxtool::VolumeSource;
use fatxtool::create_output;
use fatxtool::session::SHOWN_WARNINGS;
use memmap2::Mmap;
use rootcause::prelude::ResultExt;
use sfcx::Checked;
use sfcx::FlashImage;
use sfcx::FormatSelection;
use sfcx::IntegrityError;
use sfcx::IntegrityPolicy;
use sfcx::MemoryUnit;
use sfcx::MemoryUnitSource;
use sfcx::RawWarning;
use sfcx::ReadAt;
use sfcx::SfcxError;
use sfcx::io::SliceSource;
use sfcx::mu::MU_PAGE_SIZE;
use thiserror::Error;
use tracing::info_span;
use tracing_subscriber::EnvFilter;

#[cfg(feature = "dhat")]
#[global_allocator]
static ALLOCATOR: dhat::Alloc = dhat::Alloc;

const PROGRAM: &str = "xflash";

#[derive(Debug, Parser)]
#[command(about = "Read-only Xbox 360 devkit flash image inspection")]
struct Arguments {
	#[arg(value_name = "IMAGE")]
	image: PathBuf,

	/// Image format
	#[arg(long, value_enum, default_value_t = FormatSelection::Auto)]
	format: FormatSelection,

	/// Treat failed integrity checks, such as ECC mismatches, as errors instead of warnings
	#[arg(long)]
	strict: bool,

	#[command(subcommand)]
	command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
	Info,
	List,
	ExtractFile {
		name: String,
		output: PathBuf,
	},
	ExtractStage {
		name: String,
		output: PathBuf,
	},
	/// Print the size and partitions of the memory unit in a raw image
	MuInfo,
	/// Copy the memory unit of a raw image, or one of its partitions, to a file
	ExtractMu {
		/// File to write
		output: PathBuf,
		/// Copy only this partition
		#[arg(long, value_enum)]
		partition: Option<MuPartition>,
	},
	/// Run a FATX command on a memory unit partition or a flash volume file
	Fatx {
		/// `mu-data`, `mu-system`, or the name of a flash root file such as `xlaunch.fdf`
		#[arg(value_parser = parse_volume)]
		volume: FatxVolume,
		#[command(subcommand)]
		command: FatxCommand,
	},
}

/// A partition device of the memory unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum MuPartition {
	/// Everything before the system partition
	Data,
	/// The last 8 MiB less a page
	System,
}

impl MuPartition {
	fn range(self, unit: &MemoryUnit) -> Range<u64> {
		let partitions = unit.partitions();
		match self {
			Self::Data => partitions.data,
			Self::System => partitions.system,
		}
	}
}

/// A FATX volume of a flash image.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FatxVolume {
	MemoryUnit(MuPartition),
	/// A root file of the flash file system.
	FlashFile(String),
}

/// The memory unit as a FATX volume source; its ECC failures are integrity warnings.
struct MemoryUnitVolume<'a, S>(MemoryUnitSource<'a, S>);

impl<S> ReadAt for MemoryUnitVolume<'_, S>
where
	S: ReadAt,
{
	fn read_at(&self, context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		self.0.read_at(context, range)
	}
}

impl<S> VolumeSource for MemoryUnitVolume<'_, S>
where
	S: ReadAt,
{
	type Warning = RawWarning;

	fn take_warnings(&self) -> Vec<RawWarning> {
		self.0.take_warnings()
	}
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
	#[error(transparent)]
	Flash(#[from] SfcxError),
	#[error(transparent)]
	Integrity(#[from] IntegrityError),
	#[error("root file {name:?} was not found")]
	FileNotFound { name: String },
	#[error("boot stage {name:?} was not found")]
	StageNotFound { name: String },
	#[error(transparent)]
	Output(#[from] OutputError),
	#[error(transparent)]
	Fatx(#[from] fatx::FatxError),
	#[error(transparent)]
	FatxTool(#[from] FatxToolError),
}

type Result<T, E = rootcause::Report<CliError>> = std::result::Result<T, E>;

fn run() -> Result<()> {
	let arguments = Arguments::parse();
	let file = File::open(&arguments.image)
		.map_err(|source| CliError::OpenImage { path: arguments.image.clone(), source })
		.into_report()?;
	let map = unsafe { Mmap::map(&file) }
		.map_err(|source| CliError::MapImage { path: arguments.image.clone(), source })
		.into_report()?;
	let input = InputFile::new(&file, &arguments.image).map_err(CliError::from).into_report()?;
	let policy = if arguments.strict { IntegrityPolicy::Deny } else { IntegrityPolicy::Ignore };
	let session = Session::new(PROGRAM, policy, &input);
	let source = SliceSource(&map[..]);
	let image = FlashImage::open(&source, map.len() as u64, arguments.format).map_err(CliError::from).into_report()?;
	let image = resolve(&session, image)?;

	match arguments.command {
		Command::Info => print_info(&image),
		Command::List => list_root(&image, &source, &session)?,
		Command::ExtractFile { name, output } => extract_file(&image, &source, &session, &name, output)?,
		Command::ExtractStage { name, output } => extract_stage(&image, &session, &name, output)?,
		Command::MuInfo => print_memory_unit(&image, &source, &session)?,
		Command::ExtractMu { output, partition } => extract_memory_unit(&image, &source, &session, partition, &output)?,
		Command::Fatx { volume, command } => run_fatx(&image, &source, &session, &volume, &command)?,
	}
	Ok(())
}

fn parse_volume(text: &str) -> Result<FatxVolume, Infallible> {
	Ok(match text {
		"mu-data" => FatxVolume::MemoryUnit(MuPartition::Data),
		"mu-system" => FatxVolume::MemoryUnit(MuPartition::System),
		name => FatxVolume::FlashFile(name.to_owned()),
	})
}

/// Reports the failed integrity checks of `checked` and applies the session's policy to them.
fn resolve<T>(session: &Session<'_>, checked: Checked<T>) -> Result<T> {
	session.resolve(checked).map_err(CliError::from).into_report()
}

fn print_info(image: &FlashImage) {
	println!("format={}", image.format());
	let boot_region = image.boot_region();
	let header = boot_region.header();
	println!(
		"image build={} flags={:#010x} entry={:#010x} boot-size={:#x}",
		header.build.get(),
		header.flags,
		header.entry,
		header.boot_image_size.get(),
	);
	for stage in boot_region.boot_stages() {
		println!(
			"{} build={} flags={:#010x} entry={:#010x} offset={:#x} size={:#x}",
			stage.name_str(),
			stage.build().get(),
			stage.flags(),
			stage.entry(),
			stage.offset().get(),
			stage.size().get(),
		);
	}
}

fn list_root(image: &FlashImage, source: &SliceSource<&[u8]>, session: &Session<'_>) -> Result<()> {
	let filesystem = resolve(session, image.filesystem(source).map_err(CliError::from).into_report()?)?;
	for entry in filesystem.entries() {
		println!(
			"{}\t{:#06x}\t{}\t{:#010x}",
			entry.name(),
			entry.start_cluster().get(),
			entry.size(),
			entry.timestamp(),
		);
	}
	Ok(())
}

/// The bytes of a flash root file.
fn read_root_file(
	image: &FlashImage,
	source: &SliceSource<&[u8]>,
	session: &Session<'_>,
	name: &str,
) -> Result<Vec<u8>> {
	let filesystem = resolve(session, image.filesystem(source).map_err(CliError::from).into_report()?)?;
	let entry = filesystem
		.find(name)
		.ok_or_else(|| rootcause::Report::new(CliError::FileNotFound { name: name.to_owned() }))?;

	resolve(session, image.read_file(source, &filesystem, entry).map_err(CliError::from).into_report()?)
}

fn extract_file(
	image: &FlashImage,
	source: &SliceSource<&[u8]>,
	session: &Session<'_>,
	name: &str,
	output: PathBuf,
) -> Result<()> {
	let span = info_span!("extract_file", file = name, output = %output.display());
	span.in_scope(|| {
		let data = read_root_file(image, source, session, name)?;
		let mut file = create_output(session.input(), &output).map_err(CliError::from).into_report()?;
		file.write_all(&data)
			.map_err(|source| CliError::from(OutputError::WriteOutput { path: output, source }))
			.into_report()?;
		Ok(())
	})
}

fn extract_stage(image: &FlashImage, session: &Session<'_>, name: &str, output: PathBuf) -> Result<()> {
	let boot_region = image.boot_region();
	let stage = boot_region
		.find_stage(name)
		.ok_or_else(|| rootcause::Report::new(CliError::StageNotFound { name: name.to_owned() }))?;
	let mut file = create_output(session.input(), &output).map_err(CliError::from).into_report()?;
	file.write_all(boot_region.boot_stage_data(stage))
		.map_err(|source| CliError::from(OutputError::WriteOutput { path: output, source }))
		.into_report()?;
	Ok(())
}

fn memory_unit(image: &FlashImage, source: &SliceSource<&[u8]>, session: &Session<'_>) -> Result<MemoryUnit> {
	resolve(session, image.memory_unit(source).map_err(CliError::from).into_report()?)
}

fn print_memory_unit(image: &FlashImage, source: &SliceSource<&[u8]>, session: &Session<'_>) -> Result<()> {
	let unit = memory_unit(image, source, session)?;
	let partitions = unit.partitions();

	println!("size={:#x} pages={:#x} page-size={MU_PAGE_SIZE:#x}", unit.size(), unit.size() / MU_PAGE_SIZE as u64);
	println!("data={:#x}..{:#x}", partitions.data.start, partitions.data.end);
	println!("system={:#x}..{:#x}", partitions.system.start, partitions.system.end);
	Ok(())
}

/// Copies the memory unit, or one partition of it, in bounded reads.
fn extract_memory_unit(
	image: &FlashImage,
	source: &SliceSource<&[u8]>,
	session: &Session<'_>,
	partition: Option<MuPartition>,
	output: &Path,
) -> Result<()> {
	const CHUNK_SIZE: u64 = 0x10_0000;

	let unit = memory_unit(image, source, session)?;
	let range = match partition {
		Some(partition) => partition.range(&unit),
		None => 0..unit.size(),
	};
	let mut file = BufWriter::new(create_output(session.input(), output).map_err(CliError::from).into_report()?);

	// Only the warnings that will be printed are kept; the rest are counted.
	let mut warnings = Vec::new();
	let mut warning_count = 0;
	let mut start = range.start;
	while start < range.end {
		let chunk = start..range.end.min(start + CHUNK_SIZE);
		let (bytes, chunk_warnings) =
			unit.read(source, chunk.clone()).map_err(CliError::from).into_report()?.into_parts();
		warning_count += chunk_warnings.len();
		let room = SHOWN_WARNINGS.saturating_sub(warnings.len());
		warnings.extend(chunk_warnings.into_iter().take(room));
		// Under --strict, stop before writing bytes that failed a check.
		if session.policy() == IntegrityPolicy::Deny && warning_count > 0 {
			session.report_counted(&warnings, warning_count);
			return Err(rootcause::Report::new(CliError::Integrity(IntegrityError { warnings })));
		}

		file.write_all(&bytes)
			.map_err(|source| CliError::from(OutputError::WriteOutput { path: output.to_owned(), source }))
			.into_report()?;
		start = chunk.end;
	}
	file.flush()
		.map_err(|source| CliError::from(OutputError::WriteOutput { path: output.to_owned(), source }))
		.into_report()?;
	session.report_counted(&warnings, warning_count);

	Ok(())
}

fn run_fatx(
	image: &FlashImage,
	source: &SliceSource<&[u8]>,
	session: &Session<'_>,
	volume: &FatxVolume,
	command: &FatxCommand,
) -> Result<()> {
	let mut out = BufWriter::new(io::stdout().lock());
	match volume {
		FatxVolume::MemoryUnit(partition) => {
			let unit = memory_unit(image, source, session)?;
			let volume_source = MemoryUnitVolume(MemoryUnitSource::new(&unit, source));
			let location =
				VolumeLocation::infer(&volume_source, partition.range(&unit)).map_err(CliError::from).into_report()?;
			fatxtool::run(command, &volume_source, &location, session, &mut out)
				.map_err(CliError::from)
				.into_report()?;
		}
		FatxVolume::FlashFile(name) => {
			let data = read_root_file(image, source, session, name)?;
			let length = data.len() as u64;
			let volume_source = SliceSource(data);
			let location = VolumeLocation::infer(&volume_source, 0..length).map_err(CliError::from).into_report()?;
			fatxtool::run(command, &volume_source, &location, session, &mut out)
				.map_err(CliError::from)
				.into_report()?;
		}
	}

	out.flush().map_err(|error| CliError::from(FatxToolError::WriteListing(error))).into_report()
}

fn main() -> ExitCode {
	tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env()).with_writer(io::stderr).init();

	#[cfg(feature = "dhat")]
	let _profiler = dhat::Profiler::new_heap();

	match run() {
		Ok(()) => ExitCode::SUCCESS,
		Err(error) => {
			eprintln!("{PROGRAM}: {error:?}");
			ExitCode::FAILURE
		}
	}
}
