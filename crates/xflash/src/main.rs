use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use anstyle::AnsiColor;
use anstyle::Style;
use clap::Parser;
use clap::Subcommand;
use memmap2::Mmap;
use rootcause::prelude::ResultExt;
use same_file::Handle;
use sfcx::Checked;
use sfcx::FlashImage;
use sfcx::FormatSelection;
use sfcx::IntegrityError;
use sfcx::IntegrityPolicy;
use sfcx::SfcxError;
use sfcx::io::SliceSource;
use thiserror::Error;
use tracing::info_span;
use tracing_subscriber::EnvFilter;

#[cfg(feature = "dhat")]
#[global_allocator]
static ALLOCATOR: dhat::Alloc = dhat::Alloc;

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
	ExtractFile { name: String, output: PathBuf },
	ExtractStage { name: String, output: PathBuf },
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
	#[error("cannot create output {path:?}")]
	CreateOutput {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("cannot identify file {path:?}")]
	IdentifyFile {
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("output {output:?} aliases input image {image:?}")]
	OutputAliasesImage { image: PathBuf, output: PathBuf },
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
}

type Result<T, E = rootcause::Report<CliError>> = std::result::Result<T, E>;

fn main() -> ExitCode {
	tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env()).with_writer(io::stderr).init();

	#[cfg(feature = "dhat")]
	let _profiler = dhat::Profiler::new_heap();

	match run() {
		Ok(()) => ExitCode::SUCCESS,
		Err(error) => {
			eprintln!("xflash: {error:?}");
			ExitCode::FAILURE
		}
	}
}

fn run() -> Result<()> {
	let arguments = Arguments::parse();
	let file = File::open(&arguments.image)
		.map_err(|source| CliError::OpenImage { path: arguments.image.clone(), source })
		.into_report()?;
	let map = unsafe { Mmap::map(&file) }
		.map_err(|source| CliError::MapImage { path: arguments.image.clone(), source })
		.into_report()?;
	let source = SliceSource(&map[..]);
	let policy = if arguments.strict { IntegrityPolicy::Deny } else { IntegrityPolicy::Ignore };
	let image = FlashImage::open(&source, map.len() as u64, arguments.format).map_err(CliError::from).into_report()?;
	let image = resolve(image, policy)?;

	match arguments.command {
		Command::Info => print_info(&image),
		Command::List => list_root(&image, &source, policy)?,
		Command::ExtractFile { name, output } => {
			extract_file(&image, &source, policy, &file, &arguments.image, &name, output)?
		}
		Command::ExtractStage { name, output } => extract_stage(&image, &file, &arguments.image, &name, output)?,
	}
	Ok(())
}

/// Reports the failed integrity checks of `checked` and applies `policy` to them.
fn resolve<T>(checked: Checked<T>, policy: IntegrityPolicy) -> Result<T> {
	const SHOWN: usize = 16;

	let style = Style::new().bold().fg_color(Some(AnsiColor::Yellow.into()));
	let warnings = checked.warnings();
	for warning in warnings.iter().take(SHOWN) {
		anstream::eprintln!("xflash: {style}warning{style:#}: {warning}");
	}
	if warnings.len() > SHOWN {
		anstream::eprintln!("xflash: {style}warning{style:#}: {} more warnings", warnings.len() - SHOWN);
	}

	checked.resolve(policy).map_err(CliError::from).into_report()
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

fn list_root(image: &FlashImage, source: &SliceSource<&[u8]>, policy: IntegrityPolicy) -> Result<()> {
	let filesystem = resolve(image.filesystem(source).map_err(CliError::from).into_report()?, policy)?;
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

fn extract_file(
	image: &FlashImage,
	source: &SliceSource<&[u8]>,
	policy: IntegrityPolicy,
	image_file: &File,
	image_path: &Path,
	name: &str,
	output: PathBuf,
) -> Result<()> {
	let span = info_span!("extract_file", file = name, output = %output.display());
	span.in_scope(|| {
		let filesystem = resolve(image.filesystem(source).map_err(CliError::from).into_report()?, policy)?;
		let entry = filesystem
			.find(name)
			.ok_or_else(|| rootcause::Report::new(CliError::FileNotFound { name: name.to_owned() }))?;
		let data = resolve(image.read_file(source, &filesystem, entry).map_err(CliError::from).into_report()?, policy)?;
		let mut file = create_output(image_file, image_path, &output)?;
		file.write_all(&data).map_err(|source| CliError::WriteOutput { path: output, source }).into_report()?;
		Ok(())
	})
}

fn extract_stage(image: &FlashImage, image_file: &File, image_path: &Path, name: &str, output: PathBuf) -> Result<()> {
	let boot_region = image.boot_region();
	let stage = boot_region
		.find_stage(name)
		.ok_or_else(|| rootcause::Report::new(CliError::StageNotFound { name: name.to_owned() }))?;
	let mut file = create_output(image_file, image_path, &output)?;
	file.write_all(boot_region.boot_stage_data(stage))
		.map_err(|source| CliError::WriteOutput { path: output, source })
		.into_report()?;
	Ok(())
}

fn create_output(image_file: &File, image_path: &Path, output: &Path) -> Result<File> {
	let output_file = OpenOptions::new()
		.write(true)
		.create(true)
		.truncate(false)
		.open(output)
		.map_err(|source| CliError::CreateOutput { path: output.to_owned(), source })
		.into_report()?;
	let image_handle = file_handle(image_file, image_path)?;
	let output_handle = file_handle(&output_file, output)?;
	if image_handle == output_handle {
		return Err(rootcause::Report::new(CliError::OutputAliasesImage {
			image: image_path.to_owned(),
			output: output.to_owned(),
		}));
	}
	output_file
		.set_len(0)
		.map_err(|source| CliError::TruncateOutput { path: output.to_owned(), source })
		.into_report()?;
	Ok(output_file)
}

fn file_handle(file: &File, path: &Path) -> Result<Handle> {
	let file =
		file.try_clone().map_err(|source| CliError::IdentifyFile { path: path.to_owned(), source }).into_report()?;
	Handle::from_file(file).map_err(|source| CliError::IdentifyFile { path: path.to_owned(), source }).into_report()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn output_hard_link_cannot_truncate_image() {
		let directory = std::env::temp_dir().join(format!("xflash-alias-test-{}", std::process::id()));
		let image_path = directory.join("image.bin");
		let output_path = directory.join("output.bin");
		let _ = std::fs::remove_dir_all(&directory);
		std::fs::create_dir(&directory).unwrap();
		std::fs::write(&image_path, b"image-data").unwrap();
		std::fs::hard_link(&image_path, &output_path).unwrap();

		let image_file = File::open(&image_path).unwrap();
		let error = create_output(&image_file, &image_path, &output_path).unwrap_err();

		assert!(matches!(error.current_context(), CliError::OutputAliasesImage { .. }));
		assert_eq!(std::fs::read(&image_path).unwrap(), b"image-data");
		std::fs::remove_dir_all(directory).unwrap();
	}
}
