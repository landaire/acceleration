use std::convert::Infallible;
use std::fs::File;
use std::io;
use std::io::BufWriter;
use std::io::Write;
use std::iter;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;
use fatx::VolumeOptions;
use fatxtool::FatxCommand;
use fatxtool::FatxToolError;
use fatxtool::InputFile;
use fatxtool::OutputError;
use fatxtool::Session;
use fatxtool::VolumeLocation;
use fatxtool::VolumeSource;
use fatxtool::create_output;
use fatxtool::diagnostics::print_error;
use fatxtool::parse_number;
use fatxtool::session::SHOWN_WARNINGS;
use memmap2::Mmap;
use rootcause::prelude::ResultExt;
use sfcx::BootChain;
use sfcx::BootStage;
use sfcx::ChainKind;
use sfcx::Checked;
use sfcx::Filesystem;
use sfcx::FlashImage;
use sfcx::FormatSelection;
use sfcx::IntegrityError;
use sfcx::IntegrityPolicy;
use sfcx::MemoryUnit;
use sfcx::MemoryUnitSource;
use sfcx::PatchSlot;
use sfcx::PatchSlotContents;
use sfcx::RawWarning;
use sfcx::ReadAt;
use sfcx::RetailCb;
use sfcx::SfcxError;
use sfcx::StageRole;
use sfcx::io::SliceSource;
use sfcx::mu::MU_PAGE_SIZE;
use thiserror::Error;
use tracing::info_span;
use tracing_subscriber::EnvFilter;

#[cfg(test)]
mod tests;

#[cfg(feature = "dhat")]
#[global_allocator]
static ALLOCATOR: dhat::Alloc = dhat::Alloc;

const PROGRAM: &str = "xflash";

