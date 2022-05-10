use parking_lot::Mutex;
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::Path,
    sync::Arc,
};

use bitflags::bitflags;
use byteorder::{BigEndian, LittleEndian, ReadBytesExt};
use chrono::{DateTime, Utc};
use num_enum::TryFromPrimitive;
use serde::Serialize;
use std::io::{Cursor, Result as IOResult};
use thiserror::Error;

use crate::sparse_reader::SparseReader;

pub type StfsEntryRef = Arc<Mutex<StfsEntry>>;

const INVALID_STR: &'static str = "<INVALID>";

fn input_byte_ref<'a>(cursor: &mut Cursor<&'a [u8]>, input: &'a [u8], size: usize) -> &'a [u8] {
    let position: usize = cursor
        .position()
        .try_into()
        .expect("failed to convert position to usize");
    cursor.set_position(
        (position + size)
            .try_into()
            .expect("failed to convert pos into usize"),
    );
    &input[position..position + size]
}

fn read_utf16_cstr<'a>(cursor: &mut Cursor<&'a [u8]>, input: &'a [u8]) -> String {
    let position: usize = cursor
        .position()
        .try_into()
        .expect("failed to convert position to usize");

    let mut end_of_str_position = None;

    for i in (0..input.len()).step_by(2) {
        if input[position + i] == 0 && input[position + i + 1] == 0 {
            // We found the null terminator
            end_of_str_position = Some(position + i);
            break;
        }
    }

    let end_of_str_position = end_of_str_position.expect("failed to find null terminator");

    cursor.set_position(
        (position + end_of_str_position + 2)
            .try_into()
            .expect("failed to convert pos into usize"),
    );
    let byte_range = &input[position..end_of_str_position];

    let mut utf16_str = Vec::with_capacity(byte_range.len() / 2);
    for chunk in byte_range.chunks(2) {
        utf16_str.push(((chunk[0] as u16) << 8) | chunk[1] as u16);
    }

    String::from_utf16(utf16_str.as_slice()).expect("failed to convert data to utf16")
}

fn read_utf8_with_max_len<'a>(
    cursor: &mut Cursor<&'a [u8]>,
    input: &'a [u8],
    len: usize,
) -> String {
    let position: usize = cursor
        .position()
        .try_into()
        .expect("failed to convert position to usize");

    let mut end_of_str_position = None;

    for i in (0..input.len()).take(len) {
        if input[position + i] == 0 {
            // We found the null terminator
            end_of_str_position = Some(position + i);
            break;
        }
    }

    let end_of_str_position = end_of_str_position.unwrap_or(position + len);

    cursor.set_position(
        (position + len)
            .try_into()
            .expect("failed to convert pos into usize"),
    );
    let byte_range = &input[position..end_of_str_position];
    String::from_utf8(byte_range.to_owned()).expect("failed to convert data to utf8")
}

#[derive(Error, Debug)]
pub enum StfsError {
    #[error("Invalid STFS package header")]
    InvalidHeader,
    #[error("I/O error")]
    IoError(#[from] std::io::Error),
    #[error("Invalid package type")]
    InvalidPackageType,
}

#[derive(Debug, Serialize)]
pub enum PackageType {
    /// User container packages that are created by an Xbox 360 console and
    /// signed by the user's private key.
    Con,
    /// Xbox LIVE-distributed package that is signed by Microsoft's private key.
    Live,
    /// Offline-distributed package that is signed by Microsoft's private key.
    Pirs,
}

impl TryFrom<[u8; 4]> for PackageType {
    type Error = StfsError;

    fn try_from(value: [u8; 4]) -> Result<Self, Self::Error> {
        match &value {
            b"CON " => Ok(PackageType::Con),
            b"LIVE" => Ok(PackageType::Live),
            b"PIRS" => Ok(PackageType::Pirs),
            _ => Err(StfsError::InvalidHeader),
        }
    }
}

#[derive(Debug, Serialize)]
pub enum StfsEntry {
    File(StfsFileEntry),
    Folder {
        entry: StfsFileEntry,
        files: Vec<StfsEntryRef>,
    },
}

impl StfsEntry {
    pub fn name(&self) -> &str {
        match self {
            StfsEntry::File(entry) | StfsEntry::Folder { entry, files: _ } => entry.name.as_str(),
        }
    }

    pub fn entry(&self) -> &StfsFileEntry {
        match self {
            StfsEntry::File(entry) | StfsEntry::Folder { entry, files: _ } => entry,
        }
    }
}

#[derive(Debug, Serialize, Copy, Clone)]
pub enum StfsPackageSex {
    Female = 0,
    Male,
}

impl StfsPackageSex {
    /// The "block step" depends on the package's "sex". This basically determines
    /// which hash tables are used.
    const fn block_step(&self) -> [usize; 2] {
        match self {
            StfsPackageSex::Female => [0xAB, 0x718F],
            StfsPackageSex::Male => [0xAC, 0x723A],
        }
    }
}

impl<'a> TryFrom<&XContentHeader<'a>> for StfsPackageSex {
    type Error = StfsError;

