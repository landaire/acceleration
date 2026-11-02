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
use sfcx::StageRole;
use sfcx::io::SliceSource;
use shadowboot::BootLayer;
use shadowboot::Copyright;
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

	/// Keep the console binding of the SB, so the image boots only on the console that bound
	/// it. By default the binding is cleared and the image boots on any devkit.
	#[arg(long, conflicts_with = "extract")]
	bind: bool,

	/// Replace the copyright text of the flash header, in Latin-1, at most 0x4f bytes. Without
	/// this option the text is the layer directory's name. Nothing on the shadowboot path reads
	/// it.
	#[arg(long, value_name = "TEXT", conflicts_with = "extract")]
	copyright: Option<Copyright>,

	/// Keep the base image's copyright text instead of writing the layer directory's name.
	#[arg(long, conflicts_with_all = ["copyright", "extract"])]
	default_copyright: bool,

	/// Accept a retail base with a warning. Only whole-stage replacements of its own stages
	/// (boot/cb.bin, cb_a.bin, cb_b.bin, cc.bin, cd.bin, ce.bin) and --copyright apply; the
	/// result cannot shadowboot on a stock console.
	#[arg(long)]
	ignore_retail_checks: bool,

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
	#[error(
		"layer directory {path:?} has no UTF-8 name to use as the copyright text; pass --copyright TEXT or --default-copyright"
	)]
	LayerNameUnusable { path: PathBuf },
	#[error("layer directory name {name:?} cannot be the copyright text; pass --copyright TEXT or --default-copyright")]
	LayerNameCopyright {
		name: String,
		#[source]
		source: shadowboot::CopyrightError,
	},
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
	/// Complete encrypted stages, named by role.
	stages: Vec<(StageRole, Mmap)>,
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
			print_error(&error);
			ExitCode::FAILURE
		}
	}
}

fn run(arguments: Arguments) -> Result<()> {
	let base_file = open_file(&arguments.base, "open base image")?;
	let base = map_file(&base_file, &arguments.base)?;
	let (layer_path, output_path) = match (arguments.layer, arguments.output, arguments.extract) {
		(_, _, Some(directory)) => {
			let policy = policy(arguments.strict);
			return extract(&base, arguments.format, policy, arguments.ignore_retail_checks, &directory);
		}
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
		bind: arguments.bind,
		copyright: arguments.copyright,
		default_copyright: arguments.default_copyright,
		ignore_retail_checks: arguments.ignore_retail_checks,
	};
	let mapped_layer = load_layer(&arguments.layer)?;
	let copyright = output_copyright(arguments.copyright, arguments.default_copyright, &arguments.layer)?;
	check_output_aliases(&base_file, &arguments.base, &arguments.output, &mapped_layer.inputs)?;

	let base = load_base(&base, arguments.format, policy(arguments.strict))?;
	let region = base.boot_region();
	let output_size = shadowboot::output_size(&region)?;
	let stages: Vec<(StageRole, &[u8])> =
		mapped_layer.boot.stages.iter().map(|(role, map)| (*role, &map[..])).collect();
	let layer = BootLayer {
		stages: &stages,
		hypervisor: mapped_layer.boot.hypervisor.as_deref(),
		kernel: mapped_layer.boot.kernel.as_deref(),
		rebuild_se: arguments.rebuild_se,
		bind: arguments.bind,
		copyright: copyright.as_ref(),
		ignore_retail_checks: arguments.ignore_retail_checks,
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

	let copyright = match &copyright {
		Some(text) => format!("{:?}", text.to_string()),
		None => "base".to_owned(),
	};
	println!(
		"wrote {} ({:#x} bytes): boot replaced={} se-rebuilt={} sd-signed={} sb-binding={} copyright={}",
		arguments.output.display(),
		output_size,
		report.replaced_boot_stages,
		report.rebuilt_se,
		report.updated_sd_signature,
		report.binding,
		copyright,
	);
	Ok(())
}

/// The copyright text written into the output: the given text, otherwise the layer directory's
/// name, or none to keep the base's own text.
fn output_copyright(text: Option<Copyright>, keep_base: bool, layer: &Path) -> Result<Option<Copyright>> {
	if keep_base {
		return Ok(None);
	}
	if let Some(text) = text {
		return Ok(Some(text));
	}

	// `--layer .` has no file name; its canonical path does.
	let path = fs::canonicalize(layer).map_err(|source| io_error("resolve layer directory", layer, source))?;
	let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
		return Err(CliError::LayerNameUnusable { path });
	};
	let copyright = name.parse().map_err(|source| CliError::LayerNameCopyright { name: name.to_owned(), source })?;

	Ok(Some(copyright))
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

	for warning in warnings.iter().take(SHOWN) {
		print_diagnostic(Diagnostic::Warning, warning);
	}
	if warnings.len() > SHOWN {
		print_diagnostic(Diagnostic::Warning, format_args!("{} more warnings", warnings.len() - SHOWN));
	}
}

