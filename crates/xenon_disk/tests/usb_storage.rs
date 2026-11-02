mod common;

use std::io;
use std::ops::Range;

use common::render;
use xenon_disk::ByteCount;
use xenon_disk::JoinedSource;
use xenon_disk::ReadAt;
use xenon_disk::UsbStorageError;
use xenon_disk::UsbStorageLayout;

const GIB: u64 = 0x4000_0000;

/// A file whose byte at each offset is derived from the file index and the offset, so that a
/// joined read shows which file and offset each byte came from without allocating the file.
#[derive(Debug, Clone, Copy)]
struct PatternFile {
	index: u64,
	length: u64,
}

impl PatternFile {
	fn byte(index: u64, offset: u64) -> u8 {
		(index.wrapping_mul(0x9e) ^ offset.wrapping_mul(31) ^ (offset >> 13)) as u8
	}
}

impl ReadAt for PatternFile {
	fn read_at(&self, _context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		if range.end > self.length {
			return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "past the file"));
		}

		Ok(range.map(|offset| Self::byte(self.index, offset)).collect::<Vec<u8>>())
	}
}

fn lengths(values: &[u64]) -> Vec<ByteCount> {
	values.iter().copied().map(ByteCount::new).collect()
}

#[test]
fn layout_of_two_and_a_half_gib_of_data() {
	let data_length = 2 * GIB + 0x2000_0000;
	let layout = UsbStorageLayout::new(ByteCount::new(data_length)).unwrap();

	let extents: Vec<(usize, u64, u64)> =
		layout.file_extents().iter().map(|extent| (extent.index, extent.start.get(), extent.length.get())).collect();
	assert_eq!(
		extents,
		[
			(0, 0, 0x2000_0000),
			(1, 0x2000_0000, 0xa_2000),
			(2, 0x200a_2000, GIB),
			(3, 0x600a_2000, GIB),
			(4, 0xa00a_2000, 0x2000_0000),
		]
	);
	insta::assert_snapshot!(render(&layout.partition_map()));
}

#[test]
fn layout_from_file_lengths_uses_only_the_needed_bytes() {
	let layout = UsbStorageLayout::from_file_lengths(&lengths(&[0x2000_0000 + 0x400, 0x10_0000, GIB, 0x8000])).unwrap();

	assert_eq!(layout.data_length(), ByteCount::new(GIB + 0x8000));
	assert_eq!(layout.metadata_size(), ByteCount::new(0x42000));
}

#[test]
fn layout_with_no_data_files() {
	let layout = UsbStorageLayout::from_file_lengths(&lengths(&[0x2000_0000, 0x2000])).unwrap();

	assert_eq!(layout.data_length(), ByteCount::new(0));
	assert_eq!(layout.file_count(), 2);
	assert_eq!(layout.partition_map().partition("Storage").unwrap().range(), 0x2000_0000..0x2000_2000);
}

#[test]
fn a_final_data_file_of_one_cluster_is_the_smallest() {
	let layout = UsbStorageLayout::from_file_lengths(&lengths(&[0x2000_0000, 0x2000, 0x4000])).unwrap();

	assert_eq!(layout.data_length(), ByteCount::new(0x4000));
	assert_eq!(layout.file_count(), 3);
}

#[test]
fn a_final_data_file_shorter_than_a_cluster_is_rejected() {
	for (files, index) in [
		(&[0x2000_0000, 0x2000, 0][..], 2),
		(&[0x2000_0000, 0x2000, 0x3fff][..], 2),
		(&[0x2000_0000, 0x6000, GIB, 0][..], 3),
	] {
		assert_eq!(
			UsbStorageLayout::from_file_lengths(&lengths(files)),
			Err(UsbStorageError::FileTooShort {
				index,
				length: ByteCount::new(files[index]),
				required: ByteCount::new(0x4000),
			}),
			"{files:x?}"
		);
	}
}

