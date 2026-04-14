use std::collections::HashMap;

use sha1::Digest;
use sha1::Sha1;

use crate::error::StfsError;
use crate::hash::HashTableMeta;
use crate::header::StfsVolumeDescriptor;
use crate::io::ReadAt;
use crate::types::*;

pub use crate::types::Sha1Digest;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HashCacheKey {
	block: usize,
	level: HashTableLevel,
}

#[derive(Debug, Clone)]
pub struct BlockHashReport {
	pub block: usize,
	pub level: HashTableLevel,
	pub calculated_hash: Sha1Digest,
	pub expected_hash: Sha1Digest,
	pub is_valid: bool,
}

pub struct StfsHasher {
	cache: HashMap<HashCacheKey, BlockHashReport>,
}

impl StfsHasher {
	pub fn new() -> Self {
		StfsHasher { cache: HashMap::new() }
	}

	pub fn hash_block(data: &[u8]) -> Sha1Digest {
		let mut hasher = Sha1::new();
		hasher.update(data);
		Sha1Digest(hasher.finalize().into())
	}

	/// Verify the hash chain for a data block, walking top-down from the root.
	///
	/// Returns reports for each level of the hash tree that was checked,
	/// from the top table down to the level-0 hash table covering this block.
	pub fn verify_data_block<R: ReadAt>(
		&mut self,
		source: &R,
		data_block: BlockNumber,
		sex: StfsPackageSex,
		hash_meta: &HashTableMeta,
		stfs_vol: &StfsVolumeDescriptor,
	) -> Result<Vec<BlockHashReport>, StfsError> {
		let data_block = data_block.0;
		let top_level = hash_meta.top_table.level;
		let mut reports = Vec::new();

		// Walk from the top level down to level 0
		match top_level {
			HashTableLevel::Third => {
				let report = self.verify_hash_table_block(
					source,
					data_block,
					HashTableLevel::Third,
					&stfs_vol.top_hash_table_hash,
					sex,
					hash_meta,
					stfs_vol,
				)?;
				reports.push(report);

				let second_expected =
					&hash_meta.top_table.entries[data_block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]].block_hash;
				let report = self.verify_hash_table_block(
					source,
					data_block,
					HashTableLevel::Second,
					second_expected,
					sex,
					hash_meta,
					stfs_vol,
				)?;
				reports.push(report);

				let first_expected = self.read_hash_entry_at_level(
					source,
					data_block,
					HashTableLevel::Second,
					sex,
					hash_meta,
					stfs_vol,
				)?;
				let report = self.verify_hash_table_block(
					source,
					data_block,
					HashTableLevel::First,
					&first_expected,
					sex,
					hash_meta,
					stfs_vol,
				)?;
				reports.push(report);
			}
			HashTableLevel::Second => {
				let report = self.verify_hash_table_block(
					source,
					data_block,
					HashTableLevel::Second,
					&stfs_vol.top_hash_table_hash,
					sex,
					hash_meta,
					stfs_vol,
				)?;
				reports.push(report);

				let first_expected =
					&hash_meta.top_table.entries[data_block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[1]].block_hash;
				let report = self.verify_hash_table_block(
					source,
					data_block,
					HashTableLevel::First,
					first_expected,
					sex,
					hash_meta,
					stfs_vol,
				)?;
				reports.push(report);
			}
			HashTableLevel::First => {
				let report = self.verify_hash_table_block(
					source,
					data_block,
					HashTableLevel::First,
					&stfs_vol.top_hash_table_hash,
					sex,
					hash_meta,
					stfs_vol,
				)?;
				reports.push(report);
			}
		}

