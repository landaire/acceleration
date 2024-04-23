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
use crate::sparse_reader::SparseReader;
use crate::util::*;

pub type StfsEntryRef = Arc<Mutex<StfsEntry>>;

const BLOCK_SIZE: usize = 0x1000;

#[derive(Debug, Serialize, PartialEq, Eq, Copy, Clone)]
#[binrw]
pub enum SignatureType {
	/// User container packages that are created by an Xbox 360 console and
	/// signed by the user's private key.
	#[brw(magic = b"CON ")]
	Console,
	/// Xbox LIVE-distributed package that is signed by Microsoft's private key.
	#[brw(magic = b"LIVE")]
	Live,
	/// Offline-distributed package that is signed by Microsoft's private key.
	#[brw(magic = b"PIRS")]
	Pirs,
}

impl std::fmt::Display for SignatureType {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let description = match self {
			SignatureType::Console => "Console (CON)",
			SignatureType::Live => "Xbox LIVE Strong Signature (LIVE)",
			SignatureType::Pirs => "Offline Strong Signature (PIRS)",
		};

		f.write_str(description)
	}
}

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
	pub first_table_address: usize,
}

impl HashTableMeta {
	pub fn new(sex: StfsPackageReadFlag, header: &XContentHeader) -> Result<Self, StfsError> {
		let mut meta = HashTableMeta::default();

		meta.block_step = sex.block_step();

		// Address of the first hash table in the package comes right after the header
		meta.first_table_address = ((header.header_size as usize) + 0x0FFF) & 0xFFFF_F000;

		let stfs_vol = header
			.metadata
			.volume_descriptor
			.stfs_ref()
			.expect("volume descriptor does not represent an STFS filesystem");

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

		meta.top_table.level = header.root_hash_table_level()?;
		meta.top_table.true_block_number =
			meta.compute_backing_hash_block_number_for_level(Block(0), meta.top_table.level, sex);

		let base_address = (meta.top_table.true_block_number.0 * BLOCK_SIZE) + meta.first_table_address;
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
	pub header: XContentHeader,
	pub hash_table_meta: Option<HashTableMeta>,
	pub files: StfsEntryRef,
}

impl TryFrom<&[u8]> for StfsPackage {
	type Error = StfsError;

	fn try_from(input: &[u8]) -> Result<Self, Self::Error> {
		let mut cursor = Cursor::new(input);
		let xcontent_header = cursor.read_be::<XContentHeader>()?;

		let is_stfs = xcontent_header.metadata.volume_descriptor.is_stfs();
		let hash_table_meta = if is_stfs {
			let mut hash_table_meta = HashTableMeta::new(xcontent_header.sex(), &xcontent_header)?;
			hash_table_meta.top_table.parse_hash_entries(&input[hash_table_meta.top_table.data_range()])?;
			Some(hash_table_meta)
		} else {
			None
		};

		let mut package = StfsPackage {
			header: xcontent_header,
			hash_table_meta,
			files: Arc::new(Mutex::new(StfsEntry::Folder { entry: Default::default(), files: Default::default() })),
		};

		if is_stfs {
			// package.read_files(input)?;
		}

		Ok(package)
	}
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

		if self.header.is_read_only() {
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
	fn hash_table_meta(&self) -> &HashTableMeta {
		self.hash_table_meta.as_ref().unwrap()
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
				.hash_table_meta()
				.compute_first_level_backing_hash_block_number(attributes.starting_block, self.header.sex())
				.0 + self.hash_table_meta().block_step[0])
				- (((start_address as usize) - self.hash_table_meta().first_table_address) / BLOCK_SIZE);

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
		let sex = self.header.sex() as usize;
		let hash_table_meta = self.hash_table_meta();

		// Convert the address to a true block number
		let mut block_number = (table_address - hash_table_meta.first_table_address) / BLOCK_SIZE;

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
		if let Some(stfs_vol) = self.header.metadata.volume_descriptor.stfs_ref() {
			if block.0 > stfs_vol.allocated_block_count as usize {
				panic!(
					"Reference to illegal block number: {:#x} ({:#x} allocated)",
					block.0, stfs_vol.allocated_block_count
				);
			}

			let mut reader = Cursor::new(input);
			reader.set_position(self.block_hash_address(block, input)?);
			Ok(reader.read_be::<HashEntry>()?)
		} else {
			panic!("invalid volume type");
		}
	}

