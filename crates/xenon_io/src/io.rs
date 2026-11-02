use std::io;
use std::ops::Range;
use std::sync::Mutex;
use std::sync::PoisonError;

pub use fskit::AsyncReadAt;
pub use fskit::ReadAt;

use crate::error::ReadError;
use crate::error::ReadErrorKind;
use crate::error::WriteError;
use crate::machine::Effect;
use crate::machine::Machine;
use crate::machine::Step;
use crate::machine::WriteMachine;

/// Synchronous positional writes to a source.
///
/// `write_at` takes `&self`, as [`ReadAt::read_at`] does, so one source serves the reads and the
/// writes of a driver and can be shared by readers. Positional file writes need no exclusive
/// borrow; an in-memory source serializes its writes with a lock.
///
/// A failed or interrupted write leaves a prefix of its bytes that ends on a 512-byte sector
/// boundary of the source, or at the end of the write: storage devices write whole sectors, and
/// a host file system writes whole pages or sectors. Machines rely on this, so a source whose
/// writes can tear inside a sector breaks their guarantees.
pub trait WriteAt {
	/// Writes all of `data` at `offset`, or fails having written a sector-aligned prefix.
	fn write_at(&self, offset: u64, data: &[u8]) -> io::Result<()>;

	/// Makes every completed write durable: a power loss after this returns loses none of them.
	fn flush(&self) -> io::Result<()>;
}

/// Async counterpart of [`WriteAt`], with the same sector-aligned failure model.
pub trait AsyncWriteAt: Send + Sync {
	fn write_at(&self, offset: u64, data: &[u8]) -> impl Future<Output = io::Result<()>> + Send;

	fn flush(&self) -> impl Future<Output = io::Result<()>> + Send;
}

impl<T> AsyncWriteAt for T
where
	T: WriteAt + Send + Sync,
{
	async fn write_at(&self, offset: u64, data: &[u8]) -> io::Result<()> {
		WriteAt::write_at(self, offset, data)
	}

	async fn flush(&self) -> io::Result<()> {
		WriteAt::flush(self)
	}
}

/// An in-memory image, such as a `Vec<u8>` or a memory map.
#[derive(Debug, Clone)]
pub struct SliceSource<T>(pub T);

impl<T> ReadAt for SliceSource<T>
where
	T: AsRef<[u8]>,
{
	fn read_at(&self, _context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		let data = self.0.as_ref();
		slice_range(data.len(), &range).map(|range| &data[range])
	}
}

/// A fixed-size in-memory image that can be read and written, such as a test image. Reads
/// return copies of the bytes.
#[derive(Debug, Default)]
pub struct MemorySource {
	bytes: Mutex<Vec<u8>>,
}

impl MemorySource {
	pub fn new(bytes: Vec<u8>) -> Self {
		Self { bytes: Mutex::new(bytes) }
	}

	/// An image of `length` zero bytes.
	pub fn zeroed(length: usize) -> Self {
		Self::new(vec![0; length])
	}

	pub fn len(&self) -> usize {
		self.lock().len()
	}

	pub fn is_empty(&self) -> bool {
		self.lock().is_empty()
	}

	/// A copy of the whole image.
	pub fn to_vec(&self) -> Vec<u8> {
		self.lock().clone()
	}

	pub fn into_inner(self) -> Vec<u8> {
		self.bytes.into_inner().unwrap_or_else(PoisonError::into_inner)
	}

	fn lock(&self) -> std::sync::MutexGuard<'_, Vec<u8>> {
		// A panic while the lock was held cannot leave the bytes half-updated: every write is
		// one slice copy.
		self.bytes.lock().unwrap_or_else(PoisonError::into_inner)
	}
}

impl Clone for MemorySource {
	fn clone(&self) -> Self {
		Self::new(self.to_vec())
	}
}

impl ReadAt for MemorySource {
	fn read_at(&self, _context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		let bytes = self.lock();
		let range = slice_range(bytes.len(), &range)?;

		Ok(bytes[range].to_vec())
	}
}

impl WriteAt for MemorySource {
	fn write_at(&self, offset: u64, data: &[u8]) -> io::Result<()> {
		let mut bytes = self.lock();
		let end = offset.checked_add(data.len() as u64).ok_or_else(|| {
			io::Error::new(io::ErrorKind::InvalidInput, format!("{:#x} bytes at {offset:#x} end past 2^64", data.len()))
		})?;
		let range = slice_range(bytes.len(), &(offset..end))?;
		bytes[range].copy_from_slice(data);

		Ok(())
	}

	fn flush(&self) -> io::Result<()> {
		// The bytes have no storage behind them for a write to reach.
		Ok(())
	}
}

/// A source whose writes fail with [`io::ErrorKind::ReadOnlyFilesystem`], for an API that needs
/// a writable source when only reads will happen.
#[derive(Debug, Clone)]
pub struct ReadOnly<S>(pub S);

impl<S> ReadAt for ReadOnly<S>
where
	S: ReadAt,
{
	fn read_at(&self, context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		self.0.read_at(context, range)
	}
}

impl<S> WriteAt for ReadOnly<S> {
	fn write_at(&self, offset: u64, data: &[u8]) -> io::Result<()> {
		Err(io::Error::new(
			io::ErrorKind::ReadOnlyFilesystem,
			format!("the source is read-only; {:#x} bytes at {offset:#x} were not written", data.len()),
		))
	}

