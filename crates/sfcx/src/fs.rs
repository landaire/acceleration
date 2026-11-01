//! The flash file system of the observed 60 MiB devkit profile: a root directory cluster that
//! also holds the cluster allocation table, and files stored as cluster chains.

use std::ops::Range;
use std::str;

use crate::boot::BOOT_STAGE_COUNT;
use crate::boot::BootRegion;
use crate::error::ParseError;

pub const DEVKIT_LOGICAL_SIZE: u64 = 0x03c0_0000;
pub const DEVKIT_FLASH_DATA_OFFSET: u64 = 0x00b8_0000;
pub const DEVKIT_CLUSTER_SIZE: usize = 0x4000;
pub const DEVKIT_FIRST_FILE_CLUSTER: u16 = 0x0010;
pub const DEVKIT_ROOT_CLUSTER: u16 = 0x0635;
pub const DIRECTORY_ENTRY_SIZE: usize = 0x20;
pub const DIRECTORY_NAME_SIZE: usize = 22;
pub const FAT_END_OF_CHAIN: u16 = 0x1fff;
pub const FAT_FREE: u16 = 0x1ffe;
pub const FAT_LINK_MASK: u16 = 0x1fff;
const DIRECTORY_PAGE_SIZE: usize = 0x200;
const DIRECTORY_PAGE_PAIR_SIZE: usize = 0x400;
const FAT_ENTRIES_PER_PAGE: usize = DIRECTORY_PAGE_SIZE / 2;
pub const DIRECTORY_SLOT_COUNT: usize =
	(DEVKIT_CLUSTER_SIZE / DIRECTORY_PAGE_PAIR_SIZE) * (DIRECTORY_PAGE_SIZE / DIRECTORY_ENTRY_SIZE);
