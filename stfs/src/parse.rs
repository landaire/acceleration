use binrw::binrw;
use binrw::BinReaderExt;
use binrw::NullString;
use binrw::NullWideString;
use modular_bitfield::prelude::*;
use std::collections::HashMap;
use std::io::Read;
use std::io::Write;
use std::ops::Range;
use std::ops::{
	self,
};
use std::sync::Arc;

use crate::consts::*;
use bitflags::bitflags;
use chrono::DateTime;
use chrono::Utc;
use parking_lot::Mutex;
use serde::Deserialize;
use serde::Serialize;
use serde::Serializer;
use std::io::Cursor;
use thiserror::Error;
use variantly::Variantly;

use crate::error::StfsError;
use crate::util::*;

pub type StfsEntryRef = Arc<Mutex<StfsEntry>>;

const BLOCK_SIZE: usize = 0x1000;

#[derive(Debug, Serialize, Variantly)]
pub enum StfsEntry {
	File(StfsFileEntry, Vec<Range<u64>>),
	Folder { entry: StfsFileEntry, files: Vec<StfsEntryRef> },
}

impl StfsEntry {
	pub fn name(&self) -> String {
		self.entry().name.to_string()
	}

	pub fn entry(&self) -> &StfsFileEntry {
		match self {
			StfsEntry::File(entry, _) | StfsEntry::Folder { entry, files: _ } => entry,
		}
	}

	pub fn file_ranges(&self) -> Option<&[Range<u64>]> {
		if let StfsEntry::File(_, ranges) = self {
			Some(ranges.as_slice())
		} else {
			None
		}
	}
}

#[derive(Debug, Serialize, Copy, Clone)]
pub enum StfsPackageReadFlag {
	ReadWrite = 0,
	ReadOnly,
}

impl StfsPackageReadFlag {
	/// The "block step" depends on the package's "sex". This basically determines
	/// which hash tables are used.
	const fn block_step(&self) -> [usize; 2] {
		match self {
			StfsPackageReadFlag::ReadWrite => [0xAB, 0x718F],
			StfsPackageReadFlag::ReadOnly => [0xAC, 0x723A],
		}
	}
}

#[derive(Default, Debug, Serialize)]
#[binrw]
struct HashEntry {
	block_hash: [u8; 0x14],
	status: u8,
	next_block: Block,
}

#[derive(Default, Debug, Serialize)]
pub struct HashTableMeta {
	pub block_step: [usize; 2],
	pub tables_per_level: [usize; 3],
	pub top_table: HashTable,
}

impl HashTableMeta {
	pub fn from_volume_descriptor(stfs_vol: &StfsVolumeDescriptor) -> Result<Self, StfsError> {
		let mut meta = HashTableMeta::default();

		meta.block_step = stfs_vol.sex().block_step();

		let allocated_block_count = stfs_vol.allocated_block_count as usize;
		meta.tables_per_level[0] = ((allocated_block_count as usize) / HASHES_PER_BLOCK)
			+ if (allocated_block_count as usize) % HASHES_PER_BLOCK != 0 { 1 } else { 0 };

		meta.tables_per_level[1] = (meta.tables_per_level[1] / HASHES_PER_BLOCK)
			+ if meta.tables_per_level[1] % HASHES_PER_BLOCK != 0 && allocated_block_count > HASHES_PER_BLOCK {
				1
			} else {
				0
			};

		meta.tables_per_level[2] = (meta.tables_per_level[2] / HASHES_PER_BLOCK)
			+ if meta.tables_per_level[2] % HASHES_PER_BLOCK != 0
				&& allocated_block_count > DATA_BLOCKS_PER_HASH_TREE_LEVEL_TEMP[2]
			{
				1
			} else {
				0
			};

		meta.top_table.level = stfs_vol.root_hash_table_level()?;
		meta.top_table.true_block_number =
			meta.compute_backing_hash_block_number_for_level(Block(0), meta.top_table.level, stfs_vol.sex());

		let base_address = meta.top_table.true_block_number.0 * BLOCK_SIZE;
		meta.top_table.address_in_file = base_address + ((stfs_vol.flags.root_active_index() as usize) << 0xC);

		meta.top_table.entry_count =
			(allocated_block_count as usize) / DATA_BLOCKS_PER_HASH_TREE_LEVEL_TEMP[meta.top_table.level as usize];

		if (allocated_block_count > DATA_BLOCKS_PER_HASH_TREE_LEVEL_TEMP[2]
			&& allocated_block_count % DATA_BLOCKS_PER_HASH_TREE_LEVEL_TEMP[2] != 0)
			|| (allocated_block_count > HASHES_PER_BLOCK && allocated_block_count % HASHES_PER_BLOCK != 0)
		{
			meta.top_table.entry_count += 1;
		}

		meta.top_table.entries.reserve(meta.top_table.entry_count);

		Ok(meta)
	}