	fn flush(&self) -> io::Result<()> {
		// No write ever succeeded, so none waits to become durable.
		Ok(())
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

/// Runs a write machine to completion with synchronous reads and writes, in the machine's
/// order.
pub fn drive_write<M, S>(mut machine: M, source: &S) -> Result<M::Output, M::Error>
where
	M: WriteMachine,
	S: ReadAt + WriteAt,
{
	loop {
		let range = match machine.step()? {
			Effect::Done(output) => return Ok(output),
			Effect::Write { offset, data } => {
				WriteAt::write_at(source, offset, data).map_err(|error| write_failed(offset, data, error))?;
				continue;
			}
			Effect::Flush => {
				WriteAt::flush(source).map_err(WriteError::Flush)?;
				continue;
			}
			Effect::Read(range) => range,
		};
		let data = ReadAt::read_at(source, &(), range.clone()).map_err(|error| read_failed(range, error))?;
		machine.feed(data.as_ref())?;
	}
}

/// Runs a write machine to completion with asynchronous reads and writes, in the machine's
/// order.
pub async fn drive_write_async<M, S>(mut machine: M, source: &S) -> Result<M::Output, M::Error>
where
	M: WriteMachine,
	S: AsyncReadAt + AsyncWriteAt,
{
	loop {
		let range = match machine.step()? {
			Effect::Done(output) => return Ok(output),
			Effect::Write { offset, data } => {
				AsyncWriteAt::write_at(source, offset, data)
					.await
					.map_err(|error| write_failed(offset, data, error))?;
				continue;
			}
			Effect::Flush => {
				AsyncWriteAt::flush(source).await.map_err(WriteError::Flush)?;
				continue;
			}
			Effect::Read(range) => range,
		};
		let data = AsyncReadAt::read_at(source, &(), range.clone()).await.map_err(|error| read_failed(range, error))?;
		machine.feed(data.as_ref())?;
	}
}

/// `range` as an index range of a `length`-byte image.
fn slice_range(length: usize, range: &Range<u64>) -> io::Result<Range<usize>> {
	usize::try_from(range.start)
		.ok()
		.zip(usize::try_from(range.end).ok())
		.filter(|(start, end)| start <= end && *end <= length)
		.map(|(start, end)| start..end)
		.ok_or_else(|| {
			io::Error::new(
				io::ErrorKind::UnexpectedEof,
				format!("range {range:#x?} extends past the {length:#x}-byte image"),
			)
		})
}

fn read_failed(range: Range<u64>, error: io::Error) -> ReadError {
	ReadError { range, kind: ReadErrorKind::Io(error) }
}

fn write_failed(offset: u64, data: &[u8], source: io::Error) -> WriteError {
	WriteError::Write { offset, length: data.len() as u64, source }
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::machine::expect_len;

	/// Increments the byte at offset 0 and writes it back, `remaining` times.
	struct Increment {
		staged: [u8; 1],
		write_next: bool,
		remaining: u8,
	}

	impl Increment {
		fn new(remaining: u8) -> Self {
			Self { staged: [0], write_next: false, remaining }
		}
	}

	impl WriteMachine for Increment {
		type Output = ();
		type Error = TestError;

		fn step(&mut self) -> Result<Effect<'_, ()>, TestError> {
			if self.write_next {
				self.write_next = false;
				self.remaining -= 1;
				return Ok(Effect::Write { offset: 0, data: &self.staged });
			}
			if self.remaining == 0 {
				return Ok(Effect::Done(()));
			}

			Ok(Effect::Read(0..1))
		}

		fn feed(&mut self, bytes: &[u8]) -> Result<(), TestError> {
			expect_len(&(0..1), bytes)?;
			self.staged = [bytes[0] + 1];
			self.write_next = true;

			Ok(())
		}
	}

	#[derive(Debug, thiserror::Error)]
	enum TestError {
		#[error(transparent)]
		Read(#[from] ReadError),
		#[error(transparent)]
		Write(#[from] WriteError),
	}

	#[test]
	fn write_machine_reads_its_own_writes() {
		let source = MemorySource::new(vec![5, 9]);

		drive_write(Increment::new(2), &source).unwrap();

		assert_eq!(source.into_inner(), [7, 9]);
	}

	#[test]
	fn async_driver_matches_sync_driver() {
		let source = MemorySource::new(vec![5, 9]);

		futures::executor::block_on(drive_write_async(Increment::new(2), &source)).unwrap();

		assert_eq!(source.into_inner(), [7, 9]);
	}

	#[test]
	fn writes_past_the_image_fail() {
		let source = MemorySource::zeroed(4);

		let error = WriteAt::write_at(&source, 3, &[1, 2]).unwrap_err();

		assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
		assert_eq!(source.into_inner(), [0; 4]);
	}

	#[test]
	fn read_only_source_refuses_writes() {
		let source = ReadOnly(SliceSource([1u8, 2]));

		let error = WriteAt::write_at(&source, 0, &[3]).unwrap_err();

		assert_eq!(error.kind(), io::ErrorKind::ReadOnlyFilesystem);
		assert_eq!(ReadAt::read_at(&source, &(), 0..2).unwrap().as_ref(), [1, 2]);
	}
}