    fn try_from(header: &XContentHeader) -> Result<Self, Self::Error> {
        if let FileSystem::STFS(stfs) = &header.volume_descriptor {
            if (!stfs.block_separation) & 1 == 0 {
                Ok(StfsPackageSex::Female)
            } else {
                Ok(StfsPackageSex::Male)
            }
        } else {
            Err(StfsError::InvalidPackageType)
        }
    }
}
#[derive(Default, Debug, Serialize)]
struct HashEntry<'a> {
    block_hash: &'a [u8],
    status: u8,
    next_block: u32,
}

#[derive(Default, Debug, Serialize)]
pub struct HashTableMeta<'a> {
    pub block_step: [usize; 2],
    pub tables_per_level: [usize; 3],
    pub top_table: HashTable<'a>,
    pub first_table_address: usize,
}

impl<'a> HashTableMeta<'a> {
    pub fn parse(
        data: &'a [u8],
        sex: StfsPackageSex,
        header: &XContentHeader,
    ) -> Result<Self, StfsError> {
        let mut meta = HashTableMeta::default();

        meta.block_step = sex.block_step();

        // Address of the first hash table in the package comes right after the header
        meta.first_table_address = ((header.header_size as usize) + 0x0FFF) & 0xFFFF_F000;

        let stfs_vol = header.volume_descriptor.stfs_ref();

        let allocated_block_count = stfs_vol.allocated_block_count as usize;
        meta.tables_per_level[0] = ((allocated_block_count as usize) / HASHES_PER_HASH_TABLE)
            + if (allocated_block_count as usize) % HASHES_PER_HASH_TABLE != 0 {
                1
            } else {
                0
            };

        meta.tables_per_level[1] = (meta.tables_per_level[1] / HASHES_PER_HASH_TABLE)
            + if meta.tables_per_level[1] % HASHES_PER_HASH_TABLE != 0
                && allocated_block_count > HASHES_PER_HASH_TABLE
            {
                1
            } else {
                0
            };

        meta.tables_per_level[2] = (meta.tables_per_level[2] / HASHES_PER_HASH_TABLE)
            + if meta.tables_per_level[2] % HASHES_PER_HASH_TABLE != 0
                && allocated_block_count > DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]
            {
                1
            } else {
                0
            };

        meta.top_table.level = header.root_hash_table_level()?;
        meta.top_table.true_block_number =
            meta.compute_backing_hash_block_number_for_level(0, meta.top_table.level, sex);

        let base_address = (meta.top_table.true_block_number << 0xC) + meta.first_table_address;
        meta.top_table.address_in_file =
            base_address + (((stfs_vol.block_separation as usize) & 2) << 0xB);

        meta.top_table.entry_count = (allocated_block_count as usize)
            / DATA_BLOCKS_PER_HASH_TREE_LEVEL[meta.top_table.level as usize];

        if (allocated_block_count > DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]
            && allocated_block_count % DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] != 0)
            || (allocated_block_count > HASHES_PER_HASH_TABLE
                && allocated_block_count % HASHES_PER_HASH_TABLE != 0)
        {
            meta.top_table.entry_count += 1;
        }

        meta.top_table.entries.reserve(meta.top_table.entry_count);

        let mut reader = Cursor::new(data);
        reader.set_position(meta.top_table.address_in_file as u64);
        for _ in 0..meta.top_table.entry_count {
            let mut entry = HashEntry {
                block_hash: input_byte_ref(&mut reader, data, 0x14),
                status: reader
                    .read_u8()
                    .expect("failed to read hash table entry status"),
                next_block: reader
                    .read_u24::<BigEndian>()
                    .expect("failed to read hash table entry next_block")
                    as u32,
            };

            meta.top_table.entries.push(entry);
        }

        Ok(meta)
    }

    pub fn compute_backing_hash_block_number_for_level(
        &self,
        block: usize,
        level: HashTableLevel,
        sex: StfsPackageSex,
    ) -> usize {
        match level {
            HashTableLevel::First => self.compute_first_level_backing_hash_block_number(block, sex),
            HashTableLevel::Second => {
                self.compute_second_level_backing_hash_block_number(block, sex)
            }
            HashTableLevel::Third => self.compute_third_level_backing_hash_block_number(),
        }
    }

    pub fn compute_first_level_backing_hash_block_number(
        &self,
        block: usize,
        sex: StfsPackageSex,
    ) -> usize {
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

    pub fn compute_second_level_backing_hash_block_number(
        &self,
        block: usize,
        sex: StfsPackageSex,
    ) -> usize {
        if block < DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] {
            self.block_step[0]
        } else {
            (1 << (sex as u8)) + (block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]) * self.block_step[1]
        }
    }

    pub fn compute_third_level_backing_hash_block_number(&self) -> usize {
        self.block_step[1]
    }
}

const HASHES_PER_HASH_TABLE: usize = 0xAA;
const HASHES_PER_HASH_TABLE_LEVEL: [usize; 3] = [
    HASHES_PER_HASH_TABLE,
    HASHES_PER_HASH_TABLE * HASHES_PER_HASH_TABLE,
    HASHES_PER_HASH_TABLE * HASHES_PER_HASH_TABLE * HASHES_PER_HASH_TABLE,
];
const DATA_BLOCKS_PER_HASH_TREE_LEVEL: [usize; 3] = [
    1,
    HASHES_PER_HASH_TABLE,
    HASHES_PER_HASH_TABLE * HASHES_PER_HASH_TABLE,
];