	fn block_hash_address(&self, block: Block, input: &[u8]) -> Result<u64, StfsError> {
		if let Some(stfs_vol) = self.header.metadata.volume_descriptor.stfs_ref() {
			if block.0 > stfs_vol.allocated_block_count as usize {
				panic!(
					"Reference to illegal block number: {:#x} ({:#x} allocated)",
					block.0, stfs_vol.allocated_block_count
				);
			}

			let hash_table_meta = self.hash_table_meta();

			let mut hash_addr = (hash_table_meta
				.compute_first_level_backing_hash_block_number(block, self.header.sex())
				.0 * BLOCK_SIZE) + hash_table_meta.first_table_address;
			// 0x18 here is the size of the HashEntry structure
			hash_addr += (block.0 % HASHES_PER_BLOCK) * 0x18;
			let address = match hash_table_meta.top_table.level {
				// TODO: might have broken things with the flags here
				HashTableLevel::First => hash_addr as u64 + ((stfs_vol.flags.root_active_index() as u64) << 0xC),
				HashTableLevel::Second => {
					hash_addr as u64
						+ ((hash_table_meta.top_table.entries[block.0 / DATA_BLOCKS_PER_HASH_TREE_LEVEL_TEMP[1]].status
							as u64 & 0x40) << 6)
				}
				HashTableLevel::Third => {
					let first_level_offset = (hash_table_meta.top_table.entries
						[block.0 / DATA_BLOCKS_PER_HASH_TREE_LEVEL_TEMP[2]]
						.status as u64 & 0x40) << 6;

					let position = (hash_table_meta
						.compute_second_level_backing_hash_block_number(block, self.header.sex())
						.0 * BLOCK_SIZE) + hash_table_meta.first_table_address
						+ first_level_offset as usize
						+ ((block.0 % DATA_BLOCKS_PER_HASH_TREE_LEVEL_TEMP[1]) * 0x18);

					let status_byte = input[position + 0x14];
					hash_addr as u64 + ((status_byte as u64 & 0x40) << 0x6)
				}
			};

			Ok(address)
		} else {
			panic!("invalid filesystem")
		}
	}

