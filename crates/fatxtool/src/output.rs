//! Output files, which must never be the input file under another name, and are never reached
//! through a symbolic link.

use std::ffi::OsString;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use same_file::Handle;

use crate::error::OutputError;

/// The file a command reads, or the files of an input made of several, kept open so that
/// outputs can be compared with them.
#[derive(Debug)]
pub struct InputFile {
	path: PathBuf,
	files: Vec<(PathBuf, Handle)>,
}

impl InputFile {
	pub fn new(file: &File, path: &Path) -> Result<Self, OutputError> {
		Ok(Self { path: path.to_owned(), files: vec![(path.to_owned(), file_handle(file, path)?)] })
	}

	/// An input at `path` that is read from `files`, such as a directory of USB storage files.
	pub fn with_files<'a, I>(path: &Path, files: I) -> Result<Self, OutputError>
	where
		I: IntoIterator<Item = (&'a File, &'a Path)>,
	{
		let files = files
			.into_iter()
			.map(|(file, file_path)| Ok((file_path.to_owned(), file_handle(file, file_path)?)))
			.collect::<Result<_, OutputError>>()?;

		Ok(Self { path: path.to_owned(), files })
	}

	pub fn path(&self) -> &Path {
		&self.path
	}

	/// The input file that `handle` refers to.
	fn aliased(&self, handle: &Handle) -> Option<&Path> {
		self.files.iter().find(|(_, input)| input == handle).map(|(path, _)| path.as_path())
	}
}

/// Creates or truncates `output`, unless it is the input file, through any path or hard link,
/// or a symbolic link.
pub fn create_output(input: &InputFile, output: &Path) -> Result<File, OutputError> {
	reject_symbolic_link(output)?;

	// Opened without truncation so that an alias of the input is detected before any byte of
	// it changes.
	let output_file = write_options()
		.create(true)
		.truncate(false)
		.open(output)
		.map_err(|source| OutputError::CreateOutput { path: output.to_owned(), source })?;
	if let Some(input) = input.aliased(&file_handle(&output_file, output)?) {
		return Err(OutputError::OutputAliasesInput { input: input.to_owned(), output: output.to_owned() });
	}

	output_file.set_len(0).map_err(|source| OutputError::TruncateOutput { path: output.to_owned(), source })?;

	Ok(output_file)
}

/// A new file beside `output` that replaces it only when [`TemporaryOutput::persist`] is called;
/// otherwise it is removed when dropped.
#[derive(Debug)]
pub struct TemporaryOutput<'a> {
	input: &'a InputFile,
	file: File,
	temporary: PathBuf,
	output: PathBuf,
	persisted: bool,
}

impl<'a> TemporaryOutput<'a> {
	pub fn create(input: &'a InputFile, output: &Path) -> Result<Self, OutputError> {
		check_replaceable(input, output)?;

		let name = output.file_name().ok_or_else(|| OutputError::NoFileName { path: output.to_owned() })?;
		let mut temporary_name = OsString::from(".");
		temporary_name.push(name);
		temporary_name.push(format!(".{}.partial", std::process::id()));
		let temporary = output.with_file_name(temporary_name);
		let file = write_options()
			.create_new(true)
			.open(&temporary)
			.map_err(|source| OutputError::CreateOutput { path: temporary.clone(), source })?;

		Ok(Self { input, file, temporary, output: output.to_owned(), persisted: false })
	}

	pub fn file(&mut self) -> &mut File {
		&mut self.file
	}

	pub fn temporary_path(&self) -> &Path {
		&self.temporary
	}

	/// Renames the file to the output path.
	pub fn persist(mut self) -> Result<(), OutputError> {
		check_replaceable(self.input, &self.output)?;
		fs::rename(&self.temporary, &self.output).map_err(|source| OutputError::RenameOutput {
			from: self.temporary.clone(),
			to: self.output.clone(),
			source,
		})?;
		self.persisted = true;

		Ok(())
	}
}

impl Drop for TemporaryOutput<'_> {
	fn drop(&mut self) {
		if !self.persisted {
			// Best effort: the file holds data that failed a check, and an error is already
			// being returned.
			let _ = fs::remove_file(&self.temporary);
		}
	}
}

/// Opens `path` for reading and writing when it is a regular file, not a device or a symbolic
/// link.
pub fn open_writable_image(path: &Path) -> Result<File, OutputError> {
	let metadata = fs::symlink_metadata(path)
		.map_err(|source| OutputError::OpenWritableImage { path: path.to_owned(), source })?;
	if !metadata.file_type().is_file() {
		return Err(OutputError::NotARegularFile { path: path.to_owned() });
	}

	let mut options = OpenOptions::new();
	options.read(true).write(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;

		options.custom_flags(libc::O_NOFOLLOW);
	}
	#[cfg(windows)]
	{
		use std::os::windows::fs::OpenOptionsExt;

		// FILE_FLAG_OPEN_REPARSE_POINT: a link swapped in after the check opens as itself and
		// fails the regular-file check below.
		const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

		options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
	}
	let file = options.open(path).map_err(|source| OutputError::OpenWritableImage { path: path.to_owned(), source })?;

	// The path may have been replaced between the check and the open.
	let opened = file.metadata().map_err(|source| OutputError::OpenWritableImage { path: path.to_owned(), source })?;
	if !opened.file_type().is_file() {
		return Err(OutputError::NotARegularFile { path: path.to_owned() });
	}

	Ok(file)
}

/// Rejects `path` when it exists as a symbolic link.
pub fn reject_symbolic_link(path: &Path) -> Result<(), OutputError> {
	match fs::symlink_metadata(path) {
		Ok(metadata) if metadata.file_type().is_symlink() => Err(OutputError::SymbolicLink { path: path.to_owned() }),
		Ok(_) => Ok(()),
		Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
		Err(source) => Err(OutputError::IdentifyFile { path: path.to_owned(), source }),
	}
}

/// Rejects an existing `output` that is a symbolic link or the input file.
fn check_replaceable(input: &InputFile, output: &Path) -> Result<(), OutputError> {
	reject_symbolic_link(output)?;

	match Handle::from_path(output) {
		Ok(handle) => match input.aliased(&handle) {
			Some(input) => Err(OutputError::OutputAliasesInput { input: input.to_owned(), output: output.to_owned() }),
			None => Ok(()),
		},
		Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
		Err(source) => Err(OutputError::IdentifyFile { path: output.to_owned(), source }),
	}
}

/// Write access that does not follow a symbolic link at the last path component, where the
/// platform supports it.
fn write_options() -> OpenOptions {
	let mut options = OpenOptions::new();
	options.write(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;

		options.custom_flags(libc::O_NOFOLLOW);
	}

	options
}

fn file_handle(file: &File, path: &Path) -> Result<Handle, OutputError> {
	let identify = |source| OutputError::IdentifyFile { path: path.to_owned(), source };
	let file = file.try_clone().map_err(identify)?;

	Handle::from_file(file).map_err(identify)
}