	pub fn compute_backing_hash_block_number_for_level(
		&self,
		block: Block,
		level: HashTableLevel,
		sex: StfsPackageReadFlag,
	) -> Block {
		match level {
			HashTableLevel::First => self.compute_first_level_backing_hash_block_number(block, sex),
			HashTableLevel::Second => self.compute_second_level_backing_hash_block_number(block, sex),
			HashTableLevel::Third => self.compute_third_level_backing_hash_block_number(),
		}
	}

	pub fn compute_first_level_backing_hash_block_number(&self, block: Block, sex: StfsPackageReadFlag) -> Block {
		if block.0 < HASHES_PER_BLOCK {
			return Block(0);
		}

		let mut block_number = (block.0 / HASHES_PER_BLOCK) * self.block_step[0];
		block_number += ((block.0 / DATA_BLOCKS_PER_HASH_TREE_LEVEL_TEMP[2]) + 1) << (sex as u8);

		let block = if block.0 / DATA_BLOCKS_PER_HASH_TREE_LEVEL_TEMP[2] == 0 {
			block_number
		} else {
			block_number + (1 << (sex as u8))
		};

		block.into()
	}

	pub fn compute_second_level_backing_hash_block_number(&self, block: Block, sex: StfsPackageReadFlag) -> Block {
		let block = if block.0 < DATA_BLOCKS_PER_HASH_TREE_LEVEL_TEMP[2] {
			self.block_step[0]
		} else {
			(1 << (sex as u8)) + (block.0 / DATA_BLOCKS_PER_HASH_TREE_LEVEL_TEMP[2]) * self.block_step[1]
		};

		block.into()
	}

	pub fn compute_third_level_backing_hash_block_number(&self) -> Block {
		self.block_step[1].into()
	}
}

#[derive(Debug, Serialize)]
pub struct StfsPackage {
	pub volume_descriptor: StfsVolumeDescriptor,
	pub hash_table_meta: HashTableMeta,
	pub files: StfsEntryRef,
}

#[derive(Default, Debug, Serialize, Copy, Clone)]
#[binrw]
pub struct Block(
	#[br(parse_with = binrw::helpers::read_u24, map = |block: u32| block as usize)]
	// TODO: write u24
	#[bw(map = |block: &usize| *block as u32 )] //, write_with = binrw::helpers::write_u24)]
	usize,
);

impl From<usize> for Block {
	fn from(value: usize) -> Self {
		Block(value)
	}
}

impl ops::Add<Block> for Block {
	type Output = Block;

	fn add(self, rhs: Block) -> Self::Output {
		Block(self.0 + rhs.0)
	}
}

impl ops::Add<usize> for Block {
	type Output = Block;

	fn add(self, rhs: usize) -> Self::Output {
		Block(self.0 + rhs)
	}
}

impl ops::Mul<usize> for Block {
	type Output = Block;

	fn mul(self, rhs: usize) -> Self::Output {
		Block(self.0 * rhs)
	}
}