#[derive(Debug, Parser)]
#[command(about = "Read-only Xbox 360 flash image inspection, for devkit and retail consoles")]
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
	/// Copy a boot stage as stored. A split retail CB is CB_A and CB_B.
	ExtractStage {
		/// SB, SC, SD, SE, CB, CB_A, CB_B, CC, CD or CE
		#[arg(value_parser = parse_stage_role)]
		stage: StageRole,
		output: PathBuf,
	},
	/// Copy the SMC firmware of the boot region to a file, decrypted unless `--encrypted`
	ExtractSmc {
		/// File to write
		output: PathBuf,
		/// Copy the bytes as stored in flash
		#[arg(long)]
		encrypted: bool,
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
		/// Partition length to mount the volume with. A memory unit partition has its device's
		/// length; a flash root file has the length the console mounts it with, when xflash knows
		/// it, and otherwise its file length.
		#[arg(long, value_parser = parse_number)]
		partition_length: Option<u64>,
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

/// A flash root file that the console mounts as a FATX partition through a device whose length
/// is not the file's.
#[derive(Debug, Clone, Copy)]
struct KnownMount {
	name: &'static str,
	partition_length: u64,
}

/// The partition lengths of known flash volume mounts. FatxProcessBootSector (mountvol.c:8-140)
/// sizes the FAT from the length IOCTL_DISK_GET_PARTITION_INFO reports for the mounted device.
///
/// xlaunch.fdf, flash xshell.xex and XBDM 2.0.21256: xshell sub_92058190 calls xbdm.xex ordinal
/// 130 (0x91f549f8) with "\SystemRoot\xlaunch.fdf", "\Device\LauncherData" and 0x1000000. The
/// ordinal's worker (sub_91f548b0, then sub_91f54718) creates a device of type 0x3d whose
/// extension holds start offset 0 at +0x10 and length 0x1000000 at +0x18, and its device control
/// routine (0x91f544f0) answers IOCTL_DISK_GET_PARTITION_INFO (0x74004) with those two values.
/// Another build may pass another length to the same ordinal.
const KNOWN_MOUNTS: [KnownMount; 1] = [KnownMount { name: "xlaunch.fdf", partition_length: 0x0100_0000 }];

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
	#[error("the {kind} boot chain has no boot stage {stage}")]
	StageNotFound { stage: StageRole, kind: ChainKind },
	#[error("the CB of this image is split; name \"CB_A\" or \"CB_B\"")]
	SplitCb,
	#[error("the boot region has no SMC firmware")]
	SmcNotFound,
	#[error("the decrypted SMC firmware does not end in a zero word, which the console requires")]
	SmcTrailer,
	#[error(transparent)]
	Output(#[from] OutputError),
	#[error(transparent)]
	Fatx(#[from] fatx::FatxError),
	#[error(transparent)]
	FatxTool(#[from] FatxToolError),
}

/// A boot stage name that names no stage role.
#[derive(Debug, Error)]
#[error("unknown boot stage {name:?}; expected SB, SC, SD, SE, CB, CB_A, CB_B, CC, CD or CE")]
struct UnknownStage {
	name: String,
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
		Command::Info => print_info(&image, &source, &session)?,
		Command::List => list_root(&image, &source, &session)?,
		Command::ExtractFile { name, output } => extract_file(&image, &source, &session, &name, output)?,
		Command::ExtractStage { stage, output } => extract_stage(&image, &session, stage, output)?,
		Command::ExtractSmc { output, encrypted } => extract_smc(&image, &session, output, encrypted)?,
		Command::MuInfo => print_memory_unit(&image, &source, &session)?,
		Command::ExtractMu { output, partition } => extract_memory_unit(&image, &source, &session, partition, &output)?,
		Command::Fatx { volume, partition_length, command } => {
			run_fatx(&image, &source, &session, &volume, partition_length, &command)?
		}
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

/// Prints the image format and kind, the flash header, the boot chain, the SMC firmware and the
/// kernel patch slots.
fn print_info(image: &FlashImage, source: &SliceSource<&[u8]>, session: &Session<'_>) -> Result<()> {
	let patch_slots = match image.patch_slots(source) {
		Ok(checked) => Ok(resolve(session, checked)?),
		Err(error) => Err(error),
	};
	let mut out = io::stdout().lock();
	write_info(&mut out, image, &patch_slots)
		.map_err(|error| CliError::from(FatxToolError::WriteListing(error)))
		.into_report()
}

fn write_info<W>(out: &mut W, image: &FlashImage, patch_slots: &Result<Vec<PatchSlot>, SfcxError>) -> io::Result<()>
where
	W: Write,
{
	let boot_region = image.boot_region();
	let header = boot_region.header();
	writeln!(out, "format={}", image.format())?;
	writeln!(out, "kind={}", boot_region.chain().kind())?;
	writeln!(
		out,
		"image build={} qfe={:#06x} flags={:#06x} entry={:#010x} boot-size={:#x}",
		header.build.get(),
		header.qfe,
		header.flags,
		header.entry,
		header.boot_image_size.get(),
	)?;
	for (role, stage) in boot_region.chain().stages() {
		writeln!(
			out,
			"{} build={} qfe={:#06x} flags={:#06x} entry={:#010x} offset={:#x} size={:#x}",
			role.label(),
			stage.build().get(),
			stage.qfe(),
			stage.flags(),
			stage.entry(),
			stage.offset().get(),
			stage.size().get(),
		)?;
	}
	match boot_region.smc() {
		Ok(Some(smc)) => writeln!(out, "SMC offset={:#x} size={:#x}", smc.offset().get(), smc.size().get())?,
		Ok(None) => writeln!(out, "SMC none")?,
		Err(error) => writeln!(out, "SMC invalid: {error}")?,
	}
	match boot_region.key_vault() {
		Ok(Some(key_vault)) => writeln!(out, "key-vault offset={:#x} size={:#x}", key_vault.start, key_vault.len())?,
		Ok(None) => writeln!(out, "key-vault none")?,
		Err(error) => writeln!(out, "key-vault invalid: {error}")?,
	}
	let slots = match patch_slots {
		Ok(slots) if slots.is_empty() => return writeln!(out, "patch-slots none"),
		Ok(slots) => slots,
		Err(error) => return writeln!(out, "patch-slots invalid: {error}"),
	};
	for (index, slot) in slots.iter().enumerate() {
		write!(out, "patch-slot {index} offset={:#x} size={:#x}", slot.offset().get(), slot.size().get())?;
		match slot.contents() {
			PatchSlotContents::Empty => writeln!(out, " empty")?,
			PatchSlotContents::Loaders { cf, cg } => {
				for stage in iter::once(cf).chain(cg) {
					write!(
						out,
						" {} build={} offset={:#x} size={:#x}",
						stage.name(),
						stage.build().get(),
						stage.offset().get(),
						stage.size().get()
					)?;
				}
				writeln!(out)?;
			}
		}
	}

	Ok(())
}

fn list_root(image: &FlashImage, source: &SliceSource<&[u8]>, session: &Session<'_>) -> Result<()> {
	let filesystem = resolve(session, image.filesystem(source).map_err(CliError::from).into_report()?)?;
	let mut out = io::stdout().lock();
	write_listing(&mut out, &filesystem)
		.map_err(|error| CliError::from(FatxToolError::WriteListing(error)))
		.into_report()
}

fn write_listing<W>(out: &mut W, filesystem: &Filesystem) -> io::Result<()>
where
	W: Write,
{
	for entry in filesystem.entries() {
		writeln!(
			out,
			"{}\t{:#06x}\t{}\t{:#010x}",
			entry.name(),
			entry.start_cluster().get(),
			entry.size(),
			entry.timestamp(),
		)?;
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

fn extract_stage(image: &FlashImage, session: &Session<'_>, role: StageRole, output: PathBuf) -> Result<()> {
	let boot_region = image.boot_region();
	let stage = find_stage(boot_region.chain(), role).into_report()?;
	let mut file = create_output(session.input(), &output).map_err(CliError::from).into_report()?;
	file.write_all(boot_region.boot_stage_data(&stage))
		.map_err(|source| CliError::from(OutputError::WriteOutput { path: output, source }))
		.into_report()?;
	Ok(())
}

/// The stage of `chain` with `role`. `CB` names the CB of a chain that does not split it.
fn find_stage(chain: &BootChain, role: StageRole) -> Result<BootStage, CliError> {
	if let Some(stage) = chain.stage(role) {
		return Ok(stage);
	}

	match (role, chain) {
		(StageRole::Cb, BootChain::Retail { cb: RetailCb::Split { .. }, .. }) => Err(CliError::SplitCb),
		_ => Err(CliError::StageNotFound { stage: role, kind: chain.kind() }),
	}
}

fn parse_stage_role(text: &str) -> Result<StageRole, UnknownStage> {
	StageRole::parse(text).ok_or_else(|| UnknownStage { name: text.to_owned() })
}

fn extract_smc(image: &FlashImage, session: &Session<'_>, output: PathBuf, encrypted: bool) -> Result<()> {
	let boot_region = image.boot_region();
	let smc = boot_region
		.smc()
		.map_err(|error| CliError::from(SfcxError::from(error)))
		.into_report()?
		.ok_or_else(|| rootcause::Report::new(CliError::SmcNotFound))?;
	let mut firmware = boot_region.smc_data(&smc).to_vec();
	if !encrypted {
		sfcx::smc::decrypt(&mut firmware);
		if !sfcx::smc::has_zero_trailer(&firmware) {
			return Err(rootcause::Report::new(CliError::SmcTrailer));
		}
	}
	let mut file = create_output(session.input(), &output).map_err(CliError::from).into_report()?;
	file.write_all(&firmware)
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
	partition_length: Option<u64>,
	command: &FatxCommand,
) -> Result<()> {
	let mut out = BufWriter::new(io::stdout().lock());
	match volume {
		FatxVolume::MemoryUnit(partition) => {
			let unit = memory_unit(image, source, session)?;
			let volume_source = MemoryUnitVolume(MemoryUnitSource::new(&unit, source));
			// mu.c reports each partition device's own length to the FATX driver.
			let options = match partition_length {
				Some(partition_length) => VolumeOptions::new().with_partition_length(partition_length),
				None => VolumeOptions::new(),
			};
			let location = VolumeLocation::new(partition.range(&unit), options);
			fatxtool::run(command, &volume_source, &location, session, &mut out)
				.map_err(CliError::from)
				.into_report()?;
		}
		FatxVolume::FlashFile(name) => {
			let data = read_root_file(image, source, session, name)?;
			let file_length = data.len() as u64;
			let volume_source = SliceSource(data);
			let known = KNOWN_MOUNTS.iter().find(|mount| mount.name.eq_ignore_ascii_case(name));
			// Other root files mount as FATX files do: with their own length.
			let options = match partition_length.or(known.map(|mount| mount.partition_length)) {
				Some(partition_length) => VolumeOptions::new().with_partition_length(partition_length),
				None => VolumeOptions::new(),
			};
			let location = VolumeLocation::new(0..file_length, options);
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
			print_error(PROGRAM, error.current_context());
			ExitCode::FAILURE
		}
	}
}
