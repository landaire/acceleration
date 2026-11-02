//! The storage device the fatw driver builds over the `Xbox360\Data####` files of a USB storage
//! device (`fs/fatw/fileinfo.c` FatwSetMountPartitionsInformation). The files are joined in
//! order into one logical device:
//!
//! - file 0: the reserved region, [`RESERVED_FILE_SIZE`] bytes;
//! - file 1: the FATX metadata of the Storage partition, a page of volume header and a FAT of
//!   16 KiB clusters;
//! - files 2 and up: the data of the Storage partition, 1 GiB each, the last holding the rest.
//!
//! The driver reads only the part of each file the layout needs; a longer file is not an error.
//! The `Data####` names are the ones XAM gives the files; the kernel only receives handles.

use std::io;
use std::ops::Range;

use fskit::ReadAt;

use crate::device::DeviceKind;
use crate::error::UsbStorageError;
use crate::partition::Partition;
use crate::partition::PartitionMap;
use crate::units::ByteCount;
use crate::units::ByteOffset;

/// `FATW_RESERVED_FILE_SIZE`.
pub const RESERVED_FILE_SIZE: ByteCount = ByteCount::new(0x2000_0000);
/// `FATW_DATA_FILE_SIZE`.
pub const DATA_FILE_SIZE: ByteCount = ByteCount::new(0x4000_0000);
/// `FATW_MAXIMUM_DATA_PARTITION_LENGTH`.
pub const MAXIMUM_DATA_LENGTH: ByteCount = ByteCount::new(32 << 30);
/// `FATW_FATX_CLUSTER_SIZE`: the cluster size of the Storage partition.
pub const CLUSTER_SIZE: u64 = 0x4000;
const METADATA_FILE_COUNT: usize = 2;
/// The most files the driver mounts: the two metadata files and 32 data files.
pub const MAXIMUM_FILE_COUNT: usize = (MAXIMUM_DATA_LENGTH.get() / DATA_FILE_SIZE.get()) as usize + METADATA_FILE_COUNT;
const PAGE_SIZE: u64 = 0x1000;
const FAT_ENTRY_SIZE: u64 = 4;

/// The file layout of a Storage partition with a given data length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbStorageLayout {
	data_length: ByteCount,
}

/// Where one file sits in the logical device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileExtent {
	pub index: usize,
	pub start: ByteOffset,
	/// The bytes of the file the device uses.
	pub length: ByteCount,
}

/// A part of a logical read that one file serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
	pub file: usize,
	/// Bytes of the file.
	pub range: Range<u64>,
}

impl UsbStorageLayout {
	/// The layout of a data partition of `data_length` bytes, which must be a whole number of
	/// clusters and at most [`MAXIMUM_DATA_LENGTH`].
	pub fn new(data_length: ByteCount) -> Result<Self, UsbStorageError> {
		if !data_length.get().is_multiple_of(CLUSTER_SIZE) {
			return Err(UsbStorageError::UnalignedDataLength { length: data_length });
		}
		if data_length > MAXIMUM_DATA_LENGTH {
			return Err(UsbStorageError::DataLengthTooLarge { length: data_length, maximum: MAXIMUM_DATA_LENGTH });
		}

		Ok(Self { data_length })
	}

