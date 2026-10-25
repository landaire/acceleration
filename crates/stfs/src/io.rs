use std::ops::Range;
use std::sync::Arc;

use crate::error::StfsError;

pub trait ReadAt {
	fn read_at(&self, range: Range<usize>) -> Result<impl AsRef<[u8]>, StfsError>;
}

pub struct SliceReader<'a>(pub &'a [u8]);

impl<'a> ReadAt for SliceReader<'a> {
	fn read_at(&self, range: Range<usize>) -> Result<impl AsRef<[u8]>, StfsError> {
		slice_range(self.0, range)
	}
}

impl ReadAt for Vec<u8> {
	fn read_at(&self, range: Range<usize>) -> Result<impl AsRef<[u8]>, StfsError> {
		slice_range(self, range)
	}
}

impl<T> ReadAt for Arc<T>
where
	T: AsRef<[u8]>,
{
	fn read_at(&self, range: Range<usize>) -> Result<impl AsRef<[u8]>, StfsError> {
		slice_range(self.as_ref().as_ref(), range)
	}
}

fn slice_range(data: &[u8], range: Range<usize>) -> Result<&[u8], StfsError> {
	if range.end > data.len() {
		return Err(StfsError::ReadError {
			offset: range.start,
			message: format!("read range {}..{} exceeds data length {}", range.start, range.end, data.len()),
		});
	}

	Ok(&data[range])
}