/// Prints `error` and the chain of errors that caused it.
fn print_error(error: &CliError) {
	print_diagnostic(Diagnostic::Error, error);
	let mut source = std::error::Error::source(error);
	while let Some(error) = source {
		anstream::eprintln!("  caused by: {error}");
		source = error.source();
	}
}

#[derive(Debug, Clone, Copy)]
enum Diagnostic {
	Warning,
	Error,
}

/// Prints a labeled diagnostic to stderr, with the label colored and the message in bold. A
/// message of several lines keeps its layout. Color is dropped when stderr is not a terminal or
/// `NO_COLOR` is set.
fn print_diagnostic<M>(kind: Diagnostic, message: M)
where
	M: fmt::Display,
{
	anstream::eprint!("{}", diagnostic(kind, &message));
}

fn diagnostic(kind: Diagnostic, message: &dyn fmt::Display) -> String {
	let (label, color) = match kind {
		Diagnostic::Warning => ("warning:", AnsiColor::Yellow),
		Diagnostic::Error => ("error:", AnsiColor::Red),
	};
	let label_style = Style::new().bold().fg_color(Some(color.into()));
	let bold = Style::new().bold();
	let message = message.to_string();
	let mut lines = message.lines();
	let mut text =
		format!("shadowboot: {label_style}{label}{label_style:#} {bold}{}{bold:#}\n", lines.next().unwrap_or(""));
	for line in lines {
		text.push_str(&format!("{bold}{line}{bold:#}\n"));
	}
	text
}

struct CompileArguments {
	base: PathBuf,
	format: FormatSelection,
	strict: bool,
	layer: PathBuf,
	output: PathBuf,
	rebuild_se: bool,
	bind: bool,
	copyright: Option<Copyright>,
	default_copyright: bool,
	ignore_retail_checks: bool,
}

/// Writes the hypervisor and kernel of the base SE as a layer's boot inputs.
fn extract(
	base: &[u8],
	format: FormatSelection,
	policy: IntegrityPolicy,
	ignore_retail_checks: bool,
	directory: &Path,
) -> Result<()> {
	let base = load_base(base, format, policy)?;
	let extracted = shadowboot::payload(&base.boot_region(), ignore_retail_checks)?;
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
		let (map, input) = match name.as_str() {
			"hypervisor.bin" => {
				let (map, input) = map_input(&entry_path)?;
				boot.hypervisor = Some(map);
				(None, input)
			}
			"xboxkrnl.exe" => {
				let (map, input) = map_input(&entry_path)?;
				boot.kernel = Some(map);
				(None, input)
			}
			_ => {
				// A stage file is named by its role in lower case, such as sb.bin or cb_a.bin.
				let role = name
					.strip_suffix(".bin")
					.and_then(|stem| StageRole::ALL.into_iter().find(|role| role.label().to_ascii_lowercase() == stem))
					.ok_or_else(|| CliError::UnknownBootInput { path: entry_path.clone() })?;
				let (map, input) = map_input(&entry_path)?;
				(Some((role, map)), input)
			}
		};
		boot.stages.extend(map);
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

#[cfg(test)]
mod tests {
	use super::*;

	/// The label is colored, the message is bold, and a message of several lines keeps them.
	#[test]
	fn diagnostics_keep_the_message_layout() {
		let error = diagnostic(Diagnostic::Error, &"first line\n\nlast line");
		assert_eq!(anstream::adapter::strip_str(&error).to_string(), "shadowboot: error: first line\n\nlast line\n");
		assert_eq!(
			error.lines().next(),
			Some("shadowboot: \u{1b}[1m\u{1b}[31merror:\u{1b}[0m \u{1b}[1mfirst line\u{1b}[0m")
		);

		let warning = diagnostic(Diagnostic::Warning, &"retail");
		assert_eq!(warning, "shadowboot: \u{1b}[1m\u{1b}[33mwarning:\u{1b}[0m \u{1b}[1mretail\u{1b}[0m\n");
	}

	#[test]
	fn copyright_defaults_to_the_layer_name() {
		let root = tempfile::tempdir().unwrap();
		let layer = root.path().join("rgloader-17489");
		fs::create_dir(&layer).unwrap();

		let named = output_copyright(None, false, &layer).unwrap().unwrap();
		assert_eq!(named.to_string(), "rgloader-17489");

		// A relative path without a final component still names the directory.
		let dotted = output_copyright(None, false, &layer.join(".")).unwrap().unwrap();
		assert_eq!(dotted, named);

		let given: Copyright = "custom".parse().unwrap();
		assert_eq!(output_copyright(Some(given.clone()), false, &layer).unwrap(), Some(given));
		assert_eq!(output_copyright(None, true, &layer).unwrap(), None);

		let long = root.path().join("x".repeat(0x50));
		fs::create_dir(&long).unwrap();
		let error = output_copyright(None, false, &long).unwrap_err();
		assert!(matches!(
			error,
			CliError::LayerNameCopyright { source: shadowboot::CopyrightError::TooLong { length: 0x50 }, .. }
		));
	}
}