#[derive(Copy, Clone, Debug)]
enum HashBlockTable {
	Level0,
	Level1,
	Level2,
}

impl StfsPackage {
	pub fn from_volume_descriptor(volume_descriptor: StfsVolumeDescriptor, input: &[u8]) -> Result<Self, StfsError> {
		let mut hash_table_meta = HashTableMeta::from_volume_descriptor(&volume_descriptor)?;
		hash_table_meta.top_table.parse_hash_entries(&input[hash_table_meta.top_table.data_range()])?;

		let mut package = Self {
			volume_descriptor,
			hash_table_meta,
			files: Arc::new(Mutex::new(StfsEntry::Folder { entry: Default::default(), files: Default::default() })),
		};

		package.read_files(input)?;

		Ok(package)
	}

	fn compute_hash_block_number(&self, block: Block, table_level: HashBlockTable) -> usize {
		const BLOCKS_FOR_LEVEL_0_HASH_TREE_READ_ONLY: usize = HASHES_PER_BLOCK + 1;
		const BLOCKS_FOR_LEVEL_1_HASH_TREE_READ_ONLY: usize =
			(BLOCKS_FOR_LEVEL_0_HASH_TREE_READ_ONLY * BLOCKS_FOR_LEVEL_0_HASH_TREE_READ_ONLY) + 1;
		const BLOCKS_FOR_LEVEL_2_HASH_TREE_READ_ONLY: usize =
			(BLOCKS_FOR_LEVEL_1_HASH_TREE_READ_ONLY * BLOCKS_FOR_LEVEL_1_HASH_TREE_READ_ONLY) + 1;

		const BLOCKS_FOR_LEVEL_0_HASH_TREE_READ_WRITE: usize = HASHES_PER_BLOCK + 2;
		const BLOCKS_FOR_LEVEL_1_HASH_TREE_READ_WRITE: usize =
			(BLOCKS_FOR_LEVEL_0_HASH_TREE_READ_WRITE * BLOCKS_FOR_LEVEL_0_HASH_TREE_READ_WRITE) + 2;
		const BLOCKS_FOR_LEVEL_2_HASH_TREE_READ_WRITE: usize =
			(BLOCKS_FOR_LEVEL_1_HASH_TREE_READ_WRITE * BLOCKS_FOR_LEVEL_1_HASH_TREE_READ_WRITE) + 2;

		if self.volume_descriptor.is_read_only() {
			match table_level {
				HashBlockTable::Level0 => BLOCKS_FOR_LEVEL_0_HASH_TREE_READ_ONLY,
				HashBlockTable::Level1 => BLOCKS_FOR_LEVEL_1_HASH_TREE_READ_ONLY,
				HashBlockTable::Level2 => BLOCKS_FOR_LEVEL_2_HASH_TREE_READ_ONLY,
			}
		} else {
			match table_level {
				HashBlockTable::Level0 => BLOCKS_FOR_LEVEL_0_HASH_TREE_READ_WRITE,
				HashBlockTable::Level1 => BLOCKS_FOR_LEVEL_1_HASH_TREE_READ_WRITE,
				HashBlockTable::Level2 => BLOCKS_FOR_LEVEL_2_HASH_TREE_READ_WRITE,
			}
		}
	}