	fn read_files(&mut self, input: &[u8]) -> Result<(), StfsError> {
		let stfs_vol =
			self.header.metadata.volume_descriptor.stfs_ref().expect("volume descriptor is not an STFS file");

		let mut reader = Cursor::new(input);
		let mut block = stfs_vol.file_table_block_num;
		let mut folders = HashMap::<u16, StfsEntryRef>::new();
		let mut files = Vec::new();
		// Inject a fake root folder
		folders.insert(
			0xffff,
			Arc::new(Mutex::new(StfsEntry::Folder { entry: StfsFileEntry::default(), files: Vec::new() })),
		);

		for block_idx in 0..(stfs_vol.file_table_block_count as usize) {
			let current_addr = self.block_to_addr(block);
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

		(self.compute_data_block_num(block) * BLOCK_SIZE as u64) + self.hash_table_meta().first_table_address as u64
	}

	/// Translates a data block to an absolute block, adjusting the block number to skip over any potential hash blocks.
	fn compute_data_block_num(&self, block: Block) -> u64 {
		// Read-only filesystems have different properties
		let blocks_per_hash_block = if self.header.is_read_only() { 1 } else { 2 };

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

#[derive(Debug, Serialize)]
#[binrw]
#[brw(big)]
#[br(import(signature_type: SignatureType))]
pub enum KeyMaterial {
	/// Only present in console-signed packages
	#[br(pre_assert(signature_type == SignatureType::Console))]
	Certificate(Certificate),

	/// Only present in strong-signed packages
	#[br(pre_assert(signature_type != SignatureType::Console))]
	Signature(#[br(count = 64)] Vec<u8>),
}

#[derive(Debug, Serialize)]
#[binrw]
#[br(import(is_profile_embedded_content: bool))]
pub enum XContentHeaderMetadata {
	#[br(pre_assert(!is_profile_embedded_content))]
	XContentPackage(XContentHeader),
}

#[derive(Debug, Serialize)]
#[binrw]
pub struct FixedLengthNullWideString(
	#[brw(pad_size_to = 128)]
	#[serde(serialize_with = "serialize_null_wide_string")]
	NullWideString,
);

impl std::ops::Deref for FixedLengthNullWideString {
	type Target = NullWideString;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

#[derive(Debug, Serialize)]
#[binrw]
#[brw(big)]
pub struct XContentHeader {
	pub signature_type: SignatureType,

	#[br(args(signature_type), pad_size_to = 0x228)]
	pub key_material: KeyMaterial,

	pub license_data: [LicenseEntry; 0x10],
	/// Content ID is the hash of the metadata and all headers below it.
	pub content_id: [u8; 0x14],
	pub header_size: u32,

	#[br(args(header_size))]
	pub metadata: XContentMetadata,
}

#[derive(Debug, Serialize)]
#[binrw]
#[br(import(header_size: u32))]
pub struct XContentMetadata {
	pub content_type: ContentType,
	pub metadata_version: u32,
	pub content_size: u64,
	pub media_id: u32,
	pub version: u32,
	pub base_version: u32,
	pub title_id: u32,
	pub platform: u8,
	pub executable_type: u8,
	pub disc_number: u8,
	pub disc_in_set: u8,
	pub savegame_id: u32,
	pub console_id: [u8; 5],
	pub creator_xuid: u64,

	#[brw(seek_before = std::io::SeekFrom::Start(0x3a9))]
	pub volume_kind: FileSystemKind,

	#[brw(seek_before = std::io::SeekFrom::Start(0x379))]
	#[br(args(volume_kind))]
	pub volume_descriptor: FileSystem,

	// Start metadata v1
	pub data_file_count: u32,
	pub data_file_combined_size: u64,

	// TODO: parse the inbetween data
	#[brw(seek_before = std::io::SeekFrom::Start(0x3fd))]
	pub device_id: [u8; 0x14],

	// TODO: support localized names
	pub display_name: [FixedLengthNullWideString; 12],

	#[brw(seek_before = std::io::SeekFrom::Start(0xd11))]
	pub display_description: [FixedLengthNullWideString; 12],

	#[serde(serialize_with = "serialize_null_wide_string")]
	#[brw(seek_before = std::io::SeekFrom::Start(0x1611))]
	pub publisher_name: NullWideString,

	#[serde(serialize_with = "serialize_null_wide_string")]
	#[brw(seek_before = std::io::SeekFrom::Start(0x1691))]
	pub title_name: NullWideString,

	#[brw(seek_before = std::io::SeekFrom::Start(0x1711))]
	pub transfer_flags: u8,
	pub thumbnail_image_size: u32,
	pub title_thumbnail_image_size: u32,

	#[br(count = thumbnail_image_size)]
	#[brw(pad_size_to(MAX_IMAGE_SIZE))]
	pub thumbnail_image: Vec<u8>,

	#[br(count = title_thumbnail_image_size)]
	#[brw(pad_size_to(MAX_IMAGE_SIZE))]
	pub title_image: Vec<u8>,

	#[br(if(((header_size + 0xFFF) & 0xFFFFF000) - 0x971A > 0x15F4))]
	pub installer_type: Option<InstallerType>,
	// #[br(if(installer_type.is_some()), args(installer_type.unwrap()))]
	// pub installer_meta: Option<InstallerMeta>,
	// #[br(if(content_type.has_content_metadata()), args(content_type))]
	// pub content_metadata: Option<ContentMetadata>,
}

impl XContentHeader {
	/// Returns which hash table level the root hash is in
	fn root_hash_table_level(&self) -> Result<HashTableLevel, StfsError> {
		if let FileSystem::Stfs(volume_descriptor) = &self.metadata.volume_descriptor {
			let level = if volume_descriptor.allocated_block_count as usize <= HASHES_PER_BLOCK {
				HashTableLevel::First
			} else if volume_descriptor.allocated_block_count as usize <= DATA_BLOCKS_PER_HASH_TREE_LEVEL[1] {
				HashTableLevel::Second
			} else if volume_descriptor.allocated_block_count as usize <= DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] {
				HashTableLevel::Third
			} else {
				return Err(StfsError::InvalidHeader);
			};

			Ok(level)
		} else {
			Err(StfsError::InvalidPackageType)
		}
	}

	pub fn is_read_only(&self) -> bool {
		if let FileSystem::Stfs(stfs) = &self.metadata.volume_descriptor {
			stfs.flags.read_only()
		} else {
			false
		}
	}

	pub fn sex(&self) -> StfsPackageReadFlag {
		if self.is_read_only() {
			StfsPackageReadFlag::ReadOnly
		} else {
			StfsPackageReadFlag::ReadWrite
		}
	}
}

#[derive(Debug, Serialize)]
#[binrw]
pub struct AvatarAssetInformation {
	subcategory: AssetSubcategory,
	#[brw(little)]
	colorizable: u32,
	guid: [u8; 0x10],
	skeleton_version: SkeletonVersion,
}

#[derive(Debug, Serialize)]
#[binrw]
pub struct MediaInformation {
	series_id: [u8; 0x10],
	season_id: [u8; 0x10],
	season_number: u16,
	episode_number: u16,
}

#[derive(Debug, Serialize)]
#[binrw]
pub struct InstallerProgressCache {
	resume_state: OnlineContentResumeState,
	current_file_index: u32,
	current_file_offset: u64,
	bytes_processed: u64,
	timestamp_high: u32,
	timestamp_low: u32,
	#[br(count = 0)]
	cab_resume_data: Vec<u8>,
}

#[derive(Debug, Serialize)]
#[binrw]
pub struct FullInstallerMeta {
	installer_base_version: Version,
	installer_version: Version,
}

#[derive(Debug, Serialize, Variantly)]
#[binrw]
#[br(import(installer_type: InstallerType))]
pub enum InstallerMeta {
	#[br(pre_assert(installer_type.has_full_installer_meta()))]
	FullInstaller(FullInstallerMeta),
	#[br(pre_assert(installer_type.has_installer_progress_cache()))]
	InstallerProgressCache(InstallerProgressCache),
}

#[derive(Debug, Serialize)]
#[binrw]
pub struct Certificate {
	pubkey_cert_size: u16,
	owner_console_id: [u8; 5],
	#[brw(pad_size_to = 0x11)]
	#[serde(serialize_with = "serialize_null_wide_string")]
	owner_console_part_number: NullWideString,
	console_type_flags: Option<ConsoleTypeFlags>,
	#[br(try_map = |x: [u8; 8]| String::from_utf8(x.to_vec()))]
	#[bw(map = |x| x.as_bytes(), assert(date_generation.len() == 8, "date_generation.len() != 8"))]
	date_generation: String,
	public_exponent: u32,
	#[br(count = 0x80)]
	public_modulus: Vec<u8>,
	#[br(count = 0x100)]
	certificate_signature: Vec<u8>,
	#[br(count = 0x80)]
	signature: Vec<u8>,
}

bitflags! {
	#[derive(Serialize)]
	#[binrw]
	struct ConsoleTypeFlags: u32 {
		const DEVKIT = 0x1;
		const RETAIL = 0x2;
		const TESTKIT = 0x40000000;
		const RECOVERY_GENERATED = 0x80000000;
	}
}

#[derive(Debug, Serialize, Clone, Copy)]
#[binrw]
#[brw(repr = u16)]
enum LicenseType {
	Unused = 0x0000,
	Unrestricted = 0xFFFF,
	ConsoleProfileLicense = 0x0009,
	WindowsProfileLicense = 0x0003,
	ConsoleLicense = 0xF000,
	MediaFlags = 0xE000,
	KeyVaultPrivileges = 0xD000,
	HyperVisorFlags = 0xC000,
	UserPrivileges = 0xB000,
}

impl Default for LicenseType {
	fn default() -> Self {
		Self::Unused
	}
}

#[derive(Default, Debug, Serialize)]
#[binrw]
pub struct LicenseEntry {
	ty: LicenseType,
	data: [u8; 6],
	bits: u32,
	flags: u32,
}

#[derive(Debug, Serialize)]
#[binrw]
#[br(import(content_type: ContentType))]
pub enum ContentMetadata {
	#[br(pre_assert(content_type == ContentType::AvatarItem))]
	AvatarItem(AvatarAssetInformation),

	#[br(pre_assert(content_type == ContentType::Video))]
	Video(MediaInformation),
}

#[derive(Debug, Serialize, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[binrw]
#[brw(repr = u32)]
pub enum ContentType {
	ArcadeGame = 0xD0000,
	AvatarAssetPack = 0x8000,
	AvatarItem = 0x9000,
	CacheFile = 0x40000,
	CommunityGame = 0x2000000,
	GameDemo = 0x80000,
	GameOnDemand = 0x7000,
	GamerPicture = 0x20000,
	GamerTitle = 0xA0000,
	GameTrailer = 0xC0000,
	GameVideo = 0x400000,
	InstalledGame = 0x4000,
	Installer = 0xB0000,
	IPTVPauseBuffer = 0x2000,
	LicenseStore = 0xF0000,
	MarketplaceContent = 2,
	Movie = 0x100000,
	MusicVideo = 0x300000,
	PodcastVideo = 0x500000,
	Profile = 0x10000,
	Publisher = 3,
	SavedGame = 1,
	StorageDownload = 0x50000,
	Theme = 0x30000,
	Video = 0x200000,
	ViralVideo = 0x600000,
	XboxDownload = 0x70000,
	XboxOriginalGame = 0x5000,
	XboxSavedGame = 0x60000,
	Xbox360Title = 0x1000,
	XNA = 0xE0000,
}

impl ContentType {
	pub fn has_content_metadata(&self) -> bool {
		matches!(self, ContentType::AvatarItem | ContentType::Video)
	}
}

#[derive(Debug, Serialize, Copy, Clone)]
#[binrw]
#[brw(repr = u32)]
pub enum InstallerType {
	None = 0,
	SystemUpdate = 0x53555044,
	TitleUpdate = 0x54555044,
	SystemUpdateProgressCache = 0x50245355,
	TitleUpdateProgressCache = 0x50245455,
	TitleContentProgressCache = 0x50245443,
}

impl InstallerType {
	pub fn has_full_installer_meta(&self) -> bool {
		matches!(self, InstallerType::SystemUpdate | InstallerType::TitleUpdate)
	}

	pub fn has_installer_progress_cache(&self) -> bool {
		matches!(
			self,
			InstallerType::SystemUpdateProgressCache
				| InstallerType::TitleUpdateProgressCache
				| Self::TitleContentProgressCache
		)
	}
}

#[derive(Debug, Serialize, Copy, Clone)]
#[binrw]
#[br(map = |input: u32| Self::from(input))]
#[bw(map = |this: &Self| u32::from(*this))]
pub struct Version {
	major: u16,
	minor: u16,
	build: u16,
	revision: u16,
}

impl From<u32> for Version {
	fn from(input: u32) -> Self {
		Version {
			major: ((input & 0xF000_0000) >> 28) as u16,
			minor: ((input & 0x0F00_0000) >> 24) as u16,
			build: ((input & 0x00FF_FF00) >> 8) as u16,
			revision: (input & 0xFF) as u16,
		}
	}
}

impl From<Version> for u32 {
	fn from(value: Version) -> Self {
		let Version { major, minor, build, revision } = value;
		let major = major as u32;
		let minor = minor as u32;
		let build = build as u32;
		let revision = revision as u32;

		(major << 28) | (minor << 24) | (build << 8) | revision
	}
}

#[derive(Debug, Serialize, Copy, Clone)]
#[binrw]
#[brw(repr = u32)]
enum OnlineContentResumeState {
	FileHeadersNotReady = 0x46494C48,
	NewFolder = 0x666F6C64,
	NewFolderResumeAttempt1 = 0x666F6C31,
	NewFolderResumeAttempt2 = 0x666F6C32,
	NewFolderResumeAttemptUnknown = 0x666F6C3F,
	NewFolderResumeAttemptSpecific = 0x666F6C40,
}

#[derive(Debug, Serialize, Copy, Clone)]
pub enum XContentFlags {
	MetadataIsPEC = 1,
	MetadataSkipRead = 2,
	MetadataDontFreeThumbnails = 4,
}

#[derive(Debug, Serialize, PartialEq, Eq, Copy, Clone)]
#[binrw]
#[brw(repr = u32)]
pub enum FileSystemKind {
	Stfs = 0,
	Svod,
	Fatx,
}

#[derive(Debug, Serialize, Variantly)]
#[binrw]
#[br(import(fs_kind: FileSystemKind))]
pub enum FileSystem {
	#[br(pre_assert(fs_kind == FileSystemKind::Stfs))]
	Stfs(StfsVolumeDescriptor),
	#[br(pre_assert(fs_kind == FileSystemKind::Svod))]
	Svod(SvodVolumeDescriptor),
	#[br(pre_assert(fs_kind == FileSystemKind::Fatx))]
	Fatx,
}

impl Default for FileSystem {
	fn default() -> Self {
		FileSystem::Stfs(StfsVolumeDescriptor::default())
	}
}

impl FileSystem {}

#[bitfield]
#[binrw]
#[br(map = Self::from_bytes)]
#[bw(map = |flags: &Self| flags.into_bytes())]
#[derive(Default, Debug, Copy, Clone, Serialize)]
pub struct StfsVolumeDescriptorFlags {
	_reserved: B4,
	dir_index_bounds_are_valid: bool,
	dir_is_overallocated: bool,
	root_active_index: bool,
	read_only: bool,
}

#[derive(Default, Debug, Serialize)]
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

#[derive(Debug, Serialize, Copy, Clone, Eq, PartialEq)]
#[binrw]
#[brw(repr = u32)]
enum AssetSubcategory {
	CarryableCarryable = 0x44c,
	// CarryableFirst = 0x44c,
	// CarryableLast = 0x44c,
	CostumeCasualSuit = 0x68,
	CostumeCostume = 0x69,
	// CostumeFirst = 100,
	CostumeFormalSuit = 0x67,
	// CostumeLast = 0x6a,
	CostumeLongDress = 0x65,
	CostumeShortDress = 100,
	EarringsDanglers = 0x387,
	// EarringsFirst = 900,
	EarringsLargehoops = 0x38b,
	// EarringsLast = 0x38b,
	EarringsSingleDangler = 0x386,
	EarringsSingleLargeHoop = 0x38a,
	EarringsSingleSmallHoop = 0x388,
	EarringsSingleStud = 900,
	EarringsSmallHoops = 0x389,
	EarringsStuds = 0x385,
	GlassesCostume = 0x2be,
	// GlassesFirst = 700,
	GlassesGlasses = 700,
	// GlassesLast = 0x2be,
	GlassesSunglasses = 0x2bd,
	GlovesFingerless = 600,
	// GlovesFirst = 600,
	GlovesFullFingered = 0x259,
	// GlovesLast = 0x259,
	HatBaseballCap = 0x1f6,
	HatBeanie = 500,
	HatBearskin = 0x1fc,
	HatBrimmed = 0x1f8,
	HatCostume = 0x1fb,
	HatFez = 0x1f9,
	// HatFirst = 500,
	HatFlatCap = 0x1f5,
	HatHeadwrap = 0x1fa,
	HatHelmet = 0x1fd,
	// HatLast = 0x1fd,
	HatPeakCap = 0x1f7,
	// RingFirst = 0x3e8,
	RingLast = 0x3ea,
	RingLeft = 0x3e9,
	RingRight = 0x3e8,
	ShirtCoat = 210,
	// ShirtFirst = 200,
	ShirtHoodie = 0xd0,
	ShirtJacket = 0xd1,
	// ShirtLast = 210,
	ShirtLongSleeveShirt = 0xce,
	ShirtLongSleeveTee = 0xcc,
	ShirtPolo = 0xcb,
	ShirtShortSleeveShirt = 0xcd,
	ShirtSportsTee = 200,
	ShirtSweater = 0xcf,
	ShirtTee = 0xc9,
	ShirtVest = 0xca,
	ShoesCostume = 0x197,
	// ShoesFirst = 400,
	ShoesFormal = 0x193,
	ShoesHeels = 0x191,
	ShoesHighBoots = 0x196,
	// ShoesLast = 0x197,
	ShoesPumps = 0x192,
	ShoesSandals = 400,
	ShoesShortBoots = 0x195,
	ShoesTrainers = 0x194,
	TrousersCargo = 0x131,
	// TrousersFirst = 300,
	TrousersHotpants = 300,
	TrousersJeans = 0x132,
	TrousersKilt = 0x134,
	// TrousersLast = 0x135,
	TrousersLeggings = 0x12f,
	TrousersLongShorts = 0x12e,
	TrousersLongSkirt = 0x135,
	TrousersShorts = 0x12d,
	TrousersShortSkirt = 0x133,
	TrousersTrousers = 0x130,
	WristwearBands = 0x322,
	WristwearBracelet = 800,
	// WristwearFirst = 800,
	// WristwearLast = 0x323,
	WristwearSweatbands = 0x323,
	WristwearWatch = 0x321,
}

#[derive(Debug, Serialize)]
enum BinaryAssetType {
	Component = 1,
	Texture = 2,
	ShapeOverride = 3,
	Animation = 4,
	ShapeOverridePost = 5,
}

#[derive(Debug, Serialize)]
#[binrw]
#[brw(repr = u8)]
enum SkeletonVersion {
	Nxe = 1,
	Natal,
	NxeAndNatal,
}

#[derive(Debug, Serialize)]
#[binrw]
#[brw(repr = u8)]
enum AssetGender {
	Male = 1,
	Female,
	Both,
}

#[derive(Debug, Serialize)]
#[binrw]
pub struct SvodVolumeDescriptor {
	size: u8,
	block_cache_element_count: u8,
	worker_thread_processor: u8,
	worker_thread_priority: u8,
	root_hash: [u8; 0x14],
	flags: u8,
	/// Encoded as an int24
	#[br(parse_with = binrw::helpers::read_u24)]
	#[bw(write_with = binrw::helpers::write_u24)]
	data_block_count: u32,
	/// Encoded as an int24
	#[br(parse_with = binrw::helpers::read_u24)]
	#[bw(write_with = binrw::helpers::write_u24)]
	data_block_offset: u32,
	reserved: [u8; 5],
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
