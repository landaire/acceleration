use std::fmt;
use std::fs;
use std::fs::File;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use anstyle::AnsiColor;
use anstyle::Style;
use clap::Parser;
use memmap2::Mmap;
use memmap2::MmapMut;
use same_file::Handle;
use sfcx::Checked;
use sfcx::FlashImage;
use sfcx::FormatSelection;
use sfcx::IntegrityPolicy;
use sfcx::io::SliceSource;
use shadowboot::BootLayer;
use tempfile::NamedTempFile;
use thiserror::Error;

#[cfg(feature = "dhat")]
#[global_allocator]
static ALLOCATOR: dhat::Alloc = dhat::Alloc;

#[derive(Debug, Parser)]
#[command(about = "Compile an Xbox 360 devkit shadowboot boot-region image")]
struct Arguments {
	/// Read-only flash image or boot region used as the base
	#[arg(long, value_name = "IMAGE")]
	base: PathBuf,

	/// Format of the base image
	#[arg(long, value_enum, default_value_t = FormatSelection::Auto)]
	format: FormatSelection,

	/// Treat failed integrity checks, such as ECC mismatches and unverified SD signatures, as
	/// errors instead of warnings
	#[arg(long)]
	strict: bool,

	/// Layer directory containing boot/ inputs
	#[arg(long, value_name = "DIRECTORY", required_unless_present = "extract")]
	layer: Option<PathBuf>,

	/// Shadowboot image to create or replace atomically
	#[arg(long, value_name = "IMAGE", required_unless_present = "extract")]
	output: Option<PathBuf>,

	/// Recompress SE and update its SD digest even when the payload is unchanged
	#[arg(long, conflicts_with = "extract")]
	rebuild_se: bool,
	/// Instead of compiling, write the hypervisor and kernel of the base SE as
	/// DIRECTORY/boot/hypervisor.bin and DIRECTORY/boot/xboxkrnl.exe, a layer
	#[arg(long, value_name = "DIRECTORY", conflicts_with_all = ["layer", "output"])]
	extract: Option<PathBuf>,
}

