use std::collections::HashMap;

use sha1::Digest;
use sha1::Sha1;

use crate::error::StfsError;
use crate::hash::HashTableMeta;
use crate::header::StfsVolumeDescriptor;
use crate::io::ReadAt;
use crate::types::*;

pub use crate::types::Sha1Digest;

pub struct HashVerifyContext<'a> {
	pub sex: StfsPackageSex,
	pub hash_meta: &'a HashTableMeta,
	pub stfs_vol: &'a StfsVolumeDescriptor,
}

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
		ctx: &HashVerifyContext,
	) -> Result<Vec<BlockHashReport>, StfsError> {
		let data_block = data_block.0;
		let top_level = ctx.hash_meta.top_table.level;
		let mut reports = Vec::new();

		match top_level {
			HashTableLevel::Third => {
				reports.push(self.verify_hash_table_block(
					source,
					data_block,
					HashTableLevel::Third,
					&ctx.stfs_vol.top_hash_table_hash,
					ctx,
				)?);

				let second_expected =
					&ctx.hash_meta.top_table.entries[data_block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]].block_hash;
				reports.push(self.verify_hash_table_block(
					source,
					data_block,
					HashTableLevel::Second,
					second_expected,
					ctx,
				)?);

				let first_expected = self.read_hash_entry_at_level(source, data_block, HashTableLevel::Second, ctx)?;
				reports.push(self.verify_hash_table_block(
					source,
					data_block,
					HashTableLevel::First,
					&first_expected,
					ctx,
				)?);
			}
			HashTableLevel::Second => {
				reports.push(self.verify_hash_table_block(
					source,
					data_block,
					HashTableLevel::Second,
					&ctx.stfs_vol.top_hash_table_hash,
					ctx,
				)?);

				let first_expected =
					&ctx.hash_meta.top_table.entries[data_block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[1]].block_hash;
				reports.push(self.verify_hash_table_block(
					source,
					data_block,
					HashTableLevel::First,
					first_expected,
					ctx,
				)?);
			}
			HashTableLevel::First => {
				reports.push(self.verify_hash_table_block(
					source,
					data_block,
					HashTableLevel::First,
					&ctx.stfs_vol.top_hash_table_hash,
					ctx,
				)?);
			}
		}

		Ok(reports)
	}

	pub fn verify_data_block_content<R: ReadAt>(
		&self,
		source: &R,
		data_block: BlockNumber,
		ctx: &HashVerifyContext,
	) -> Result<BlockHashReport, StfsError> {
		let entry = ctx.hash_meta.read_block_hash_entry(source, data_block, ctx.sex, ctx.stfs_vol)?;
		let addr = ctx.hash_meta.block_to_addr(data_block, ctx.sex);
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
		ctx: &HashVerifyContext,
	) -> Result<BlockHashReport, StfsError> {
		let hash_block_num = ctx.hash_meta.compute_backing_hash_block_number_for_level(data_block, level, ctx.sex);

		let key = HashCacheKey { block: hash_block_num, level };
		if let Some(cached) = self.cache.get(&key) {
			return Ok(cached.clone());
		}

		let active_index = self.active_index_for_level(level, data_block, ctx);
		let base_addr = (hash_block_num * BLOCK_SIZE) + ctx.hash_meta.first_table_address + (active_index << 12);

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

	fn active_index_for_level(&self, level: HashTableLevel, data_block: usize, ctx: &HashVerifyContext) -> usize {
		match (ctx.hash_meta.top_table.level, level) {
			(l, target) if l as u8 == target as u8 => ((ctx.stfs_vol.block_separation as usize) & 2) >> 1,
			(HashTableLevel::Second, HashTableLevel::First) => {
				let entry = &ctx.hash_meta.top_table.entries[data_block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[1]];
				((entry.status as usize) & 0x40) >> 6
			}
			(HashTableLevel::Third, HashTableLevel::Second) => {
				let entry = &ctx.hash_meta.top_table.entries[data_block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]];
				((entry.status as usize) & 0x40) >> 6
			}
			_ => 0,
		}
	}

	fn read_hash_entry_at_level<R: ReadAt>(
		&self,
		source: &R,
		data_block: usize,
		level: HashTableLevel,
		ctx: &HashVerifyContext,
	) -> Result<Sha1Digest, StfsError> {
		let hash_block_num = ctx.hash_meta.compute_backing_hash_block_number_for_level(data_block, level, ctx.sex);

		let active_index = self.active_index_for_level(level, data_block, ctx);
		let base_addr = (hash_block_num * BLOCK_SIZE) + ctx.hash_meta.first_table_address + (active_index << 12);

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