#[test]
fn file_length_errors() {
	assert_eq!(
		UsbStorageLayout::from_file_lengths(&lengths(&[0x2000_0000])),
		Err(UsbStorageError::TooFewFiles { count: 1 })
	);
	assert_eq!(
		UsbStorageLayout::from_file_lengths(&lengths(&[0x1fff_f000, 0x2000])),
		Err(UsbStorageError::FileTooShort {
			index: 0,
			length: ByteCount::new(0x1fff_f000),
			required: ByteCount::new(0x2000_0000),
		})
	);
	assert_eq!(
		UsbStorageLayout::from_file_lengths(&lengths(&[0x2000_0000, 0x2000, GIB - 0x4000, 0x4000])),
		Err(UsbStorageError::FileTooShort {
			index: 2,
			length: ByteCount::new(GIB - 0x4000),
			required: ByteCount::new(GIB)
		})
	);
	assert_eq!(
		UsbStorageLayout::from_file_lengths(&lengths(&[0x2000_0000, 0x2000, 0x400_0000])),
		Err(UsbStorageError::FileTooShort {
			index: 1,
			length: ByteCount::new(0x2000),
			required: ByteCount::new(0x6000)
		})
	);
	assert_eq!(
		UsbStorageLayout::from_file_lengths(&lengths(&[0x2000_0000, 0x3000, 0x6000])),
		Err(UsbStorageError::UnalignedDataLength { length: ByteCount::new(0x6000) })
	);
	assert_eq!(
		UsbStorageLayout::new(ByteCount::new(33 * GIB)),
		Err(UsbStorageError::DataLengthTooLarge {
			length: ByteCount::new(33 * GIB),
			maximum: ByteCount::new(32 * GIB)
		})
	);
}

#[test]
fn logical_length_gives_back_the_data_length() {
	for data_length in [0, 0x4000, 0x3ffc000, 0x400_0000, GIB, 5 * GIB + 0x1234_4000, 32 * GIB] {
		let layout = UsbStorageLayout::new(ByteCount::new(data_length)).unwrap();

		assert_eq!(UsbStorageLayout::from_logical_length(layout.logical_length()), Ok(layout), "{data_length:#x}");
	}

	let length = ByteCount::new(0x2000_0000 + 0x2000 + 0x200);
	assert_eq!(UsbStorageLayout::from_logical_length(length), Err(UsbStorageError::NotALayoutLength { length }));
}

#[test]
fn joined_reads_cross_file_boundaries() {
	let layout = UsbStorageLayout::new(ByteCount::new(GIB + 0x4000)).unwrap();
	let files: Vec<PatternFile> = layout
		.file_extents()
		.iter()
		.map(|extent| PatternFile { index: extent.index as u64, length: extent.length.get() + 0x1000 })
		.collect();
	let extents = layout.file_extents();
	let joined = JoinedSource::new(layout, files).unwrap();

	let data_start = extents[2].start.get();
	let range = data_start - 0x10..data_start + GIB + 0x10;
	let read = |range: Range<u64>| joined.read_at(&(), range).unwrap().as_ref().to_vec();

	let crossing = read(range.start..range.start + 0x20);
	let expected: Vec<u8> = (0..0x10)
		.map(|offset| PatternFile::byte(1, extents[1].length.get() - 0x10 + offset))
		.chain((0..0x10).map(|offset| PatternFile::byte(2, offset)))
		.collect();
	assert_eq!(crossing, expected);

	let tail = read(range.end - 0x20..range.end);
	let expected: Vec<u8> = (0..0x10)
		.map(|offset| PatternFile::byte(2, GIB - 0x10 + offset))
		.chain((0..0x10).map(|offset| PatternFile::byte(3, offset)))
		.collect();
	assert_eq!(tail, expected);

	let end = layout.logical_length().get();
	assert!(joined.read_at(&(), end - 1..end + 1).is_err());
}

#[test]
fn joining_needs_every_file() {
	let layout = UsbStorageLayout::new(ByteCount::new(0x4000)).unwrap();

	assert_eq!(
		JoinedSource::new(layout, vec![PatternFile { index: 0, length: 0x2000_0000 }]).unwrap_err(),
		UsbStorageError::FileCount { expected: 3, actual: 1 }
	);
}