#[derive(Debug, Serialize)]
pub struct StfsPackage<'a> {
    #[serde(skip)]
    input: &'a [u8],

    pub header: XContentHeader<'a>,
    pub sex: StfsPackageSex,
    pub hash_table_meta: HashTableMeta<'a>,
    pub files: StfsEntryRef,
}

impl<'a> TryFrom<&'a [u8]> for StfsPackage<'a> {
    type Error = StfsError;

    fn try_from(input: &'a [u8]) -> Result<Self, Self::Error> {
        let mut cursor = Cursor::new(input);
        let xcontent_header = xcontent_header_parser(&mut cursor, input)?;
        // TODO: Don't unwrap
        let package_sex = StfsPackageSex::try_from(&xcontent_header).unwrap();
        let hash_table_meta = HashTableMeta::parse(input, package_sex, &xcontent_header)?;

        let mut package = StfsPackage {
            input,
            header: xcontent_header,
            sex: package_sex,
            hash_table_meta,
            files: Arc::new(Mutex::new(StfsEntry::Folder {
                entry: Default::default(),
                files: Default::default(),
            })),
        };

        package.read_files(input);

        Ok(package)
    }
}

impl<'a> StfsPackage<'a> {
    pub fn extract_file(&self, path: &Path, entry: StfsFileEntry) -> std::io::Result<()> {
        let mut output_file = std::fs::File::create(path)?;
        if entry.file_size == 0 {
            return Ok(());
        }

        let mut mappings = Vec::new();

        let start_address = self.block_to_addr(entry.starting_block_num) as usize;

        let mut next_address = start_address;
        let mut data_remaining = entry.file_size;

        // Check if we can read consecutive blocks
        if entry.flags & 1 != 0 {
            let blocks_until_hash_table = (self
                .hash_table_meta
                .compute_first_level_backing_hash_block_number(entry.starting_block_num, self.sex)
                + self.hash_table_meta.block_step[0])
                - ((start_address - self.hash_table_meta.first_table_address) >> 0xC);

            if entry.block_count <= blocks_until_hash_table {
                mappings.push(&self.input[start_address..(start_address + entry.file_size)]);
            } else {
                drop(start_address);

                // The file is broken up by hash tables
                while data_remaining > 0 {
                    let read_len = std::cmp::min(HASHES_PER_HASH_TABLE * 0x1000, data_remaining);

                    mappings.push(&self.input[next_address..(next_address + read_len)]);

                    let data_read = mappings.last().unwrap().len();
                    data_remaining -= data_read;
                    next_address += data_read;
                    next_address += self.hash_table_skip_for_address(next_address)
                }
            }
        } else {
            let mut data_remaining = entry.file_size;

            // This file does not have all-consecutive blocks
            let mut block_count = data_remaining / 0x1000;
            if data_remaining % 0x1000 != 0 {
                block_count += 1;
            }

            let mut block = entry.starting_block_num;
            for _ in 0..block_count {
                let read_len = std::cmp::min(0x1000, data_remaining);

                let block_address = self.block_to_addr(block) as usize;
                mappings.push(&self.input[block_address..(block_address + read_len)]);

                let hash_entry = self.block_hash_entry(block, self.input);
                block = hash_entry.next_block as usize;
                data_remaining -= read_len;
            }
        }

        let mut reader = SparseReader::new(mappings.as_ref());
        let mut data = Vec::new();
        reader
            .read_to_end(&mut data)
            .expect("failed to read STFS file");
        output_file
            .write(data.as_slice())
            .expect("failed to write to file output");

        Ok(())
    }

    fn hash_table_skip_for_address(&self, table_address: usize) -> usize {
        // Convert the address to a true block number
        let mut block_number = (table_address - self.hash_table_meta.first_table_address) >> 0xC;

        // Check if it's the first hash table
        if block_number == 0 {
            return 0x1000 << self.sex as usize;
        }

        // Check if it's the level 3 or above table
        if block_number == self.hash_table_meta.block_step[1] {
            return 0x3000 << self.sex as usize;
        } else if block_number > self.hash_table_meta.block_step[1] {
            block_number -= self.hash_table_meta.block_step[1] + (1 << self.sex as usize);
        }

        // Check if it's at a level 2 table
        if block_number == self.hash_table_meta.block_step[0]
            || block_number % self.hash_table_meta.block_step[1] == 0
        {
            return 0x2000 << self.sex as usize;
        }

        // Assume it's the level 0 table
        return 0x1000 << self.sex as usize;
    }

    fn block_hash_entry(&self, block: usize, input: &'a [u8]) -> HashEntry {
        let stfs_vol = self.header.volume_descriptor.stfs_ref();
        let mut reader = Cursor::new(input);
        if block > stfs_vol.allocated_block_count as usize {
            panic!(
                "Reference to illegal block number: {:#x} ({:#x} allocated)",
                block, stfs_vol.allocated_block_count
            );
        }

        reader.set_position(self.block_hash_address(block, input));
        HashEntry {
            block_hash: input_byte_ref(&mut reader, input, 0x14),
            status: reader
                .read_u8()
                .expect("failed to read hash table entry status"),
            next_block: reader
                .read_u24::<BigEndian>()
                .expect("failed to read hash table entry next_block")
                as u32,
        }
    }