	/// The layout that files of `lengths`, Data0000 first, hold. Every data file but the last
	/// holds a whole 1 GiB; the last holds the rest of the data, up to 1 GiB.
	pub fn from_file_lengths(lengths: &[ByteCount]) -> Result<Self, UsbStorageError> {
		let count = lengths.len();
		if count < METADATA_FILE_COUNT {
			return Err(UsbStorageError::TooFewFiles { count });
		}
		if count > MAXIMUM_FILE_COUNT {
			return Err(UsbStorageError::TooManyFiles { count, maximum: MAXIMUM_FILE_COUNT });
		}

		let data_files = &lengths[METADATA_FILE_COUNT..];
		let data_length = match data_files.split_last() {
			None => 0,
			Some((last, whole)) => {
				for (offset, length) in whole.iter().enumerate() {
					check_file(METADATA_FILE_COUNT + offset, *length, DATA_FILE_SIZE)?;
				}
				// A data file exists only for data, so the last one holds at least a cluster.
				check_file(METADATA_FILE_COUNT + whole.len(), *last, ByteCount::new(CLUSTER_SIZE))?;
				whole.len() as u64 * DATA_FILE_SIZE.get() + last.get().min(DATA_FILE_SIZE.get())
			}
		};

		let layout = Self::new(ByteCount::new(data_length))?;
		for extent in layout.file_extents() {
			check_file(extent.index, lengths[extent.index], extent.length)?;
		}

		Ok(layout)
	}

	/// The layout whose joined files are `length` bytes long.
	pub fn from_logical_length(length: ByteCount) -> Result<Self, UsbStorageError> {
		let not_a_layout = UsbStorageError::NotALayoutLength { length };
		let storage = length.get().checked_sub(RESERVED_FILE_SIZE.get()).ok_or(not_a_layout)?;

		// The storage length, metadata plus data, grows with the data length, so the cluster
		// count that gives `storage` is found by bisection.
		let mut low = 0;
		let mut high = storage.min(MAXIMUM_DATA_LENGTH.get()) / CLUSTER_SIZE;
		while low < high {
			let middle = low + (high - low).div_ceil(2);
			if storage_length(middle * CLUSTER_SIZE) <= storage {
				low = middle;
			} else {
				high = middle - 1;
			}
		}

		let data_length = low * CLUSTER_SIZE;
		if storage_length(data_length) != storage {
			return Err(not_a_layout);
		}

		Self::new(ByteCount::new(data_length))
	}

	pub const fn data_length(&self) -> ByteCount {
		self.data_length
	}

	/// The length of file 1: a page of volume header, then one 4-byte FAT entry per cluster and
	/// the reserved entry, in whole pages.
	pub fn metadata_size(&self) -> ByteCount {
		ByteCount::new(metadata_size(self.data_length.get()))
	}

	/// The length of the Storage partition: its metadata and its data.
	pub fn storage_length(&self) -> ByteCount {
		ByteCount::new(storage_length(self.data_length.get()))
	}

	pub fn logical_length(&self) -> ByteCount {
		ByteCount::new(RESERVED_FILE_SIZE.get() + self.storage_length().get())
	}

	pub fn file_count(&self) -> usize {
		self.data_length.get().div_ceil(DATA_FILE_SIZE.get()) as usize + METADATA_FILE_COUNT
	}

	/// The name XAM gives file `index`.
	pub fn file_name(index: usize) -> String {
		format!("Data{index:04}")
	}

	pub fn file_extents(&self) -> Vec<FileExtent> {
		let mut extents = Vec::with_capacity(self.file_count());
		let mut start = 0;
		for index in 0..self.file_count() {
			let length = match index {
				0 => RESERVED_FILE_SIZE.get(),
				1 => self.metadata_size().get(),
				_ => {
					let before = (index - METADATA_FILE_COUNT) as u64 * DATA_FILE_SIZE.get();
					(self.data_length.get() - before).min(DATA_FILE_SIZE.get())
				}
			};
			extents.push(FileExtent { index, start: ByteOffset::new(start), length: ByteCount::new(length) });
			start += length;
		}

		extents
	}

	/// The file reads that make up `range` of the logical device, or `None` when the range
	/// extends past its end.
	pub fn segments(&self, range: Range<u64>) -> Option<Vec<Segment>> {
		if range.end > self.logical_length().get() || range.start > range.end {
			return None;
		}

		let mut segments = Vec::new();
		for extent in self.file_extents() {
			let file_start = extent.start.get();
			let file_end = file_start + extent.length.get();
			let start = range.start.max(file_start);
			let end = range.end.min(file_end);
			if start < end {
				segments.push(Segment { file: extent.index, range: start - file_start..end - file_start });
			}
		}

		Some(segments)
	}