	fn file_ranges(&self, entry: &StfsFileEntry, input: &[u8]) -> Result<Vec<Range<u64>>, StfsError> {
		let mut mappings = Vec::new();
		if entry.file_attributes.is_none() {
			return Ok(Vec::new());
		}

		let attributes = entry.file_attributes.as_ref().unwrap();
		if attributes.file_size == 0 {
			return Ok(Vec::new());
		}

		let start_address = self.block_to_addr(attributes.starting_block);

		let mut next_address = start_address;
		let mut data_remaining = attributes.file_size as u64;

		// Check if we can read consecutive blocks
		if entry.flags.has_consecutive_blocks() {
			let blocks_until_hash_table = (self
				.hash_table_meta
				.compute_first_level_backing_hash_block_number(attributes.starting_block, self.volume_descriptor.sex())
				.0 + self.hash_table_meta.block_step[0])
				- ((start_address as usize) / BLOCK_SIZE);

			if attributes.block_count as usize <= blocks_until_hash_table {
				mappings.push(start_address..(start_address + attributes.file_size as u64));
			} else {
				// The file is broken up by hash tables
				while data_remaining > 0 {
					let read_len = std::cmp::min(HASHES_PER_BLOCK * BLOCK_SIZE, data_remaining as usize) as u64;

					let range = next_address..(next_address + read_len);
					mappings.push(range.clone());

					let data_read = range.end - range.start;
					data_remaining -= data_read;
					next_address += data_read;
					next_address += self.hash_table_skip_for_address(next_address as usize)? as u64;
				}
			}
		} else {
			let mut data_remaining = attributes.file_size as u64;

			// This file does not have all-consecutive blocks
			let mut block_count = data_remaining / (BLOCK_SIZE as u64);
			if data_remaining % (BLOCK_SIZE as u64) != 0 {
				block_count += 1;
			}

			let mut block = attributes.starting_block;
			for _ in 0..block_count {
				let read_len = std::cmp::min(BLOCK_SIZE as u64, data_remaining);

				let block_address = self.block_to_addr(block);
				mappings.push(block_address..(block_address + read_len));

				let hash_entry = self.block_hash_entry(block, input)?;
				block = hash_entry.next_block;
				data_remaining -= read_len;
			}
		}

		Ok(mappings)
	}

	fn hash_table_skip_for_address(&self, table_address: usize) -> Result<usize, StfsError> {
		let sex = self.volume_descriptor.sex() as usize;
		let hash_table_meta = &self.hash_table_meta;

		// Convert the address to a true block number
		let mut block_number = table_address / BLOCK_SIZE;

		// Check if it's the first hash table
		if block_number == 0 {
			return Ok(BLOCK_SIZE << sex);
		}

		// Check if it's the level 3 or above table
		if block_number == hash_table_meta.block_step[1] {
			return Ok((BLOCK_SIZE * 3) << sex);
		} else if block_number > hash_table_meta.block_step[1] {
			block_number -= hash_table_meta.block_step[1] + (1 << sex);
		}

		// Check if it's at a level 2 table
		if block_number == hash_table_meta.block_step[0] || block_number % hash_table_meta.block_step[1] == 0 {
			Ok((BLOCK_SIZE * 2) << sex)
		} else {
			// Assume it's the level 0 table
			Ok(BLOCK_SIZE << sex)
		}
	}

	fn block_hash_entry(&self, block: Block, input: &[u8]) -> Result<HashEntry, StfsError> {
		if block.0 > self.volume_descriptor.allocated_block_count as usize {
			panic!(
				"Reference to illegal block number: {:#x} ({:#x} allocated)",
				block.0, self.volume_descriptor.allocated_block_count
			);
		}

		let mut reader = Cursor::new(input);
		reader.set_position(self.block_hash_address(block, input)?);
		Ok(reader.read_be::<HashEntry>()?)
	}

