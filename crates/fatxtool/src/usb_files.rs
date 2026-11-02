//! A USB storage directory, `Xbox360` on the stick, read as the one device the console builds
//! from its `Data####` files.

use std::fs::File;
use std::path::Path;
use std::path::PathBuf;

use fatx::io::SliceSource;
use memmap2::Mmap;
use xenon_disk::ByteCount;
use xenon_disk::JoinedSource;
use xenon_disk::UsbStorageLayout;
use xenon_disk::usb_storage::MAXIMUM_FILE_COUNT;

use crate::error::DiskToolError;
use crate::output::InputFile;
use crate::source::VolumeSource;

/// The joined files of a USB storage directory.
pub type UsbStorageSource = JoinedSource<SliceSource<Mmap>>;

impl<R> VolumeSource for JoinedSource<R>
where
	R: VolumeSource,
{
	type Warning = R::Warning;

	fn take_warnings(&self) -> Vec<R::Warning> {
		self.files().iter().flat_map(VolumeSource::take_warnings).collect()
	}
}

/// Maps `Data0000`, `Data0001` and the files after them in `directory` read-only and joins
/// them. The files stop at the first missing name.
pub fn open_usb_storage(directory: &Path) -> Result<(UsbStorageSource, InputFile), DiskToolError> {
	let mut files: Vec<(File, PathBuf)> = Vec::new();
	// One name past the maximum, so that a directory with too many files is rejected.
	for index in 0..=MAXIMUM_FILE_COUNT {
		let path = directory.join(UsbStorageLayout::file_name(index));
		if !path.is_file() {
			break;
		}

		let file = File::open(&path).map_err(|source| DiskToolError::OpenImage { path: path.clone(), source })?;
		files.push((file, path));
	}

	let mut lengths = Vec::with_capacity(files.len());
	for (file, path) in &files {
		let metadata = file.metadata().map_err(|source| DiskToolError::OpenImage { path: path.clone(), source })?;
		lengths.push(ByteCount::new(metadata.len()));
	}
	let layout = UsbStorageLayout::from_file_lengths(&lengths)
		.map_err(|source| DiskToolError::UsbStorage { path: directory.to_owned(), source })?;

	let input = InputFile::with_files(directory, files.iter().map(|(file, path)| (file, path.as_path())))?;
	let mut maps = Vec::with_capacity(files.len());
	for (file, path) in &files {
		// SAFETY: the files are mapped read-only; they are not expected to change while the
		// command runs.
		let map = unsafe { Mmap::map(file) }.map_err(|source| DiskToolError::MapFile { path: path.clone(), source })?;
		maps.push(SliceSource(map));
	}
	let source = JoinedSource::new(layout, maps)
		.map_err(|source| DiskToolError::UsbStorage { path: directory.to_owned(), source })?;

	Ok((source, input))
}