    fn block_hash_address(&self, block: usize, input: &'a [u8]) -> u64 {
        let stfs_vol = self.header.volume_descriptor.stfs_ref();
        if block > stfs_vol.allocated_block_count as usize {
            panic!(
                "Reference to illegal block number: {:#x} ({:#x} allocated)",
                block, stfs_vol.allocated_block_count
            );
        }

        let mut hash_addr = (self
            .hash_table_meta
            .compute_first_level_backing_hash_block_number(block, self.sex)
            << 0xC)
            + self.hash_table_meta.first_table_address;
        // 0x18 here is the size of the HashEntry structure
        hash_addr += (block % HASHES_PER_HASH_TABLE) * 0x18;
        match self.hash_table_meta.top_table.level {
            HashTableLevel::First => {
                hash_addr as u64 + (((stfs_vol.block_separation as u64) & 2) << 0xB)
            }
            HashTableLevel::Second => {
                hash_addr as u64
                    + ((self.hash_table_meta.top_table.entries
                        [block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[1]]
                        .status as u64
                        & 0x40)
                        << 6)
            }
            HashTableLevel::Third => {
                let mut reader = Cursor::new(input);
                let first_level_offset = ((self.hash_table_meta.top_table.entries
                    [block / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]]
                    .status as u64
                    & 0x40)
                    << 6);

                let position = (self
                    .hash_table_meta
                    .compute_second_level_backing_hash_block_number(block, self.sex)
                    << 0xC)
                    + self.hash_table_meta.first_table_address
                    + first_level_offset as usize
                    + ((block % DATA_BLOCKS_PER_HASH_TREE_LEVEL[1]) * 0x18);
                reader.set_position(position as u64 + 0x14);

                hash_addr as u64
                    + ((reader.read_u8().unwrap_or_else(|_| {
                        panic!("failed to read hash entry status byte at {:#x}", position)
                    }) as u64
                        & 0x40)
                        << 0x6)
            }
        }
    }

    fn read_files(&mut self, input: &'a [u8]) {
        let stfs_vol = self.header.volume_descriptor.stfs_ref();
        let mut reader = Cursor::new(input);
        let mut block = stfs_vol.file_table_block_num;
        let mut folders = HashMap::<u16, StfsEntryRef>::new();
        let mut files = Vec::new();
        // Inject a fake root folder
        folders.insert(
            0xffff,
            Arc::new(Mutex::new(StfsEntry::Folder {
                entry: StfsFileEntry::default(),
                files: Vec::new(),
            })),
        );

        for block_idx in 0..(stfs_vol.file_table_block_count as usize) {
            let current_addr = self.block_to_addr(block as usize);
            reader.set_position(current_addr);

            for file_entry_idx in 0..0x40 {
                let mut entry = StfsFileEntry::default();
                entry.file_entry_address = current_addr + (file_entry_idx as u64 * 0x40);
                entry.index = (block_idx * 0x40) + file_entry_idx;

                entry.name = read_utf8_with_max_len(&mut reader, input, 0x28);
                let name_len = reader.read_u8().unwrap_or_else(|_| {
                    panic!("failed to read name_len at {:#x}", entry.file_entry_address)
                });
                if name_len & 0x3F == 0 {
                    // Continue to the next entry
                    reader.set_position(entry.file_entry_address + 0x40);
                    continue;
                }

                if name_len == 0 {
                    break;
                }

                entry.block_count = reader
                    .read_u24::<LittleEndian>()
                    .expect("failed to read blocks_for_file")
                    as usize;

                reader.set_position(reader.position() + 3);

                entry.starting_block_num = reader
                    .read_u24::<LittleEndian>()
                    .expect("failed to read blocks_for_file")
                    as usize;
                entry.path_indicator = reader
                    .read_u16::<BigEndian>()
                    .expect("failed to read blocks_for_file");
                entry.file_size = reader
                    .read_u32::<BigEndian>()
                    .expect("failed to read file_size") as usize;
                entry.created_time_stamp = reader
                    .read_u32::<BigEndian>()
                    .expect("failed to read created_time_stamp");
                entry.access_time_stamp = reader
                    .read_u32::<BigEndian>()
                    .expect("failed to read access_time_stamp");
                entry.flags = name_len >> 6;

                if entry.flags & 2 != 0 {
                    let entry_idx = entry.index;
                    let folder = Arc::new(Mutex::new(StfsEntry::Folder {
                        entry,
                        files: Vec::new(),
                    }));
                    folders.insert(entry_idx as u16, folder.clone());
                    files.push(folder.clone());
                } else {
                    files.push(Arc::new(Mutex::new(StfsEntry::File(entry))));
                }
            }

            block = self.block_hash_entry(block as usize, input).next_block;
        }

        // Associate each file with the folder it needs to be in
        for file in files.drain(..) {
            if let StfsEntry::File(entry) | StfsEntry::Folder { entry, files: _ } = &*file.lock() {
                let cached_entry = folders.get(&entry.path_indicator);
                if let Some(entry) = cached_entry {
                    if let StfsEntry::Folder { entry: _, files } = &mut *entry.lock() {
                        files.push(file.clone());
                    }
                } else {
                    panic!(
                        "Corrupt STFS file: missing folder index {:#x}",
                        entry.path_indicator
                    );
                }
            }
        }

        self.files = folders.remove(&0xffff).expect("no root file entry");
    }

    fn block_to_addr(&self, block: usize) -> u64 {
        if block > 2usize.pow(24) - 1 {
            panic!("invalid block: {:#x}", block);
        }

        (self.compute_data_block_num(block) << 0xC)
            + self.hash_table_meta.first_table_address as u64
    }

    fn compute_data_block_num(&self, block: usize) -> u64 {
        let addr = ((((block + HASHES_PER_HASH_TABLE) / HASHES_PER_HASH_TABLE)
            << (self.sex as usize))
            + block) as u64;
        if block < HASHES_PER_HASH_TABLE {
            addr
        } else if block < DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] {
            addr + (((addr + DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] as u64)
                / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2] as u64)
                << self.sex as usize)
        } else {
            ((1 << self.sex as usize)
                + ((addr as usize
                    + ((block + DATA_BLOCKS_PER_HASH_TREE_LEVEL[2])
                        / DATA_BLOCKS_PER_HASH_TREE_LEVEL[2]))
                    << self.sex as usize)) as u64
        }
    }
}

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