	fn block_hash_address(&self, block: Block, input: &[u8]) -> Result<u64, StfsError> {
		if block.0 > self.volume_descriptor.allocated_block_count as usize {
			panic!(
				"Reference to illegal block number: {:#x} ({:#x} allocated)",
				block.0, self.volume_descriptor.allocated_block_count
			);
		}

		let hash_table_meta = &self.hash_table_meta;

		let mut hash_addr = hash_table_meta
			.compute_first_level_backing_hash_block_number(block, self.volume_descriptor.sex())
			.0 * BLOCK_SIZE;
		// 0x18 here is the size of the HashEntry structure
		hash_addr += (block.0 % HASHES_PER_BLOCK) * 0x18;
		let address = match hash_table_meta.top_table.level {
			// TODO: might have broken things with the flags here
			HashTableLevel::First => {
				hash_addr as u64 + ((self.volume_descriptor.flags.root_active_index() as u64) << 0xC)
			}
			HashTableLevel::Second => {
				hash_addr as u64
					+ ((hash_table_meta.top_table.entries[block.0 / DATA_BLOCKS_PER_HASH_TREE_LEVEL_TEMP[1]].status
						as u64 & 0x40) << 6)
			}
			HashTableLevel::Third => {
				let first_level_offset =
					(hash_table_meta.top_table.entries[block.0 / DATA_BLOCKS_PER_HASH_TREE_LEVEL_TEMP[2]].status
						as u64 & 0x40) << 6;

				let position = (hash_table_meta
					.compute_second_level_backing_hash_block_number(block, self.volume_descriptor.sex())
					.0 * BLOCK_SIZE) + first_level_offset as usize
					+ ((block.0 % DATA_BLOCKS_PER_HASH_TREE_LEVEL_TEMP[1]) * 0x18);

				let status_byte = input[position + 0x14];
				hash_addr as u64 + ((status_byte as u64 & 0x40) << 0x6)
			}
		};

		Ok(address)
	}

	fn read_files(&mut self, input: &[u8]) -> Result<(), StfsError> {
		let mut reader = Cursor::new(input);
		let mut block = self.volume_descriptor.file_table_block_num;
		let mut folders = HashMap::<u16, StfsEntryRef>::new();
		let mut files = Vec::new();
		// Inject a fake root folder
		folders.insert(
			0xffff,
			Arc::new(Mutex::new(StfsEntry::Folder { entry: StfsFileEntry::default(), files: Vec::new() })),
		);

		for block_idx in 0..(self.volume_descriptor.file_table_block_count as usize) {
			let current_addr = self.block_to_addr(block + block_idx);
			reader.set_position(current_addr);

			for file_entry_idx in 0..0x40 {
				let addressing_info = StfsFileEntryAddressingInfo {
					file_entry_address: current_addr + (file_entry_idx as u64 * 0x40),
					file_table_index: (block_idx * 0x40) + file_entry_idx,
				};

				let mut entry = reader.read_be::<StfsFileEntry>()?;

				// If we encounter a NULL name, that signifies that we've reached the end of the file table
				if entry.flags.name_len() == 0 {
					// Continue to the next entry -- this one was stomped over
					break;
				}

				let file_ranges = self.file_ranges(&entry, input)?;

				let file_table_index = addressing_info.file_table_index;
				entry.addressing_info = addressing_info;
				if entry.flags.is_folder() {
					let entry_idx = file_table_index;
					let folder = Arc::new(Mutex::new(StfsEntry::Folder { entry, files: Vec::new() }));
					folders.insert(entry_idx as u16, folder.clone());
					files.push(folder.clone());
				} else {
					files.push(Arc::new(Mutex::new(StfsEntry::File(entry, file_ranges))));
				}
			}

			block = self.block_hash_entry(block.into(), input)?.next_block;
		}

		// Associate each file with the folder it needs to be in
		for file in files.drain(..) {
			let file_lock = file.lock();
			let entry = file_lock.entry();

			if let Some(attributes) = entry.file_attributes.as_ref() {
				let cached_entry = folders.get(&attributes.dirent);
				if let Some(entry) = cached_entry {
					if let StfsEntry::Folder { entry: _, files } = &mut *entry.lock() {
						files.push(Arc::clone(&file));
					}
				} else {
					panic!("Corrupt STFS file: missing folder dirent {:#x}", attributes.dirent);
				}
			}
		}

		self.files = folders.remove(&0xffff).expect("no root file entry");

		Ok(())
	}

	fn block_to_addr(&self, block: Block) -> u64 {
		if block.0 > 2usize.pow(24) - 1 {
			panic!("invalid block: {:#x}", block.0);
		}

		(self.compute_data_block_num(block) * BLOCK_SIZE as u64)
	}