const EXPECTED_IMAGE_BUILD: u16 = 16547;
const EXPECTED_IMAGE_FLAGS: u32 = 0x8000_0000;
const EXPECTED_IMAGE_ENTRY: u32 = 0x0000_8000;
const EXPECTED_BOOT_IMAGE_SIZE: usize = 0x000e_0000;
const EXPECTED_STAGE_BUILDS: [u16; BOOT_STAGE_COUNT] = [14352, 16547, 16547, 16547];
const EXPECTED_STAGE_FLAGS: [u32; BOOT_STAGE_COUNT] = [0, 0x8000_0000, 0x8000_0000, 0x8000_0000];
const EXPECTED_STAGE_ENTRIES: [u32; BOOT_STAGE_COUNT] = [0x3d0, 0x120, 0x310, 0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClusterId(u16);

impl ClusterId {
	pub const fn new(value: u16) -> Self {
		Self(value)
	}

	pub const fn get(self) -> u16 {
		self.0
	}

	/// Logical offset of the cluster's data.
	pub const fn offset(self) -> u64 {
		DEVKIT_FLASH_DATA_OFFSET + self.0 as u64 * DEVKIT_CLUSTER_SIZE as u64
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
	name: String,
	start_cluster: ClusterId,
	size: u64,
	timestamp: u32,
	/// Logical offset of the 32-byte directory record.
	record_offset: u64,
}

impl DirectoryEntry {
	pub fn name(&self) -> &str {
		&self.name
	}

	pub const fn start_cluster(&self) -> ClusterId {
		self.start_cluster
	}

	pub const fn size(&self) -> u64 {
		self.size
	}

	pub const fn timestamp(&self) -> u32 {
		self.timestamp
	}

	pub const fn record_offset(&self) -> u64 {
		self.record_offset
	}
}

/// The root directory and allocation table of the observed devkit profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filesystem {
	entries: Vec<DirectoryEntry>,
	root: Vec<u8>,
}

impl Filesystem {
	/// Logical range of the root directory cluster.
	pub const fn root_range() -> Range<u64> {
		let start = ClusterId(DEVKIT_ROOT_CLUSTER).offset();
		start..start + DEVKIT_CLUSTER_SIZE as u64
	}

	/// Checks that the image matches the observed profile and parses its root directory
	/// cluster, the bytes of [`Filesystem::root_range`].
	pub fn parse(boot_region: &BootRegion<'_>, logical_size: u64, root: Vec<u8>) -> Result<Self, ParseError> {
		if logical_size != DEVKIT_LOGICAL_SIZE {
			return Err(ParseError::UnsupportedImageSize { actual: logical_size });
		}
		validate_profile(boot_region)?;
		if root.len() != DEVKIT_CLUSTER_SIZE {
			return Err(ParseError::RootDirectorySize { expected: DEVKIT_CLUSTER_SIZE, actual: root.len() });
		}

		let entries = (0..DIRECTORY_SLOT_COUNT)
			.filter_map(|slot| {
				let offset = directory_slot_offset(slot);
				let record = &root[offset..offset + DIRECTORY_ENTRY_SIZE];
				let empty = record.iter().all(|byte| *byte == 0);
				(!empty).then(|| parse_directory_entry(record, Self::root_range().start + offset as u64))
			})
			.collect::<Result<Vec<_>, _>>()?;
		if entries.is_empty() {
			return Err(ParseError::EmptyRootDirectory);
		}

		Ok(Self { entries, root })
	}

	pub fn entries(&self) -> &[DirectoryEntry] {
		&self.entries
	}

	pub fn find(&self, name: &str) -> Option<&DirectoryEntry> {
		self.entries.iter().find(|entry| entry.name.eq_ignore_ascii_case(name))
	}

	pub fn allocation_entry(&self, cluster: ClusterId) -> Result<u16, ParseError> {
		let index = cluster.0 as usize;
		let offset = index / FAT_ENTRIES_PER_PAGE * DIRECTORY_PAGE_PAIR_SIZE + index % FAT_ENTRIES_PER_PAGE * 2;
		let bytes =
			self.root.get(offset..offset + 2).ok_or(ParseError::AllocationEntryOutOfBounds { cluster: cluster.0 })?;

		Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
	}

	/// The logical ranges that hold the file's data, in order, with adjacent clusters merged.
	pub fn file_ranges(&self, entry: &DirectoryEntry) -> Result<Vec<Range<u64>>, ParseError> {
		const FILE_CLUSTERS: usize = (DEVKIT_ROOT_CLUSTER - DEVKIT_FIRST_FILE_CLUSTER) as usize;

		let mut ranges: Vec<Range<u64>> = Vec::new();
		let mut cluster = entry.start_cluster;
		let mut remaining = entry.size;
		let mut steps = 0usize;
		while remaining != 0 {
			if steps >= FILE_CLUSTERS {
				return Err(ParseError::ClusterChainCycle);
			}
			validate_file_cluster(cluster)?;

			let len = remaining.min(DEVKIT_CLUSTER_SIZE as u64);
			let start = cluster.offset();
			match ranges.last_mut() {
				Some(last) if last.end == start => last.end += len,
				_ => ranges.push(start..start + len),
			}
			remaining -= len;
			steps += 1;

			let next = self.allocation_entry(cluster)?;
			let link = next & FAT_LINK_MASK;
			if remaining == 0 {
				if link != FAT_END_OF_CHAIN {
					return Err(ParseError::ClusterChainNotTerminated { cluster: cluster.0, next });
				}
				break;
			}
			cluster = match link {
				FAT_END_OF_CHAIN => return Err(ParseError::TruncatedClusterChain),
				FAT_FREE => return Err(ParseError::FreeClusterInChain { cluster: cluster.0 }),
				value => ClusterId(value),
			};
		}

		Ok(ranges)
	}
}

fn validate_profile(boot_region: &BootRegion<'_>) -> Result<(), ParseError> {
	let header = boot_region.header();
	validate_profile_value("image build", EXPECTED_IMAGE_BUILD.into(), header.build.get().into())?;
	validate_profile_value("image flags", EXPECTED_IMAGE_FLAGS.into(), header.flags.into())?;
	validate_profile_value("image entry", EXPECTED_IMAGE_ENTRY.into(), header.entry.into())?;
	validate_profile_value("boot image size", EXPECTED_BOOT_IMAGE_SIZE as u64, header.boot_image_size.get() as u64)?;
	for (index, stage) in boot_region.boot_stages().iter().enumerate() {
		validate_profile_value("boot stage build", EXPECTED_STAGE_BUILDS[index].into(), stage.build().get().into())?;
		validate_profile_value("boot stage flags", EXPECTED_STAGE_FLAGS[index].into(), stage.flags().into())?;
		validate_profile_value("boot stage entry", EXPECTED_STAGE_ENTRIES[index].into(), stage.entry().into())?;
	}

	Ok(())
}

fn validate_profile_value(field: &'static str, expected: u64, actual: u64) -> Result<(), ParseError> {
	if actual != expected {
		return Err(ParseError::UnsupportedProfileValue { field, expected, actual });
	}

	Ok(())
}

/// Offset of a directory slot in the root cluster: the second page of each page pair holds
/// directory records.
const fn directory_slot_offset(slot: usize) -> usize {
	let slots_per_page = DIRECTORY_PAGE_SIZE / DIRECTORY_ENTRY_SIZE;
	slot / slots_per_page * DIRECTORY_PAGE_PAIR_SIZE
		+ DIRECTORY_PAGE_SIZE
		+ slot % slots_per_page * DIRECTORY_ENTRY_SIZE
}

fn parse_directory_entry(record: &[u8], record_offset: u64) -> Result<DirectoryEntry, ParseError> {
	let offset = record_offset as usize;
	let name_field = &record[..DIRECTORY_NAME_SIZE];
	let name_length = name_field.iter().position(|byte| *byte == 0).unwrap_or(DIRECTORY_NAME_SIZE);
	let name_bytes = &name_field[..name_length];
	if name_bytes.is_empty()
		|| !name_field[name_length..].iter().all(|byte| *byte == 0)
		|| !name_bytes.iter().all(|byte| byte.is_ascii_graphic() && *byte != b'/' && *byte != b'\\')
	{
		return Err(ParseError::InvalidDirectoryName { offset });
	}
	let name = str::from_utf8(name_bytes).map_err(|_| ParseError::InvalidDirectoryName { offset })?;
	let start_cluster = ClusterId(u16::from_be_bytes([record[22], record[23]]));
	let size = u32::from_be_bytes([record[24], record[25], record[26], record[27]]).into();
	if size == 0 {
		return Err(ParseError::EmptyFile { offset });
	}
	let timestamp = u32::from_be_bytes([record[28], record[29], record[30], record[31]]);

	Ok(DirectoryEntry { name: name.to_owned(), start_cluster, size, timestamp, record_offset })
}

fn validate_file_cluster(cluster: ClusterId) -> Result<(), ParseError> {
	if cluster.0 < DEVKIT_FIRST_FILE_CLUSTER || cluster.0 >= DEVKIT_ROOT_CLUSTER {
		return Err(ParseError::FileClusterOutOfRange { cluster: cluster.0 });
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn directory_entry_rejects_non_ascii_name() {
		let mut record = [0u8; DIRECTORY_ENTRY_SIZE];
		record[0] = 0xff;
		assert_eq!(parse_directory_entry(&record, 0x100), Err(ParseError::InvalidDirectoryName { offset: 0x100 }));
	}

	#[test]
	fn directory_entry_rejects_zero_length_file() {
		let mut record = [0u8; DIRECTORY_ENTRY_SIZE];
		record[..8].copy_from_slice(b"file.bin");
		assert_eq!(parse_directory_entry(&record, 0x100), Err(ParseError::EmptyFile { offset: 0x100 }));
	}
}
