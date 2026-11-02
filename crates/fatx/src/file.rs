//! File data, as readwrit.c reads it: `FileSize` bytes through the file's cluster chain.

use std::collections::VecDeque;
use std::ops::Range;

use crate::entry::EntryRecord;
use crate::error::FatxError;
use crate::io;
use crate::io::AsyncReadAt;
use crate::io::ReadAt;
use crate::machine::Machine;
use crate::machine::Step;
use crate::machine::expect_len;
use crate::volume::MAXIMUM_READ_SIZE;
use crate::volume::Volume;

impl Volume {
	/// The source ranges that hold a live file's bytes, in order.
	pub fn file_ranges(&self, record: &EntryRecord) -> Result<Vec<Range<u64>>, FatxError> {
		if record.is_directory() {
			return Err(FatxError::NotAFile { name: record.name().to_owned() });
		}

		let size = u64::from(record.size());
		let runs = self.file_runs(record.first_cluster(), size)?;

		Ok(self.run_ranges(&runs, size)?)
	}

	/// A machine that reads a live file.
	pub fn file_reader(&self, record: &EntryRecord) -> Result<ReadRanges, FatxError> {
		ReadRanges::new(self.file_ranges(record)?)
	}

	pub fn read_file<R>(&self, source: &R, record: &EntryRecord) -> Result<Vec<u8>, FatxError>
	where
		R: ReadAt,
	{
		io::drive(self.file_reader(record)?, source)
	}

	pub async fn read_file_async<R>(&self, source: &R, record: &EntryRecord) -> Result<Vec<u8>, FatxError>
	where
		R: AsyncReadAt,
	{
		io::drive_async(self.file_reader(record)?, source).await
	}
}

/// Reads source ranges and returns their bytes concatenated, at most [`MAXIMUM_READ_SIZE`] bytes
/// per read.
#[derive(Debug)]
pub struct ReadRanges {
	ranges: VecDeque<Range<u64>>,
	bytes: Option<Vec<u8>>,
	requested: Option<Range<u64>>,
}

impl ReadRanges {
	/// `ranges` come from a file's cluster chain: each is forward and their total is at most a
	/// file's `u32` size.
	pub(crate) fn new<I>(ranges: I) -> Result<Self, FatxError>
	where
		I: IntoIterator<Item = Range<u64>>,
	{
		let ranges: VecDeque<Range<u64>> = ranges.into_iter().collect();
		let total = ranges.iter().map(|range| range.end - range.start).sum::<u64>();
		let capacity = usize::try_from(total).map_err(|_| FatxError::ReadTooLarge)?;

		Ok(Self { ranges, bytes: Some(Vec::with_capacity(capacity)), requested: None })
	}
}

impl Machine for ReadRanges {
	type Output = Vec<u8>;
	type Error = FatxError;

	fn step(&mut self) -> Result<Step<Self::Output>, FatxError> {
		assert!(self.requested.is_none(), "ReadRanges stepped before the requested read was fed");
		assert!(self.bytes.is_some(), "ReadRanges stepped after completion");

		let Some(mut range) = self.ranges.pop_front() else {
			return Ok(Step::Done(self.bytes.take().expect("checked above")));
		};
		if range.end - range.start > MAXIMUM_READ_SIZE {
			let split = range.start + MAXIMUM_READ_SIZE;
			self.ranges.push_front(split..range.end);
			range.end = split;
		}
		self.requested = Some(range.clone());

		Ok(Step::Read(range))
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), FatxError> {
		let range = self.requested.take().expect("ReadRanges fed without a requested read");
		expect_len(&range, bytes)?;
		self.bytes.as_mut().expect("bytes present while a read is requested").extend_from_slice(bytes);

		Ok(())
	}
}