	/// Translates a data block to an absolute block, adjusting the block number to skip over any potential hash blocks.
	fn compute_data_block_num(&self, block: Block) -> u64 {
		// // Read-only filesystems have different properties
		let blocks_per_hash_block = if self.volume_descriptor.is_read_only() { 1 } else { 2 };

		let mut block_num = block.0;
		let mut num_hash_and_data_blocks =
			(block_num + DATA_BLOCKS_PER_HASH_TREE_LEVEL[0]) / DATA_BLOCKS_PER_HASH_TREE_LEVEL[0];
		block_num += num_hash_and_data_blocks * blocks_per_hash_block;

		if block_num >= DATA_BLOCKS_PER_HASH_TREE_LEVEL[0] {
			// Skip past the level 0 hash table
			num_hash_and_data_blocks =
				(block_num + DATA_BLOCKS_PER_HASH_TREE_LEVEL[1]) / DATA_BLOCKS_PER_HASH_TREE_LEVEL[1];
			block_num += num_hash_and_data_blocks * blocks_per_hash_block;

			// Skip past the level 1 hash table
			if block_num >= DATA_BLOCKS_PER_HASH_TREE_LEVEL[1] {
				block_num += blocks_per_hash_block;
			}
		}

		u64::try_from(block_num).expect("failed to convert usize to u64")
	}
}

#[derive(Default, Clone, Debug, Serialize)]
pub struct StfsFileEntryAddressingInfo {
	pub file_table_index: usize,
	pub file_entry_address: u64,
}

#[bitfield]
#[binrw]
#[br(map = |x: u32| {Self::from(x)} )]
#[bw(map = |ts: &Self| u32::from(*ts))]
#[derive(Default, Debug, Copy, Clone, Serialize, Eq, PartialEq)]
#[repr(u32)]
pub struct StfTimestamp {
	pub seconds: B5,
	pub minute: B6,
	pub hour: B5,
	pub day: B5,
	pub month: B4,
	pub year: B7,
}

#[derive(Default, Clone, Debug, Serialize)]
#[binrw]
pub struct StfsFileAttributes {
	#[br(parse_with = binrw::helpers::read_u24)]
	#[bw(write_with = binrw::helpers::write_u24 )]
	#[brw(little)]
	pub block_count: u32,

	#[br(parse_with = binrw::helpers::read_u24)]
	#[bw(write_with = binrw::helpers::write_u24 )]
	#[brw(little)]
	pub allocation_block_count: u32,

	#[brw(little)]
	pub starting_block: Block,

	pub dirent: u16,
	pub file_size: u32,
	pub created_time_stamp: StfTimestamp,
	pub access_time_stamp: StfTimestamp,
}

#[derive(Default, Clone, Debug, Serialize)]
#[binrw]
pub struct StfsFileEntry {
	#[brw(ignore)]
	pub addressing_info: StfsFileEntryAddressingInfo,

	#[brw(pad_size_to = 0x28)]
	#[serde(serialize_with = "serialize_null_string")]
	pub name: NullString,
	pub flags: StfsEntryFlags,

	#[br(if(flags.name_len() > 0))]
	pub file_attributes: Option<StfsFileAttributes>,
}

#[bitfield]
#[binrw]
#[br(map = Self::from_bytes)]
#[bw(map = |flags: &Self| flags.into_bytes())]
#[derive(Default, Debug, Copy, Clone, Serialize)]
pub struct StfsEntryFlags {
	name_len: B6,
	has_consecutive_blocks: bool,
	is_folder: bool,
}

#[derive(Debug, Serialize)]
pub struct HashTable {
	level: HashTableLevel,
	true_block_number: Block,
	entry_count: usize,
	address_in_file: usize,
	entries: Vec<HashEntry>,
}

impl Default for HashTable {
	fn default() -> Self {
		HashTable {
			level: HashTableLevel::First,
			true_block_number: Block(0),
			entry_count: 0,
			address_in_file: 0,
			entries: Vec::default(),
		}
	}
}

