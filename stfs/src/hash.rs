use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use serde::Serialize;
use std::io::Cursor;
use std::io::Read;

use crate::error::StfsError;
use crate::header::StfsVolumeDescriptor;
use crate::header::XContentHeader;
use crate::io::ReadAt;
use crate::types::*;

#[derive(Default, Debug, Serialize, Clone)]
pub struct HashEntry {
	pub block_hash: Sha1Digest,
	pub status: u8,
	pub next_block: BlockNumber,
}

#[derive(Debug, Serialize)]
pub struct HashTable {
	pub level: HashTableLevel,
	pub true_block_number: usize,
	pub entry_count: usize,
	pub address_in_file: usize,
	pub entries: Vec<HashEntry>,
}

impl Default for HashTable {
	fn default() -> Self {
		Self {
			level: HashTableLevel::First,
			true_block_number: 0,
			entry_count: 0,
			address_in_file: 0,
			entries: Vec::new(),
		}
	}
}

#[derive(Default, Debug, Serialize)]
pub struct HashTableMeta {
	pub block_step: [usize; 2],
	pub tables_per_level: [usize; 3],
	pub top_table: HashTable,
	pub first_table_address: usize,
}

impl HashTableMeta {
	pub fn from_header(header: &XContentHeader, sex: StfsPackageSex) -> Result<Self, StfsError> {
		let block_step = sex.block_step();
		let first_table_address = ((header.header_size as usize) + 0x0FFF) & 0xFFFF_F000;

		let stfs_vol = header.volume_descriptor.stfs_ref();
		let allocated_block_count = stfs_vol.allocated_block_count as usize;

		let tables_per_level = [
			(allocated_block_count / HASHES_PER_HASH_TABLE)
				+ if !allocated_block_count.is_multiple_of(HASHES_PER_HASH_TABLE) { 1 } else { 0 },
			0,
			0,
		];

		let top_level = header.root_hash_table_level()?;

		let mut meta = HashTableMeta {
			block_step,
			tables_per_level,
			first_table_address,
			top_table: HashTable { level: top_level, ..Default::default() },
		};

		meta.top_table.true_block_number = meta.compute_backing_hash_block_number_for_level(0, top_level, sex);

		let base_address = (meta.top_table.true_block_number * BLOCK_SIZE) + first_table_address;
		meta.top_table.address_in_file = base_address + (((stfs_vol.block_separation as usize) & 2) << 0xB);

		meta.top_table.entry_count = allocated_block_count / DATA_BLOCKS_PER_HASH_TREE_LEVEL[top_level as usize];

		if (allocated_block_count > DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]
			&& !allocated_block_count.is_multiple_of(DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]))
			|| (allocated_block_count > HASHES_PER_HASH_TABLE
				&& !allocated_block_count.is_multiple_of(HASHES_PER_HASH_TABLE))
		{
			meta.top_table.entry_count += 1;
		}

		Ok(meta)
	}

	pub fn read_top_table<R: ReadAt>(&mut self, source: &R) -> Result<(), StfsError> {
		let entry_size = 0x18;
		let range_start = self.top_table.address_in_file;
		let range_end = range_start + self.top_table.entry_count * entry_size;
		let data = source.read_at(range_start..range_end)?;
		let data = data.as_ref();

		let mut cursor = Cursor::new(data);
		self.top_table.entries.reserve(self.top_table.entry_count);

		for _ in 0..self.top_table.entry_count {
			let mut hash_bytes = [0u8; 0x14];
			cursor.read_exact(&mut hash_bytes)?;
			let status = cursor.read_u8()?;
			let next_block = BlockNumber(cursor.read_u24::<BigEndian>()? as usize);
			self.top_table.entries.push(HashEntry { block_hash: Sha1Digest(hash_bytes), status, next_block });
		}

		Ok(())
	}

	pub fn compute_backing_hash_block_number_for_level(
		&self,
		block: usize,
		level: HashTableLevel,
		sex: StfsPackageSex,
	) -> usize {
		match level {
			HashTableLevel::First => self.compute_first_level_backing_hash_block_number(block, sex),
			HashTableLevel::Second => self.compute_second_level_backing_hash_block_number(block, sex),
			HashTableLevel::Third => self.compute_third_level_backing_hash_block_number(),
		}
	}

	pub fn compute_first_level_backing_hash_block_number(&self, block: usize, sex: StfsPackageSex) -> usize {
		if block < HASHES_PER_HASH_TABLE {
			return 0;
		}

		let mut block_number = (block / HASHES_PER_HASH_TABLE) * self.block_step[0];
		block_number += ((block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]) + 1) << (sex as u8);

		if block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] == 0 {
			block_number
		} else {
			block_number + (1 << (sex as u8))
		}
	}

	pub fn compute_second_level_backing_hash_block_number(&self, block: usize, sex: StfsPackageSex) -> usize {
		if block < DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] {
			self.block_step[0]
		} else {
			(1 << (sex as u8)) + (block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]) * self.block_step[1]
		}
	}

	pub fn compute_third_level_backing_hash_block_number(&self) -> usize {
		self.block_step[1]
	}

	pub fn block_to_addr(&self, block: BlockNumber, sex: StfsPackageSex) -> usize {
		let block = block.0;
		if block > 2usize.pow(24) - 1 {
			panic!("invalid block: {:#x}", block);
		}

		(self.compute_data_block_num(block, sex) * BLOCK_SIZE) + self.first_table_address
	}

	pub fn compute_data_block_num(&self, block: usize, sex: StfsPackageSex) -> usize {
		let addr = (((block + HASHES_PER_HASH_TABLE) / HASHES_PER_HASH_TABLE) << (sex as usize)) + block;
		if block < HASHES_PER_HASH_TABLE {
			addr
		} else if block < DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] {
			(addr + ((addr + DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]) / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2])) << sex as usize
		} else {
			(1 << sex as usize)
				+ ((addr + ((block + DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]) / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]))
					<< sex as usize)
		}
	}

	pub fn hash_table_skip_for_address(&self, table_address: usize, sex: StfsPackageSex) -> usize {
		let mut block_number = (table_address - self.first_table_address) / BLOCK_SIZE;

		if block_number == 0 {
			return BLOCK_SIZE << sex as usize;
		}

		if block_number == self.block_step[1] {
			return 0x3000 << sex as usize;
		} else if block_number > self.block_step[1] {
			block_number -= self.block_step[1] + (1 << sex as usize);
		}

		if block_number == self.block_step[0] || block_number.is_multiple_of(self.block_step[1]) {
			return 0x2000 << sex as usize;
		}

		BLOCK_SIZE << sex as usize
	}

	pub fn read_block_hash_entry<R: ReadAt>(
		&self,
		source: &R,
		block: BlockNumber,
		sex: StfsPackageSex,
		stfs_vol: &StfsVolumeDescriptor,
	) -> Result<HashEntry, StfsError> {
		if block.0 > stfs_vol.allocated_block_count as usize {
			return Err(StfsError::InvalidBlock { block: block.0, allocated: stfs_vol.allocated_block_count as usize });
		}

		let addr = self.block_hash_address(block.0, sex, stfs_vol, source)?;
		let data = source.read_at(addr..addr + 0x18)?;
		let data = data.as_ref();

		let mut cursor = Cursor::new(data);
		let mut hash_bytes = [0u8; 0x14];
		cursor.read_exact(&mut hash_bytes)?;
		let status = cursor.read_u8()?;
		let next_block = BlockNumber(cursor.read_u24::<BigEndian>()? as usize);

		Ok(HashEntry { block_hash: Sha1Digest(hash_bytes), status, next_block })
	}

	pub fn block_hash_address<R: ReadAt>(
		&self,
		block: usize,
		sex: StfsPackageSex,
		stfs_vol: &StfsVolumeDescriptor,
		source: &R,
	) -> Result<usize, StfsError> {
		if block > stfs_vol.allocated_block_count as usize {
			return Err(StfsError::InvalidBlock { block, allocated: stfs_vol.allocated_block_count as usize });
		}

		let mut hash_addr =
			(self.compute_first_level_backing_hash_block_number(block, sex) * BLOCK_SIZE) + self.first_table_address;
		hash_addr += (block % HASHES_PER_HASH_TABLE) * 0x18;

		match self.top_table.level {
			HashTableLevel::First => Ok(hash_addr + (((stfs_vol.block_separation as usize) & 2) << 0xB)),
			HashTableLevel::Second => {
				let entry = &self.top_table.entries[block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[1]];
				Ok(hash_addr + (((entry.status as usize) & 0x40) << 6))
			}
			HashTableLevel::Third => {
				let first_level_offset =
					((self.top_table.entries[block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]].status as usize) & 0x40) << 6;

				let position = (self.compute_second_level_backing_hash_block_number(block, sex) * BLOCK_SIZE)
					+ self.first_table_address
					+ first_level_offset
					+ ((block % DATA_BLOCKS_PER_HASH_TREE_LEVEL[1]) * 0x18);

				let data = source.read_at(position + 0x14..position + 0x15)?;
				let status_byte = data.as_ref()[0];

				Ok(hash_addr + (((status_byte as usize) & 0x40) << 0x6))
			}
		}
	}
}