#[derive(Debug, Serialize)]
pub struct HashTable<'a> {
    level: HashTableLevel,
    true_block_number: usize,
    entry_count: usize,
    address_in_file: usize,
    entries: Vec<HashEntry<'a>>,
}

impl<'a> Default for HashTable<'a> {
    fn default() -> Self {
        Self {
            level: HashTableLevel::First,
            true_block_number: Default::default(),
            entry_count: Default::default(),
            entries: Default::default(),
            address_in_file: Default::default(),
        }
    }
}

#[derive(Debug, Serialize, Copy, Clone)]
pub enum HashTableLevel {
    First,
    Second,
    Third,
}

fn certificate_parser<'a>(
    cursor: &mut Cursor<&'a [u8]>,
    input: &'a [u8],
) -> Result<Certificate<'a>, StfsError> {
    let pubkey_cert_size = cursor.read_u16::<BigEndian>()?;
    let mut owner_console_id = [0u8; 5];
    cursor.read_exact(&mut owner_console_id)?;

    let owner_console_part_number = input_byte_ref(cursor, input, 0x11);
    let owner_console_part_number = std::str::from_utf8(
        &owner_console_part_number[..owner_console_part_number
            .iter()
            .position(|b| *b == 0x0)
            .unwrap_or(owner_console_part_number.len())],
    )
    .unwrap_or(INVALID_STR);

    let owner_console_type = cursor.read_u32::<BigEndian>()?;
    let console_type_flags = ConsoleTypeFlags::from_bits(owner_console_type & 0xFFFFFFFC);
    let owner_console_type = ConsoleType::try_from((owner_console_type & 0x3) as u8).ok();

    let date_generation = input_byte_ref(cursor, input, 0x8);
    let date_generation = std::str::from_utf8(date_generation).unwrap_or(INVALID_STR);

    let public_exponent = cursor.read_u32::<BigEndian>()?;

    let public_modulus = input_byte_ref(cursor, input, 0x80);
    let certificate_signature = input_byte_ref(cursor, input, 0x100);
    let signature = input_byte_ref(cursor, input, 0x80);

    Ok(Certificate {
        pubkey_cert_size,
        owner_console_id,
        owner_console_part_number,
        owner_console_type,
        console_type_flags,
        date_generation,
        public_exponent,
        public_modulus,
        certificate_signature,
        signature,
    })
}