impl HashTable {
	/// Reads top-level hashes
	pub fn parse_hash_entries(&mut self, data: &[u8]) -> Result<(), StfsError> {
		let mut reader = Cursor::new(data);

		for _ in 0..self.entry_count {
			let entry = reader.read_be::<HashEntry>()?;
			self.entries.push(entry);
		}

		Ok(())
	}

	/// Returns the file range (start..end offset) this hash table occupies
	pub fn data_range(&self) -> Range<usize> {
		// HashEntry has 1 u24 field, so subtract 1 since it's represented internally as a u32
		self.address_in_file..(self.address_in_file + self.entry_count * (std::mem::size_of::<HashEntry>() - 1))
	}
}

#[derive(Debug, Serialize, Copy, Clone)]
pub enum HashTableLevel {
	First,
	Second,
	Third,
}

impl StfsVolumeDescriptor {
	/// Returns which hash table level the root hash is in
	fn root_hash_table_level(&self) -> Result<HashTableLevel, StfsError> {
		let level = if self.allocated_block_count as usize <= HASHES_PER_BLOCK {
			HashTableLevel::First
		} else if self.allocated_block_count as usize <= DATA_BLOCKS_PER_HASH_TREE_LEVEL[1] {
			HashTableLevel::Second
		} else if self.allocated_block_count as usize <= DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] {
			HashTableLevel::Third
		} else {
			return Err(StfsError::InvalidVolumeDescriptor);
		};

		Ok(level)
	}

	pub fn is_read_only(&self) -> bool {
		self.flags.read_only()
	}

	pub fn sex(&self) -> StfsPackageReadFlag {
		if self.is_read_only() {
			StfsPackageReadFlag::ReadOnly
		} else {
			StfsPackageReadFlag::ReadWrite
		}
	}
}

#[bitfield]
#[binrw]
#[br(map = |x: u8| Self::from(x))]
#[bw(map = |flags: &Self| u8::from(*flags))]
#[derive(Default, Debug, Copy, Clone, Serialize)]
#[repr(u8)]
pub struct StfsVolumeDescriptorFlags {
	read_only: bool,
	root_active_index: bool,
	dir_is_overallocated: bool,
	dir_index_bounds_are_valid: bool,
	_reserved: B4,
}

#[derive(Default, Clone, Debug, Serialize)]
#[binrw]
pub struct StfsVolumeDescriptor {
	size: u8,
	version: u8,
	flags: StfsVolumeDescriptorFlags,
	#[brw(little)]
	file_table_block_count: u16,
	#[brw(little)]
	file_table_block_num: Block,
	top_hash_table_hash: [u8; 0x14],
	allocated_block_count: u32,
	unallocated_block_count: u32,
}

#[cfg(test)]
mod tests {
	use super::*;

	fn test_date() -> (u32, StfTimestamp) {
		let u32_value = 0b0011_0101_1000_0101_1000_1011_1001_1101;
		(
			u32_value,
			StfTimestamp::new()
				.with_year(((u32_value & 0xFE000000) >> 25) as u8) // 7 bits
				.with_month(((u32_value & 0x1E00000) >> 21) as u8) // 4 bits
				.with_day(((u32_value & 0x1F0000) >> 16) as u8) // 5 bits
				.with_hour(((u32_value & 0xF800) >> 11) as u8) // 5 bits
				.with_minute(((u32_value & 0x7e0) >> 5) as u8) // 6 bits
				.with_seconds((u32_value & 0x1f) as u8), // 5 bits
		)
	}

	#[test]
	fn stf_date_parsing_works() {
		let (u32_value, expected_date) = test_date();
		let parsed_date = StfTimestamp::from(u32_value);
		assert_eq!(parsed_date, expected_date)
	}

	#[test]
	fn stf_date_round_trip_parsing_works() {
		let (expected_value, date) = test_date();
		assert_eq!(expected_value, u32::from(date));
	}
}