		Ok(reports)
	}

	/// Verify that a data block's content matches the hash stored in its level-0 hash entry.
	pub fn verify_data_block_content<R: ReadAt>(
		&self,
		source: &R,
		data_block: BlockNumber,
		sex: StfsPackageSex,
		hash_meta: &HashTableMeta,
		stfs_vol: &StfsVolumeDescriptor,
	) -> Result<BlockHashReport, StfsError> {
		let entry = hash_meta.read_block_hash_entry(source, data_block, sex, stfs_vol)?;
		let addr = hash_meta.block_to_addr(data_block, sex);
		let block_data = source.read_at(addr..addr + BLOCK_SIZE)?;
		let calculated = Self::hash_block(block_data.as_ref());

		Ok(BlockHashReport {
			block: data_block.0,
			level: HashTableLevel::First,
			calculated_hash: calculated,
			expected_hash: entry.block_hash,
			is_valid: calculated == entry.block_hash,
		})
	}

	fn verify_hash_table_block<R: ReadAt>(
		&mut self,
		source: &R,
		data_block: usize,
		level: HashTableLevel,
		expected_hash: &Sha1Digest,
		sex: StfsPackageSex,
		hash_meta: &HashTableMeta,
		stfs_vol: &StfsVolumeDescriptor,
	) -> Result<BlockHashReport, StfsError> {
		let hash_block_num = hash_meta.compute_backing_hash_block_number_for_level(data_block, level, sex);

		let key = HashCacheKey { block: hash_block_num, level };
		if let Some(cached) = self.cache.get(&key) {
			return Ok(cached.clone());
		}

		let active_index = self.active_index_for_level(level, data_block, hash_meta, stfs_vol);
		let base_addr = (hash_block_num * BLOCK_SIZE) + hash_meta.first_table_address + (active_index << 12);

		let block_data = source.read_at(base_addr..base_addr + BLOCK_SIZE)?;
		let calculated = Self::hash_block(block_data.as_ref());

		let report = BlockHashReport {
			block: hash_block_num,
			level,
			calculated_hash: calculated,
			expected_hash: *expected_hash,
			is_valid: calculated == *expected_hash,
		};

		self.cache.insert(key, report.clone());
		Ok(report)
	}

	fn active_index_for_level(
		&self,
		level: HashTableLevel,
		data_block: usize,
		hash_meta: &HashTableMeta,
		stfs_vol: &StfsVolumeDescriptor,
	) -> usize {
		match (hash_meta.top_table.level, level) {
			// When this level IS the top table, use root active index from volume descriptor
			(l, target) if l as u8 == target as u8 => ((stfs_vol.block_separation as usize) & 2) >> 1,
			// Level below top: active index from top table entry's status byte
			(HashTableLevel::Second, HashTableLevel::First) => {
				let entry = &hash_meta.top_table.entries[data_block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[1]];
				((entry.status as usize) & 0x40) >> 6
			}
			(HashTableLevel::Third, HashTableLevel::Second) => {
				let entry = &hash_meta.top_table.entries[data_block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]];
				((entry.status as usize) & 0x40) >> 6
			}
			// Third -> First: would need to read the second-level entry (already handled by block_hash_address)
			// This case shouldn't arise since we walk top-down and verify_hash_table_block
			// only gets called with level matching the descent order
			_ => 0,
		}
	}

	/// Read the block_hash from a hash entry at a given level for a data block.
	fn read_hash_entry_at_level<R: ReadAt>(
		&self,
		source: &R,
		data_block: usize,
		level: HashTableLevel,
		sex: StfsPackageSex,
		hash_meta: &HashTableMeta,
		stfs_vol: &StfsVolumeDescriptor,
	) -> Result<Sha1Digest, StfsError> {
		let hash_block_num = hash_meta.compute_backing_hash_block_number_for_level(data_block, level, sex);

		let active_index = self.active_index_for_level(level, data_block, hash_meta, stfs_vol);
		let base_addr = (hash_block_num * BLOCK_SIZE) + hash_meta.first_table_address + (active_index << 12);

		// Entry index within the hash table depends on the level
		let entry_index = match level {
			HashTableLevel::First => data_block % HASHES_PER_HASH_TABLE,
			HashTableLevel::Second => (data_block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[1]) % HASHES_PER_HASH_TABLE,
			HashTableLevel::Third => (data_block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]) % HASHES_PER_HASH_TABLE,
		};

		let entry_addr = base_addr + entry_index * 0x18;
		let data = source.read_at(entry_addr..entry_addr + 0x14)?;
		let mut bytes = [0u8; 0x14];
		bytes.copy_from_slice(data.as_ref());
		Ok(Sha1Digest(bytes))
	}

	pub fn reports(&self) -> impl Iterator<Item = &BlockHashReport> {
		self.cache.values()
	}

	pub fn all_valid(&self) -> bool {
		self.cache.values().all(|r| r.is_valid)
	}
}

impl Default for StfsHasher {
	fn default() -> Self {
		Self::new()
	}
}
