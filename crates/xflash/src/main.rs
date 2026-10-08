use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;
use memmap2::Mmap;
use rootcause::prelude::ResultExt;
use same_file::Handle;
use thiserror::Error;
use tracing::info_span;
use tracing_subscriber::EnvFilter;
use xflash::Image;
use xflash::ParseError;
use xflash::VisitError;

#[cfg(feature = "dhat")]
#[global_allocator]
static ALLOCATOR: dhat::Alloc = dhat::Alloc;

#[derive(Debug, Parser)]
#[command(about = "Read-only Xbox 360 devkit flash image inspection")]
struct Arguments {
	#[arg(value_name = "IMAGE")]
	image: PathBuf,

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
	Parse(#[from] ParseError),
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
	let image = Image::parse(&map).map_err(CliError::from).into_report()?;

	match arguments.command {
		Command::Info => print_info(&image),
		Command::List => list_root(&image)?,
		Command::ExtractFile { name, output } => extract_file(&image, &file, &arguments.image, &name, output)?,
		Command::ExtractStage { name, output } => extract_stage(&image, &file, &arguments.image, &name, output)?,
	}
	Ok(())
}

fn print_info(image: &Image<'_>) {
	let header = image.header();
	println!(
		"image build={} flags={:#010x} entry={:#010x} boot-size={:#x}",
		header.build.get(),
		header.flags,
		header.entry,
		header.boot_image_size.get(),
	);
	for stage in image.boot_stages() {
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

fn list_root(image: &Image<'_>) -> Result<()> {
	for entry in image.root_entries() {
		let entry = entry.map_err(CliError::from).into_report()?;
		println!(
			"{}\t{:#06x}\t{}\t{:#010x}",
			entry.name(),
			entry.start_cluster().get(),
			entry.size().get(),
			entry.timestamp(),
		);
	}
	Ok(())
}

fn extract_file(image: &Image<'_>, image_file: &File, image_path: &Path, name: &str, output: PathBuf) -> Result<()> {
	let span = info_span!("extract_file", file = name, output = %output.display());
	span.in_scope(|| {
		let entry = image
			.find_root_entry(name)
			.map_err(CliError::from)
			.into_report()?
			.ok_or_else(|| rootcause::Report::new(CliError::FileNotFound { name: name.to_owned() }))?;
		let mut file = create_output(image_file, image_path, &output)?;
		image
			.visit_file(entry, |chunk| {
				file.write_all(chunk).map_err(|source| CliError::WriteOutput { path: output.clone(), source })
			})
			.map_err(|error| match error {
				VisitError::Parse(source) => CliError::Parse(source),
				VisitError::Consumer(source) => source,
			})
			.into_report()?;
		Ok(())
	})
}

fn extract_stage(image: &Image<'_>, image_file: &File, image_path: &Path, name: &str, output: PathBuf) -> Result<()> {
	let stage = image
		.boot_stages()
		.iter()
		.find(|stage| stage.name_str().eq_ignore_ascii_case(name))
		.ok_or_else(|| rootcause::Report::new(CliError::StageNotFound { name: name.to_owned() }))?;
	let mut file = create_output(image_file, image_path, &output)?;
	file.write_all(image.boot_stage_data(stage))
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