#[derive(Debug, Error)]
enum CliError {
	#[error("cannot {operation} {path:?}")]
	Io {
		operation: &'static str,
		path: PathBuf,
		#[source]
		source: io::Error,
	},
	#[error("layer entry {path:?} is not a regular file or supported directory")]
	UnsupportedLayerEntry { path: PathBuf },
	#[error("layer entry name at {path:?} is not UTF-8")]
	NonUtf8LayerName { path: PathBuf },
	#[error("unsupported boot-layer file {path:?}")]
	UnknownBootInput { path: PathBuf },
	#[error("layer input {path:?} is empty")]
	EmptyInput { path: PathBuf },
	#[error("output {output:?} aliases base image {base:?}")]
	OutputAliasesBase { base: PathBuf, output: PathBuf },
	#[error("output {output:?} aliases layer input {input:?}")]
	OutputAliasesLayerInput { input: PathBuf, output: PathBuf },
	#[error("output path {path:?} has no parent directory")]
	MissingOutputParent { path: PathBuf },
	#[error(transparent)]
	Flash(#[from] sfcx::SfcxError),
	#[error(transparent)]
	FlashIntegrity(#[from] sfcx::IntegrityError),
	#[error("{count} integrity checks failed")]
	Integrity { count: usize },
	#[error(transparent)]
	Compile(#[from] shadowboot::Error),
}

type Result<T> = std::result::Result<T, CliError>;

struct InputIdentity {
	path: PathBuf,
	handle: Handle,
}

#[derive(Default)]
struct MappedBoot {
	sb: Option<Mmap>,
	sc: Option<Mmap>,
	sd: Option<Mmap>,
	se: Option<Mmap>,
	hypervisor: Option<Mmap>,
	kernel: Option<Mmap>,
}

struct MappedLayer {
	boot: MappedBoot,
	inputs: Vec<InputIdentity>,
}

fn main() -> ExitCode {
	#[cfg(feature = "dhat")]
	let _profiler = dhat::Profiler::new_heap();

	match run(Arguments::parse()) {
		Ok(()) => ExitCode::SUCCESS,
		Err(error) => {
			eprintln!("shadowboot: {error}");
			let mut source = std::error::Error::source(&error);
			while let Some(error) = source {
				eprintln!("  caused by: {error}");
				source = error.source();
			}
			ExitCode::FAILURE
		}
	}
}

fn run(arguments: Arguments) -> Result<()> {
	let base_file = open_file(&arguments.base, "open base image")?;
	let base = map_file(&base_file, &arguments.base)?;
	let (layer_path, output_path) = match (arguments.layer, arguments.output, arguments.extract) {
		(_, _, Some(directory)) => return extract(&base, arguments.format, policy(arguments.strict), &directory),
		(Some(layer), Some(output), None) => (layer, output),
		_ => unreachable!("clap requires --layer and --output without --extract"),
	};
	let arguments = CompileArguments {
		base: arguments.base,
		format: arguments.format,
		strict: arguments.strict,
		layer: layer_path,
		output: output_path,
		rebuild_se: arguments.rebuild_se,
	};
	let mapped_layer = load_layer(&arguments.layer)?;
	check_output_aliases(&base_file, &arguments.base, &arguments.output, &mapped_layer.inputs)?;

	let base = load_base(&base, arguments.format, policy(arguments.strict))?;
	let region = base.boot_region();
	let output_size = shadowboot::output_size(&region)?;
	let layer = BootLayer {
		sb: mapped_layer.boot.sb.as_deref(),
		sc: mapped_layer.boot.sc.as_deref(),
		sd: mapped_layer.boot.sd.as_deref(),
		se: mapped_layer.boot.se.as_deref(),
		hypervisor: mapped_layer.boot.hypervisor.as_deref(),
		kernel: mapped_layer.boot.kernel.as_deref(),
		rebuild_se: arguments.rebuild_se,
	};

	// A bare file name has an empty parent: the current directory.
	let parent = match arguments.output.parent() {
		Some(parent) if parent.as_os_str().is_empty() => Path::new("."),
		Some(parent) => parent,
		None => return Err(CliError::MissingOutputParent { path: arguments.output.clone() }),
	};
	let temporary = NamedTempFile::new_in(parent)
		.map_err(|source| io_error("create temporary output", &arguments.output, source))?;
	temporary
		.as_file()
		.set_len(output_size as u64)
		.map_err(|source| io_error("size temporary output", &arguments.output, source))?;
	let mut output = unsafe { MmapMut::map_mut(temporary.as_file()) }
		.map_err(|source| io_error("map temporary output", &arguments.output, source))?;
	let report = shadowboot::compile(&region, &mut output, &layer)?;
	output.flush().map_err(|source| io_error("flush temporary output", &arguments.output, source))?;
	drop(output);
	print_warnings(&report.warnings);
	reject_warnings(&report.warnings, policy(arguments.strict))?;
	temporary.persist(&arguments.output).map_err(|error| io_error("replace output", &arguments.output, error.error))?;

	println!(
		"wrote {} ({:#x} bytes): boot replaced={} se-rebuilt={} sd-signed={}",
		arguments.output.display(),
		output_size,
		report.replaced_boot_stages,
		report.rebuilt_se,
		report.updated_sd_signature,
	);
	Ok(())
}

fn policy(strict: bool) -> IntegrityPolicy {
	if strict { IntegrityPolicy::Deny } else { IntegrityPolicy::Ignore }
}

fn load_base(base: &[u8], format: FormatSelection, policy: IntegrityPolicy) -> Result<FlashImage> {
	let image: Checked<FlashImage> = FlashImage::open(&SliceSource(base), base.len() as u64, format)?;
	print_warnings(image.warnings());

	Ok(image.resolve(policy)?)
}

/// Fails on the shadowboot warnings when the policy denies failed integrity checks.
fn reject_warnings(warnings: &[shadowboot::Warning], policy: IntegrityPolicy) -> Result<()> {
	match policy {
		IntegrityPolicy::Deny if !warnings.is_empty() => Err(CliError::Integrity { count: warnings.len() }),
		IntegrityPolicy::Deny | IntegrityPolicy::Ignore => Ok(()),
	}
}

fn print_warnings<W>(warnings: &[W])
where
	W: fmt::Display,
{
	const SHOWN: usize = 16;

	let style = Style::new().bold().fg_color(Some(AnsiColor::Yellow.into()));
	for warning in warnings.iter().take(SHOWN) {
		anstream::eprintln!("shadowboot: {style}warning{style:#}: {warning}");
	}
	if warnings.len() > SHOWN {
		anstream::eprintln!("shadowboot: {style}warning{style:#}: {} more warnings", warnings.len() - SHOWN);
	}
}

struct CompileArguments {
	base: PathBuf,
	format: FormatSelection,
	strict: bool,
	layer: PathBuf,
	output: PathBuf,
	rebuild_se: bool,
}

/// Writes the hypervisor and kernel of the base SE as a layer's boot inputs.
fn extract(base: &[u8], format: FormatSelection, policy: IntegrityPolicy, directory: &Path) -> Result<()> {
	let base = load_base(base, format, policy)?;
	let extracted = shadowboot::payload(&base.boot_region())?;
	print_warnings(&extracted.warnings);
	reject_warnings(&extracted.warnings, policy)?;
	let boot = directory.join("boot");
	fs::create_dir_all(&boot).map_err(|source| io_error("create directory", &boot, source))?;
	let payload = &extracted.payload;
	for (name, bytes) in [("hypervisor.bin", payload.hypervisor()), ("xboxkrnl.exe", payload.kernel())] {
		let path = boot.join(name);
		fs::write(&path, bytes).map_err(|source| io_error("write", &path, source))?;
		println!("wrote {} ({:#x} bytes)", path.display(), bytes.len());
	}
	Ok(())
}

fn load_layer(path: &Path) -> Result<MappedLayer> {
	let metadata = fs::symlink_metadata(path).map_err(|source| io_error("inspect layer directory", path, source))?;
	if !metadata.is_dir() {
		return Err(CliError::UnsupportedLayerEntry { path: path.to_owned() });
	}

	let mut boot = MappedBoot::default();
	let mut inputs = Vec::new();
	for entry in read_directory(path)? {
		let entry_path = entry.path();
		let name = utf8_file_name(&entry_path)?;
		match name.as_str() {
			"boot" => {
				let (loaded, mut loaded_inputs) = load_boot_directory(&entry_path)?;
				boot = loaded;
				inputs.append(&mut loaded_inputs);
			}
			_ => return Err(CliError::UnsupportedLayerEntry { path: entry_path }),
		}
	}
	Ok(MappedLayer { boot, inputs })
}

fn load_boot_directory(path: &Path) -> Result<(MappedBoot, Vec<InputIdentity>)> {
	require_directory(path)?;
	let mut boot = MappedBoot::default();
	let mut inputs = Vec::new();
	for entry in read_directory(path)? {
		let entry_path = entry.path();
		let name = utf8_file_name(&entry_path)?;
		let target = match name.as_str() {
			"sb.bin" => &mut boot.sb,
			"sc.bin" => &mut boot.sc,
			"sd.bin" => &mut boot.sd,
			"se.bin" => &mut boot.se,
			"hypervisor.bin" => &mut boot.hypervisor,
			"xboxkrnl.exe" => &mut boot.kernel,
			_ => return Err(CliError::UnknownBootInput { path: entry_path }),
		};
		let (map, input) = map_input(&entry_path)?;
		*target = Some(map);
		inputs.push(input);
	}
	Ok((boot, inputs))
}

fn read_directory(path: &Path) -> Result<Vec<fs::DirEntry>> {
	let mut entries: Vec<fs::DirEntry> = fs::read_dir(path)
		.map_err(|source| io_error("read directory", path, source))?
		.collect::<std::result::Result<_, _>>()
		.map_err(|source| io_error("read directory entry", path, source))?;
	entries.sort_by_key(fs::DirEntry::file_name);
	Ok(entries)
}

fn require_directory(path: &Path) -> Result<()> {
	let metadata = fs::symlink_metadata(path).map_err(|source| io_error("inspect layer entry", path, source))?;
	if !metadata.is_dir() || metadata.file_type().is_symlink() {
		return Err(CliError::UnsupportedLayerEntry { path: path.to_owned() });
	}
	Ok(())
}

fn require_regular_file(path: &Path) -> Result<fs::Metadata> {
	let metadata = fs::symlink_metadata(path).map_err(|source| io_error("inspect layer input", path, source))?;
	if !metadata.is_file() || metadata.file_type().is_symlink() {
		return Err(CliError::UnsupportedLayerEntry { path: path.to_owned() });
	}
	Ok(metadata)
}

fn map_input(path: &Path) -> Result<(Mmap, InputIdentity)> {
	let metadata = require_regular_file(path)?;
	if metadata.len() == 0 {
		return Err(CliError::EmptyInput { path: path.to_owned() });
	}
	let file = open_file(path, "open layer input")?;
	let identity = identify_file(&file, path)?;
	let map = map_file(&file, path)?;
	Ok((map, identity))
}

fn open_file(path: &Path, operation: &'static str) -> Result<File> {
	File::open(path).map_err(|source| io_error(operation, path, source))
}

fn map_file(file: &File, path: &Path) -> Result<Mmap> {
	unsafe { Mmap::map(file) }.map_err(|source| io_error("map input", path, source))
}

fn utf8_file_name(path: &Path) -> Result<String> {
	path.file_name()
		.and_then(|name| name.to_str())
		.map(str::to_owned)
		.ok_or_else(|| CliError::NonUtf8LayerName { path: path.to_owned() })
}

fn identify_file(file: &File, path: &Path) -> Result<InputIdentity> {
	let file = file.try_clone().map_err(|source| io_error("identify input", path, source))?;
	let handle = Handle::from_file(file).map_err(|source| io_error("identify input", path, source))?;
	Ok(InputIdentity { path: path.to_owned(), handle })
}

fn check_output_aliases(
	base_file: &File,
	base_path: &Path,
	output_path: &Path,
	layer_inputs: &[InputIdentity],
) -> Result<()> {
	let output_file = match File::open(output_path) {
		Ok(file) => file,
		Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
		Err(source) => return Err(io_error("inspect existing output", output_path, source)),
	};
	let output_handle =
		Handle::from_file(output_file).map_err(|source| io_error("identify existing output", output_path, source))?;
	let base_handle = identify_file(base_file, base_path).map(|identity| identity.handle)?;
	if base_handle == output_handle {
		return Err(CliError::OutputAliasesBase { base: base_path.to_owned(), output: output_path.to_owned() });
	}
	for input in layer_inputs {
		if input.handle == output_handle {
			return Err(CliError::OutputAliasesLayerInput {
				input: input.path.clone(),
				output: output_path.to_owned(),
			});
		}
	}
	Ok(())
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> CliError {
	CliError::Io { operation, path: path.to_owned(), source }
}
