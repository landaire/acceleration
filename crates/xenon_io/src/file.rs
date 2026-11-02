//! A file read and written in place with positional I/O.

use std::fs::File;
use std::io;
use std::ops::Range;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix::read_exact_at;
#[cfg(unix)]
use unix::write_all_at;
#[cfg(windows)]
use windows::read_exact_at;
#[cfg(windows)]
use windows::write_all_at;

use crate::io::ReadAt;
use crate::io::WriteAt;

/// An image file. Reads past the end of the file fail; writes past it extend the file.
#[derive(Debug)]
pub struct FileSource {
	file: File,
}

impl FileSource {
	pub fn new(file: File) -> Self {
		Self { file }
	}

	pub fn file(&self) -> &File {
		&self.file
	}

	pub fn into_inner(self) -> File {
		self.file
	}
}

impl ReadAt for FileSource {
	fn read_at(&self, _context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		let length = range
			.end
			.checked_sub(range.start)
			.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("range {range:#x?} is reversed")))?;
		let length = usize::try_from(length)
			.map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, format!("range {range:#x?} cannot be buffered")))?;
		let mut bytes = vec![0; length];
		read_exact_at(&self.file, &mut bytes, range.start)?;

		Ok(bytes)
	}
}

impl WriteAt for FileSource {
	fn write_at(&self, offset: u64, data: &[u8]) -> io::Result<()> {
		write_all_at(&self.file, data, offset)
	}

	fn flush(&self) -> io::Result<()> {
		self.file.sync_data()
	}
}
