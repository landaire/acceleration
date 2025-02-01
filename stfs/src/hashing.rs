use std::collections::HashMap;
use std::io::Read;
use std::io::Seek;

use crate::consts::BLOCK_SIZE;
use crate::AbsoluteBlock;
use crate::Block;
use crate::HashEntry;
use crate::HashTableLevel;
use crate::StfsError;
use crate::StfsPackage;
use sha1::Digest;
use sha1::Sha1;

pub type StfsBlockHash = [u8; 20];

pub struct BlockHashReport {
	block: Block,
	calculated_hash: StfsBlockHash,
	hash_entry: HashEntry,
	is_valid: bool,
}

impl BlockHashReport {
	pub fn block(&self) -> Block {
		self.block
	}

	pub fn calculated_hash(&self) -> [u8; 20] {
		self.calculated_hash
	}

	pub fn hash_entry(&self) -> &HashEntry {
		&self.hash_entry
	}

	pub fn is_valid(&self) -> bool {
		self.is_valid
	}
}

struct StfsHasher {
	checked_hashes: HashMap<(Block, HashTableLevel), BlockHashReport>,
}

impl StfsHasher {
	pub fn hash_block(&self, block_data: &[u8]) -> StfsBlockHash {
		let mut hasher = Sha1::new();
		hasher.update(block_data);

		hasher.finalize().into()
	}

	pub fn block_hash_is_valid(&self, block_hash: &[u8], block_data: &[u8]) -> bool {
		self.hash_block(block_data) == block_hash
	}

	pub fn verify_hash_block(
		&mut self,
		package: &StfsPackage,
		stfs_file: &[u8],
		data_block: Block,
		level: HashTableLevel,
	) -> Result<BlockHashReport, StfsError> {
		let mut current_level = Some(package.hash_table_meta.top_table.level());
		let mut current_hash = package.volume_descriptor.top_hash_table_hash();
		let mut current_active_index = package.volume_descriptor.flags().root_active_index();

		while let Some(level) = current_level {
			let hash_block = package.data_block_to_hash_block(data_block, level)?;
			let hash_block = Block(hash_block.0 + (current_active_index as usize));
			let hash_entry = package.block_hash_entry(data_block, level, stfs_file)?;

			if !self.checked_hashes.contains_key(&(hash_block, level)) {
				let hash_block_addr = package.block_to_addr(hash_block) as usize;

				let calculated_hash = self.hash_block(&stfs_file[hash_block_addr..(hash_block_addr + BLOCK_SIZE)]);
				let valid_hash = calculated_hash == hash_entry.block_hash;

				self.checked_hashes.insert(
					(hash_block, level),
					BlockHashReport {
						block: hash_block,
						calculated_hash,
						hash_entry: hash_entry.clone(),
						is_valid: valid_hash,
					},
				);
			}

			match hash_entry.meta {
				crate::HashEntryMeta::LevelFirst(hash_entry_level_first_meta) => {}
				crate::HashEntryMeta::LevelN(hash_entry_level_nmeta) => {}
			}
			current_level = level.previous();
		}

		unreachable!("Somehow made it to a hash tree level that doesn't exist");
	}
}

impl StfsPackage {}