	/// The partitions fatw creates over the logical device (`FatwPiafPartitionTable`), and
	/// Partition0, the whole device.
	pub fn partition_map(&self) -> PartitionMap {
		let at = ByteOffset::new;
		let length = ByteCount::new;
		let partitions = vec![
			Partition::new("Partition0", at(0), self.logical_length()).raw(),
			Partition::new("Storage", at(RESERVED_FILE_SIZE.get()), self.storage_length()).alternate(),
			Partition::new("StorageSystem", at(0x0800_0400), length(0x0400_0000)),
			Partition::new("SystemAuxPartition", at(0x0000_0400), length(0x0800_0000)),
			Partition::new("SystemURLCachePartition", at(0x0c00_0400), length(0x0600_0000)).raw(),
			Partition::new("SystemExtPartition", at(0x1200_0400), length(0x0ce3_0000)),
			Partition::new("ReservationPartition", at(0), RESERVED_FILE_SIZE).raw(),
		];

		PartitionMap::new(DeviceKind::UsbStorage, self.logical_length(), partitions)
	}
}

/// The `Data####` files read as the one logical device of a [`UsbStorageLayout`].
#[derive(Debug, Clone)]
pub struct JoinedSource<R> {
	layout: UsbStorageLayout,
	files: Vec<R>,
}

impl<R> JoinedSource<R> {
	/// `files` must be the layout's files, Data0000 first.
	pub fn new(layout: UsbStorageLayout, files: Vec<R>) -> Result<Self, UsbStorageError> {
		let expected = layout.file_count();
		if files.len() != expected {
			return Err(UsbStorageError::FileCount { expected, actual: files.len() });
		}

		Ok(Self { layout, files })
	}

	pub const fn layout(&self) -> &UsbStorageLayout {
		&self.layout
	}

	pub fn files(&self) -> &[R] {
		&self.files
	}
}

impl<R> ReadAt for JoinedSource<R>
where
	R: ReadAt,
{
	fn read_at(&self, context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		let segments = self.layout.segments(range.clone()).ok_or_else(|| {
			io::Error::new(
				io::ErrorKind::UnexpectedEof,
				format!(
					"range {range:#x?} extends past the {:#x}-byte joined files",
					self.layout.logical_length().get()
				),
			)
		})?;

		if let [segment] = segments.as_slice() {
			let bytes = self.files[segment.file].read_at(context, segment.range.clone())?;
			return Ok(Joined::Single(bytes));
		}

		let mut joined = Vec::with_capacity(usize::try_from(range.end - range.start).unwrap_or(0));
		for segment in segments {
			let bytes = self.files[segment.file].read_at(context, segment.range)?;
			joined.extend_from_slice(bytes.as_ref());
		}

		Ok(Joined::Copied(joined))
	}
}

/// The bytes of a read that one file served, or the joined bytes of several.
enum Joined<B> {
	Single(B),
	Copied(Vec<u8>),
}

impl<B> AsRef<[u8]> for Joined<B>
where
	B: AsRef<[u8]>,
{
	fn as_ref(&self) -> &[u8] {
		match self {
			Self::Single(bytes) => bytes.as_ref(),
			Self::Copied(bytes) => bytes,
		}
	}
}

fn metadata_size(data_length: u64) -> u64 {
	((data_length / CLUSTER_SIZE + 1) * FAT_ENTRY_SIZE).next_multiple_of(PAGE_SIZE) + PAGE_SIZE
}

fn storage_length(data_length: u64) -> u64 {
	metadata_size(data_length) + data_length
}

fn check_file(index: usize, length: ByteCount, required: ByteCount) -> Result<(), UsbStorageError> {
	if length < required {
		return Err(UsbStorageError::FileTooShort { index, length, required });
	}

	Ok(())
}
