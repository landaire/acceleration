use std::io;
use std::ops::Range;

pub use fskit::AsyncReadAt;
pub use fskit::ReadAt;

use crate::error::ReadError;
use crate::error::ReadErrorKind;
use crate::machine::Machine;
use crate::machine::Step;

/// An in-memory image, such as a `Vec<u8>` or a memory map.
#[derive(Debug, Clone)]
pub struct SliceSource<T>(pub T);

impl<T> ReadAt for SliceSource<T>
where
	T: AsRef<[u8]>,
{
	fn read_at(&self, _context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		let data = self.0.as_ref();
		usize::try_from(range.start)
			.ok()
			.zip(usize::try_from(range.end).ok())
			.and_then(|(start, end)| data.get(start..end))
			.ok_or_else(|| {
				io::Error::new(
					io::ErrorKind::UnexpectedEof,
					format!("range {range:#x?} extends past the {:#x}-byte image", data.len()),
				)
			})
	}
}

/// Runs `machine` to completion with synchronous reads.
pub fn drive<M, R>(mut machine: M, source: &R) -> Result<M::Output, M::Error>
where
	M: Machine,
	R: ReadAt,
{
	loop {
		let range = match machine.step()? {
			Step::Done(output) => return Ok(output),
			Step::Read(range) => range,
		};
		let data = ReadAt::read_at(source, &(), range.clone()).map_err(|error| read_failed(range, error))?;
		machine.feed(data.as_ref())?;
	}
}

/// Runs `machine` to completion with asynchronous reads.
pub async fn drive_async<M, R>(mut machine: M, source: &R) -> Result<M::Output, M::Error>
where
	M: Machine,
	R: AsyncReadAt,
{
	loop {
		let range = match machine.step()? {
			Step::Done(output) => return Ok(output),
			Step::Read(range) => range,
		};
		let data = AsyncReadAt::read_at(source, &(), range.clone()).await.map_err(|error| read_failed(range, error))?;
		machine.feed(data.as_ref())?;
	}
}

fn read_failed(range: Range<u64>, error: io::Error) -> ReadError {
	ReadError { range, kind: ReadErrorKind::Io(error) }
}
