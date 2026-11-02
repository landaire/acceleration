//! Copies of live files, read in bounded chunks. Outputs are never written through a symbolic
//! link, so a link already under the destination cannot redirect the copy elsewhere.

use std::fs;
use std::fs::File;
use std::io;
use std::io::Write;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;

use fatx::IntegrityPolicy;
use fatx::ListedEntry;
use fatx::Listing;
use fatx::ReadAt;
use fatx::ReadError;
use fatx::ReadErrorKind;
use fatx::Volume;
use fatx::machine::expect_len;

use crate::command::normalize_path;
use crate::command::same_path;
use crate::command::walk;
use crate::error::FatxToolError;
use crate::error::OutputError;
use crate::output::create_output;
use crate::output::reject_symbolic_link;
use crate::session::Session;
use crate::source::VolumeSource;

/// Largest read from the source.
pub(crate) const CHUNK_SIZE: u64 = 0x10_0000;

pub fn extract<S>(
	volume: &Volume,
	source: &S,
	session: &Session<'_>,
	path: &str,
	output: &Path,
) -> Result<(), FatxToolError>
where
	S: VolumeSource,
{
	let listing = walk(volume, source, session)?;
	let entry = find_live(&listing, path)?;
	if entry.record().is_directory() {
		return Err(FatxToolError::IsDirectory { path: entry.path().to_owned() });
	}

	let ranges = volume.file_ranges(entry.record())?;
	let mut file = create_output(session.input(), output)?;

	copy_ranges(source, session, &ranges, &mut file, output)
}

pub fn extract_all<S, W>(
	volume: &Volume,
	source: &S,
	session: &Session<'_>,
	directory: &Path,
	out: &mut W,
) -> Result<(), FatxToolError>
where
	S: VolumeSource,
	W: Write,
{
	let listing = walk(volume, source, session)?;
	create_directory(directory)?;

	let mut files = 0;
	let mut directories = 0;
	let mut skipped = 0;
	for entry in listing.entries() {
		let components: Vec<&str> = entry.path().split('/').filter(|component| !component.is_empty()).collect();
		if entry.record().is_directory() {
			create_components(directory, &components)?;
			directories += 1;
			continue;
		}

		let ranges = match volume.file_ranges(entry.record()) {
			Ok(ranges) => ranges,
			Err(error) if session.policy() == IntegrityPolicy::Ignore => {
				session.report(&[format!("{} skipped: {error}", entry.path())]);
				skipped += 1;
				continue;
			}
			Err(error) => return Err(error.into()),
		};
		let (name, parents) = components.split_last().expect("a live entry has a name");
		let target = create_components(directory, parents)?.join(name);
		let mut file = create_output(session.input(), &target)?;
		copy_ranges(source, session, &ranges, &mut file, &target)?;
		files += 1;
	}

	writeln!(
		out,
		"extracted to {}: files {files}, directories {directories}, files skipped {skipped}",
		directory.display()
	)
	.map_err(FatxToolError::WriteListing)
}

/// Copies `ranges` of the source to `file`, checking the source's integrity before each write.
pub(crate) fn copy_ranges<S>(
	source: &S,
	session: &Session<'_>,
	ranges: &[Range<u64>],
	file: &mut File,
	path: &Path,
) -> Result<(), FatxToolError>
where
	S: VolumeSource,
{
	for range in ranges {
		let mut start = range.start;
		while start < range.end {
			let chunk = start..range.end.min(start + CHUNK_SIZE);
			let bytes = read_source(source, chunk.clone())?;
			session.check_source(source)?;

			file.write_all(bytes.as_ref())
				.map_err(|source| OutputError::WriteOutput { path: path.to_owned(), source })?;
			start = chunk.end;
		}
	}

	Ok(())
}

/// Reads `range` of the source, which must return all of it.
pub(crate) fn read_source<S>(source: &S, range: Range<u64>) -> Result<impl AsRef<[u8]>, FatxToolError>
where
	S: ReadAt,
{
	let bytes = ReadAt::read_at(source, &(), range.clone())
		.map_err(|error| ReadError { range: range.clone(), kind: ReadErrorKind::Io(error) })?;
	expect_len(&range, bytes.as_ref())?;

	Ok(bytes)
}

fn find_live<'a>(listing: &'a Listing, path: &str) -> Result<&'a ListedEntry, FatxToolError> {
	let path = normalize_path(path);

	listing.entries().iter().find(|entry| same_path(entry.path(), &path)).ok_or(FatxToolError::NotFound { path })
}

/// Creates the directories `components` names under `root`, one at a time, and returns the
/// last. Volume names cannot hold a separator, `.` or `..`, so every path stays under `root`;
/// an existing symbolic link among the components is refused rather than followed.
fn create_components(root: &Path, components: &[&str]) -> Result<PathBuf, OutputError> {
	let mut path = root.to_owned();
	for component in components {
		path.push(component);
		reject_symbolic_link(&path)?;
		match fs::create_dir(&path) {
			Ok(()) => {}
			Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => {
				// Created by an earlier entry, or present before the extraction.
			}
			Err(source) => return Err(OutputError::CreateDirectory { path, source }),
		}
	}

	Ok(path)
}

fn create_directory(path: &Path) -> Result<(), OutputError> {
	fs::create_dir_all(path).map_err(|source| OutputError::CreateDirectory { path: path.to_owned(), source })
}
