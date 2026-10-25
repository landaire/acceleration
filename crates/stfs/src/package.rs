use serde::Serialize;
use std::io::Write;

use crate::error::StfsError;
use crate::file_table::StfsFileEntry;
use crate::file_table::StfsFileTable;
use crate::hash::HashTableMeta;
use crate::header::FileSystem;
use crate::header::XContentHeader;
use crate::io::ReadAt;
use crate::types::*;

#[derive(Debug, Serialize)]
pub struct StfsPackage {
	pub header: XContentHeader,
	pub sex: StfsPackageSex,
	pub hash_table_meta: HashTableMeta,
	pub file_table: StfsFileTable,
}

impl StfsPackage {
	pub fn open<R: ReadAt>(source: &R) -> Result<Self, StfsError> {
		// Read enough for the header (conservatively up to 0x9800 bytes)
		let header_data = source.read_at(0..0x9800)?;
		let header = XContentHeader::parse(header_data.as_ref())?;

		let sex = StfsPackageSex::from_header(&header)?;
		let mut hash_table_meta = HashTableMeta::from_header(&header, sex)?;
		hash_table_meta.read_top_table(source)?;

		let stfs_vol = header.volume_descriptor.stfs_ref();
		let file_table = StfsFileTable::read(source, &hash_table_meta, stfs_vol, sex)?;

		Ok(StfsPackage { header, sex, hash_table_meta, file_table })
	}

	pub fn extract_file<R: ReadAt, W: Write>(
		&self,
		source: &R,
		writer: &mut W,
		entry: &StfsFileEntry,
	) -> Result<(), StfsError> {
		if entry.file_size == 0 {
			return Ok(());
		}

		if entry.flags & 1 != 0 {
			self.extract_consecutive_blocks(source, writer, entry)?;
		} else {
			self.extract_chained_blocks(source, writer, entry)?;
		}

		Ok(())
	}

	fn extract_consecutive_blocks<R: ReadAt, W: Write>(
		&self,
		source: &R,
		writer: &mut W,
		entry: &StfsFileEntry,
	) -> Result<(), StfsError> {
		let mut data_remaining = entry.file_size;
		let mut block = entry.starting_block_num.0;

		// Hash tables interrupt the data blocks, so read each physically contiguous run of blocks separately.
		while data_remaining > 0 {
			let run_start = self.hash_table_meta.block_to_addr(BlockNumber(block), self.sex)?;
			let mut run_len = 0;
			loop {
				run_len += std::cmp::min(BLOCK_SIZE, data_remaining - run_len);
				block += 1;
				if run_len == data_remaining
					|| self.hash_table_meta.block_to_addr(BlockNumber(block), self.sex)? != run_start + run_len
				{
					break;
				}
			}

			let data = source.read_at(run_start..run_start + run_len)?;
			writer.write_all(data.as_ref())?;
			data_remaining -= run_len;
		}

		Ok(())
	}

	fn extract_chained_blocks<R: ReadAt, W: Write>(
		&self,
		source: &R,
		writer: &mut W,
		entry: &StfsFileEntry,
	) -> Result<(), StfsError> {
		let mut data_remaining = entry.file_size;
		let mut block_count = data_remaining / BLOCK_SIZE;
		if !data_remaining.is_multiple_of(BLOCK_SIZE) {
			block_count += 1;
		}

		let stfs_vol = self.header.volume_descriptor.stfs_ref();
		let mut block = entry.starting_block_num;

		for _ in 0..block_count {
			let read_len = std::cmp::min(BLOCK_SIZE, data_remaining);
			let block_address = self.hash_table_meta.block_to_addr(block, self.sex)?;

			let data = source.read_at(block_address..block_address + read_len)?;
			writer.write_all(data.as_ref())?;

			let hash_entry = self.hash_table_meta.read_block_hash_entry(source, block, self.sex, stfs_vol)?;
			block = hash_entry.next_block;
			data_remaining -= read_len;
		}

		Ok(())
	}
}

impl StfsPackageSex {
	pub fn from_header(header: &XContentHeader) -> Result<Self, StfsError> {
		if let FileSystem::STFS(stfs) = &header.volume_descriptor {
			if (!stfs.block_separation) & 1 == 0 { Ok(StfsPackageSex::Female) } else { Ok(StfsPackageSex::Male) }
		} else {
			Err(StfsError::InvalidPackageType)
		}
	}
}