fn xcontent_header_parser<'a>(
    cursor: &mut Cursor<&'a [u8]>,
    input: &'a [u8],
) -> Result<XContentHeader<'a>, StfsError> {
    let mut package_type = [0u8; 4];
    cursor.read_exact(&mut package_type)?;
    let package_type = PackageType::try_from(package_type)?;

    let certificate = if let package_type = PackageType::Con {
        Some(certificate_parser(cursor, input)?)
    } else {
        None
    };

    let (input, package_signature) =
        if matches!(package_type, PackageType::Live | PackageType::Pirs) {
            let sig = input_byte_ref(cursor, input, 0x100);
            (input, Some(sig))
        } else {
            (input, None)
        };

    cursor.set_position(0x22c);

    let mut license_data = [LicenseEntry::default(); 16];
    for i in 0..license_data.len() {
        let license = cursor.read_u64::<BigEndian>()?;
        license_data[i].ty = LicenseType::try_from(
            u16::try_from(license >> 48).expect("failed to convert license type to u16"),
        )
        .expect("invalid LicenseType");
        license_data[i].data = license & 0xFFFFFFFFFFFF;
        license_data[i].bits = cursor.read_u32::<BigEndian>()?;
        license_data[i].flags = cursor.read_u32::<BigEndian>()?;
    }

    let header_hash = input_byte_ref(cursor, input, 0x14);
    let header_size = cursor.read_u32::<BigEndian>()?;

    let content_type =
        ContentType::try_from(cursor.read_u32::<BigEndian>()?).expect("invalid content type");
    let metadata_version = cursor.read_u32::<BigEndian>()?;
    let content_size = cursor.read_u64::<BigEndian>()?;
    let media_id = cursor.read_u32::<BigEndian>()?;
    let version = cursor.read_u32::<BigEndian>()?;
    let base_version = cursor.read_u32::<BigEndian>()?;
    let title_id = cursor.read_u32::<BigEndian>()?;
    let platform = cursor.read_u8()?;
    let executable_type = cursor.read_u8()?;
    let disc_number = cursor.read_u8()?;
    let disc_in_set = cursor.read_u8()?;
    let savegame_id = cursor.read_u32::<BigEndian>()?;

    let mut console_id = [0u8; 5];
    cursor.read_exact(&mut console_id)?;

    let mut profile_id = [0u8; 8];
    cursor.read_exact(&mut profile_id)?;

    // read the file system type
    cursor.set_position(0x3a9);
    let filesystem_type =
        FileSystemType::try_from(cursor.read_u32::<BigEndian>()?).expect("invalid filesystem type");

    let volume_descriptor = match filesystem_type {
        FileSystemType::STFS => {
            cursor.set_position(0x379);
            FileSystem::STFS(StfsVolumeDescriptor::parse(cursor, input)?)
        }
        FileSystemType::SVOD => FileSystem::SVOD(SvodVolumeDescriptor::parse(cursor, input)?),
        _ => panic!("Invalid filesystem type"),
    };

    let data_file_count = cursor.read_u32::<BigEndian>()?;
    let data_file_combined_size = cursor.read_u64::<BigEndian>()?;

    let content_metadata = match content_type {
        ContentType::AvatarItem => {
            cursor.set_position(0x3d9);
            Some(ContentMetadata::AvatarItem(AvatarAssetInformation::parse(
                cursor, input,
            )?))
        }
        ContentType::Video => {
            cursor.set_position(0x3d9);
            Some(ContentMetadata::Video(MediaInformation::parse(
                cursor, input,
            )?))
        }
        _ => None,
    };

    cursor.set_position(0x3fd);

    let device_id = input_byte_ref(cursor, input, 0x14);

    let display_name = read_utf16_cstr(cursor, input);

    cursor.set_position(0xD11);
    let display_description = read_utf16_cstr(cursor, input);

    cursor.set_position(0x1611);
    let publisher_name = read_utf16_cstr(cursor, input);

    cursor.set_position(0x1691);
    let title_name = read_utf16_cstr(cursor, input);

    cursor.set_position(0x1711);
    let transfer_flags = cursor.read_u8()?;

    let thumbnail_image_size = cursor.read_u32::<BigEndian>()? as usize;
    let title_thumbnail_image_size = cursor.read_u32::<BigEndian>()? as usize;

    let thumbnail_image = input_byte_ref(cursor, input, thumbnail_image_size);
    cursor.set_position(0x571a);

    let title_image = input_byte_ref(cursor, input, title_thumbnail_image_size);
    cursor.set_position(0x971a);

    let mut installer_type = None;
    let mut installer_meta = None;
    if ((header_size + 0xFFF) & 0xFFFFF000) - 0x971A > 0x15F4 {
        installer_type = Some(
            InstallerType::try_from(cursor.read_u32::<BigEndian>()?)
                .expect("invalid InstallerType"),
        );
        installer_meta = match *installer_type.as_ref().unwrap() {
            InstallerType::SystemUpdate | InstallerType::TitleUpdate => {
                let installer_base_version = Version::from(cursor.read_u32::<BigEndian>()?);
                let installer_version = Version::from(cursor.read_u32::<BigEndian>()?);
                Some(InstallerMeta::FullInstaller(FullInstallerMeta {
                    installer_base_version,
                    installer_version,
                }))
            }
            InstallerType::SystemUpdateProgressCache
            | InstallerType::TitleUpdateProgressCache
            | InstallerType::TitleContentProgressCache => {
                let resume_state =
                    OnlineContentResumeState::try_from(cursor.read_u32::<BigEndian>()?)
                        .expect("invalid resume state");
                let current_file_index = cursor.read_u32::<BigEndian>()?;
                let current_file_offset = cursor.read_u64::<BigEndian>()?;
                let bytes_processed = cursor.read_u64::<BigEndian>()?;

                let high_date_time = cursor.read_u32::<BigEndian>()?;
                let low_date_time = cursor.read_u32::<BigEndian>()?;

                // TODO: Fix
                let last_modified = Utc::now();

                Some(InstallerMeta::InstallerProgressCache(
                    InstallerProgressCache {
                        resume_state,
                        current_file_index,
                        current_file_offset,
                        bytes_processed,
                        last_modified,
                        cab_resume_data: todo!("need to implement CAB resume data"),
                    },
                ));
            }
            _ => {
                // anything else is ok
                None
            }
        }
    }

    let enabled = false;
    Ok(XContentHeader {
        package_type,
        certificate,
        package_signature,
        license_data,
        header_hash,
        header_size,
        content_type,
        metadata_version,
        content_size,
        media_id,
        version,
        base_version,
        title_id,
        platform,
        executable_type,
        disc_number,
        disc_in_set,
        savegame_id,
        console_id,
        profile_id,
        volume_descriptor,
        filesystem_type,
        enabled,
        data_file_count,
        data_file_combined_size,
        device_id,
        display_name,
        display_description,
        publisher_name,
        title_name,
        transfer_flags,
        thumbnail_image_size,
        thumbnail_image,
        title_thumbnail_image_size,
        title_image,
        installer_type,
        installer_meta,
        content_metadata,
    })
}

