use crate::error::StfsError;
use crate::hash::HashTableMeta;
use crate::header::StfsVolumeDescriptor;
use crate::io::ReadAt;
use crate::types::*;
use byteorder::BigEndian;
use byteorder::LittleEndian;
use byteorder::ReadBytesExt;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Cursor;

#[derive(Default, Clone, Debug, Serialize)]
pub struct StfsFileEntry {
	pub index: usize,
	pub name: String,
	pub flags: u8,
	pub block_count: usize,
	pub starting_block_num: usize,
	pub path_indicator: u16,
	pub file_size: usize,
	pub created_time_stamp: u32,
	pub access_time_stamp: u32,
	pub file_entry_address: u64,
}

impl StfsFileEntry {
	pub fn is_directory(&self) -> bool {
		self.flags & 2 != 0
	}
}

#[derive(Debug, Serialize)]
pub struct StfsFileTable {
	pub entries: Vec<StfsFileEntry>,
}

#[derive(Debug, Clone)]
pub struct StfsTreeNode {
	pub entry: StfsFileEntry,
	pub children: Vec<StfsTreeNode>,
}

impl StfsFileTable {
	pub fn read<R: ReadAt>(
		source: &R,
		hash_meta: &HashTableMeta,
		stfs_vol: &StfsVolumeDescriptor,
		sex: StfsPackageSex,
	) -> Result<Self, StfsError> {
		let mut entries = Vec::new();
		let mut block = stfs_vol.file_table_block_num;

		for block_idx in 0..(stfs_vol.file_table_block_count as usize) {
			let current_addr = hash_meta.block_to_addr(block as usize, sex);
			let block_data = source.read_at(current_addr..current_addr + BLOCK_SIZE)?;
			let block_data = block_data.as_ref();
			let mut cursor = Cursor::new(block_data);

			for file_entry_idx in 0..0x40usize {
				let entry_offset = file_entry_idx * 0x40;
				let file_entry_address = current_addr as u64 + entry_offset as u64;
				let index = (block_idx * 0x40) + file_entry_idx;

				// Read name (0x28 bytes)
				let name = read_utf8_with_max_len(&block_data[entry_offset..], 0x28);
				cursor.set_position((entry_offset + 0x28) as u64);

				let name_len = cursor.read_u8().map_err(|_| StfsError::ReadError {
					offset: current_addr + entry_offset + 0x28,
					message: "failed to read name_len".into(),
				})?;

				if name_len & 0x3F == 0 {
					continue;
				}

				if name_len == 0 {
					break;
				}

				let block_count = cursor.read_u24::<LittleEndian>()? as usize;

				// Skip 3 bytes padding
				cursor.set_position(cursor.position() + 3);

				let starting_block_num = cursor.read_u24::<LittleEndian>()? as usize;
				let path_indicator = cursor.read_u16::<BigEndian>()?;
				let file_size = cursor.read_u32::<BigEndian>()? as usize;
				let created_time_stamp = cursor.read_u32::<BigEndian>()?;
				let access_time_stamp = cursor.read_u32::<BigEndian>()?;
				let flags = name_len >> 6;

				entries.push(StfsFileEntry {
					index,
					name,
					flags,
					block_count,
					starting_block_num,
					path_indicator,
					file_size,
					created_time_stamp,
					access_time_stamp,
					file_entry_address,
				});
			}

			// Follow block chain to next file table block
			let hash_entry = hash_meta.read_block_hash_entry(source, block as usize, sex, stfs_vol)?;
			block = hash_entry.next_block;
		}

		Ok(StfsFileTable { entries })
	}

	pub fn build_tree(&self) -> StfsTreeNode {
		let root = StfsFileEntry { name: String::new(), ..Default::default() };

		let mut folder_children: HashMap<u16, Vec<usize>> = HashMap::new();

		// Group entries by their path_indicator (parent folder index)
		for (i, entry) in self.entries.iter().enumerate() {
			folder_children.entry(entry.path_indicator).or_default().push(i);
		}

		fn build_children(
			entries: &[StfsFileEntry],
			folder_children: &HashMap<u16, Vec<usize>>,
			parent_index: u16,
		) -> Vec<StfsTreeNode> {
			let Some(child_indices) = folder_children.get(&parent_index) else {
				return Vec::new();
			};

			child_indices
				.iter()
				.map(|&i| {
					let entry = &entries[i];
					let children = if entry.is_directory() {
						build_children(entries, folder_children, entry.index as u16)
					} else {
						Vec::new()
					};
					StfsTreeNode { entry: entry.clone(), children }
				})
				.collect()
		}

		let children = build_children(&self.entries, &folder_children, 0xFFFF);

		StfsTreeNode { entry: root, children }
	}

	pub fn walk_files(&self) -> Vec<(String, StfsFileEntry)> {
		let tree = self.build_tree();
		let mut result = Vec::new();

		fn walk(node: &StfsTreeNode, path: &str, result: &mut Vec<(String, StfsFileEntry)>) {
			for child in &node.children {
				let child_path =
					if path.is_empty() { child.entry.name.clone() } else { format!("{}/{}", path, child.entry.name) };
				if child.entry.is_directory() {
					walk(child, &child_path, result);
				} else {
					result.push((child_path, child.entry.clone()));
				}
			}
		}

		walk(&tree, "", &mut result);
		result
	}
}

fn read_utf8_with_max_len(data: &[u8], len: usize) -> String {
	let end = data[..len].iter().position(|b| *b == 0).unwrap_or(len);
	String::from_utf8(data[..end].to_vec()).expect("failed to convert data to utf8")
}