#[derive(Debug, Serialize)]
pub struct XContentHeader<'a> {
    pub package_type: PackageType,
    /// Only present in console-signed packages
    pub certificate: Option<Certificate<'a>>,
    /// Only present in strong-signed packages
    pub package_signature: Option<&'a [u8]>,

    pub license_data: [LicenseEntry; 0x10],
    pub header_hash: &'a [u8],
    pub header_size: u32,
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
    pub profile_id: [u8; 8],
    pub volume_descriptor: FileSystem<'a>,
    pub filesystem_type: FileSystemType,
    /// Only in PEC -- not sure what this represents. This always needs to be set to 1
    pub enabled: bool,

    // Start metadata v1
    pub data_file_count: u32,
    pub data_file_combined_size: u64,
    pub device_id: &'a [u8],
    pub display_name: String,
    pub display_description: String,
    pub publisher_name: String,
    pub title_name: String,
    pub transfer_flags: u8,
    pub thumbnail_image_size: usize,
    pub thumbnail_image: &'a [u8],
    pub title_thumbnail_image_size: usize,
    pub title_image: &'a [u8],
    pub installer_type: Option<InstallerType>,
    pub installer_meta: Option<InstallerMeta<'a>>,
    pub content_metadata: Option<ContentMetadata<'a>>,
}

impl<'a> XContentHeader<'a> {
    /// Returns which hash table level the root hash is in
    fn root_hash_table_level(&self) -> Result<HashTableLevel, StfsError> {
        if let FileSystem::STFS(volume_descriptor) = &self.volume_descriptor {
            let level = if volume_descriptor.allocated_block_count as usize <= HASHES_PER_HASH_TABLE
            {
                HashTableLevel::First
            } else if volume_descriptor.allocated_block_count as usize
                <= HASHES_PER_HASH_TABLE_LEVEL[1]
            {
                HashTableLevel::Second
            } else if volume_descriptor.allocated_block_count as usize
                <= HASHES_PER_HASH_TABLE_LEVEL[2]
            {
                HashTableLevel::Third
            } else {
                return Err(StfsError::InvalidHeader);
            };

            Ok(level)
        } else {
            Err(StfsError::InvalidPackageType)
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AvatarAssetInformation<'a> {
    subcategory: AssetSubcategory,
    colorizable: u32,
    guid: &'a [u8],
    skeleton_version: SkeletonVersion,
}

impl<'a> AvatarAssetInformation<'a> {
    fn parse(
        cursor: &mut Cursor<&'a [u8]>,
        input: &'a [u8],
    ) -> Result<AvatarAssetInformation<'a>, StfsError> {
        // This data is little endian for some reason
        let subcategory = AssetSubcategory::try_from(cursor.read_u32::<LittleEndian>()?)
            .expect("invalid avatar asset subcategory");
        let colorizable = cursor.read_u32::<LittleEndian>()?;
        let guid = input_byte_ref(cursor, input, 0x10);
        let skeleton_version =
            SkeletonVersion::try_from(cursor.read_u8()?).expect("invalid skeleton version");

        Ok(AvatarAssetInformation {
            subcategory,
            colorizable,
            guid,
            skeleton_version,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct MediaInformation<'a> {
    series_id: &'a [u8],
    season_id: &'a [u8],
    season_number: u16,
    episode_number: u16,
}

impl<'a> MediaInformation<'a> {
    fn parse(
        cursor: &mut Cursor<&'a [u8]>,
        input: &'a [u8],
    ) -> Result<MediaInformation<'a>, StfsError> {
        let series_id = input_byte_ref(cursor, input, 0x10);
        let season_id = input_byte_ref(cursor, input, 0x10);
        let season_number = cursor.read_u16::<BigEndian>()?;
        let episode_number = cursor.read_u16::<BigEndian>()?;

        Ok(MediaInformation {
            series_id,
            season_id,
            season_number,
            episode_number,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct InstallerProgressCache<'a> {
    resume_state: OnlineContentResumeState,
    current_file_index: u32,
    current_file_offset: u64,
    bytes_processed: u64,
    last_modified: DateTime<Utc>,
    cab_resume_data: &'a [u8],
}

#[derive(Debug, Serialize)]
pub struct FullInstallerMeta {
    installer_base_version: Version,
    installer_version: Version,
}

#[derive(Debug, Serialize)]
pub enum InstallerMeta<'a> {
    FullInstaller(FullInstallerMeta),
    InstallerProgressCache(InstallerProgressCache<'a>),
}

#[derive(Debug, Serialize)]
pub struct Certificate<'a> {
    pubkey_cert_size: u16,
    owner_console_id: [u8; 5],
    owner_console_part_number: &'a str,
    owner_console_type: Option<ConsoleType>,
    console_type_flags: Option<ConsoleTypeFlags>,
    date_generation: &'a str,
    public_exponent: u32,
    public_modulus: &'a [u8],
    certificate_signature: &'a [u8],
    signature: &'a [u8],
}

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u8)]
enum ConsoleType {
    DevKit = 1,
    Retail = 2,
}

bitflags! {
    #[derive(Serialize)]
    struct ConsoleTypeFlags: u32 {
        const TESTKIT = 0x40000000;
        const RECOVERY_GENERATED = 0x80000000;
    }
}

#[derive(Debug, Serialize, Clone, Copy, TryFromPrimitive)]
#[repr(u16)]
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

#[derive(Default, Debug, Serialize, Clone, Copy)]
pub struct LicenseEntry {
    ty: LicenseType,
    data: u64,
    bits: u32,
    flags: u32,
}

#[derive(Debug, Serialize)]
pub enum ContentMetadata<'a> {
    AvatarItem(AvatarAssetInformation<'a>),
    Video(MediaInformation<'a>),
}

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u32)]
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
    MarketPlaceContent = 2,
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

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u32)]
pub enum InstallerType {
    None = 0,
    SystemUpdate = 0x53555044,
    TitleUpdate = 0x54555044,
    SystemUpdateProgressCache = 0x50245355,
    TitleUpdateProgressCache = 0x50245455,
    TitleContentProgressCache = 0x50245443,
}

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u32)]
enum OnlineContentResumeState {
    FileHeadersNotReady = 0x46494C48,
    NewFolder = 0x666F6C64,
    NewFolderResumeAttempt1 = 0x666F6C31,
    NewFolderResumeAttempt2 = 0x666F6C32,
    NewFolderResumeAttemptUnknown = 0x666F6C3F,
    NewFolderResumeAttemptSpecific = 0x666F6C40,
}
#[derive(Debug, Serialize)]
pub enum XContentFlags {
    MetadataIsPEC = 1,
    MetadataSkipRead = 2,
    MetadataDontFreeThumbnails = 4,
}

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u32)]
pub enum FileSystemType {
    STFS = 0,
    SVOD,
    FATX,
}

#[derive(Debug, Serialize)]
pub enum FileSystem<'a> {
    STFS(StfsVolumeDescriptor<'a>),
    SVOD(SvodVolumeDescriptor<'a>),
}

impl<'a> FileSystem<'a> {
    pub fn stfs_ref(&self) -> &StfsVolumeDescriptor<'a> {
        if let Self::STFS(volume_descriptor) = self {
            volume_descriptor
        } else {
            panic!("FileSystem is not an StfsVolumeDescriptor")
        }
    }

    pub fn svod_ref(&self) -> &SvodVolumeDescriptor<'a> {
        if let Self::SVOD(volume_descriptor) = self {
            volume_descriptor
        } else {
            panic!("FileSystem is not an SvodVolumeDescriptor")
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StfsVolumeDescriptor<'a> {
    size: u8,
    reserved: u8,
    block_separation: u8,
    file_table_block_count: u16,
    /// This is encoded as a 24-bit integer
    file_table_block_num: u32,
    top_hash_table_hash: &'a [u8],
    allocated_block_count: u32,
    unallocated_block_count: u32,
}

impl<'a> StfsVolumeDescriptor<'a> {
    fn parse(
        cursor: &mut Cursor<&'a [u8]>,
        input: &'a [u8],
    ) -> Result<StfsVolumeDescriptor<'a>, StfsError> {
        Ok(StfsVolumeDescriptor {
            size: cursor.read_u8()?,
            reserved: cursor.read_u8()?,
            block_separation: cursor.read_u8()?,
            file_table_block_count: cursor.read_u16::<LittleEndian>()?,
            file_table_block_num: cursor.read_u24::<LittleEndian>()?,
            top_hash_table_hash: input_byte_ref(cursor, input, 0x14),
            allocated_block_count: cursor.read_u32::<BigEndian>()?,
            unallocated_block_count: cursor.read_u32::<BigEndian>()?,
        })
    }
}

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u32)]
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

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u8)]
enum SkeletonVersion {
    Nxe = 1,
    Natal,
    NxeAndNatal,
}

#[derive(Debug, Serialize)]
enum AssetGender {
    Male = 1,
    Female,
    Both,
}

#[derive(Debug, Serialize)]
pub struct SvodVolumeDescriptor<'a> {
    size: u8,
    block_cache_element_count: u8,
    worker_thread_processor: u8,
    worker_thread_priority: u8,
    root_hash: &'a [u8],
    flags: u8,
    /// Encoded as an int24
    data_block_count: u32,
    /// Encoded as an int24
    data_block_offset: u32,
    reserved: [u8; 5],
}

impl<'a> SvodVolumeDescriptor<'a> {
    fn parse(
        cursor: &mut Cursor<&'a [u8]>,
        input: &'a [u8],
    ) -> Result<SvodVolumeDescriptor<'a>, StfsError> {
        let size = cursor.read_u8()?;
        let block_cache_element_count = cursor.read_u8()?;
        let worker_thread_processor = cursor.read_u8()?;
        let worker_thread_priority = cursor.read_u8()?;
        let root_hash = input_byte_ref(cursor, input, 0x14);
        let flags = cursor.read_u8()?;
        let data_block_count = cursor.read_u24::<BigEndian>()?;
        let data_block_offset = cursor.read_u24::<BigEndian>()?;
        let mut reserved = [0u8; 5];
        cursor.read_exact(&mut reserved)?;

        Ok(SvodVolumeDescriptor {
            size,
            block_cache_element_count,
            worker_thread_processor,
            worker_thread_priority,
            root_hash,
            flags,
            data_block_count,
            data_block_offset,
            reserved,
        })
    }
}
