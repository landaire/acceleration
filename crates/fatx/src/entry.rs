//! FATX_DIRECTORY_ENTRY: one 64-byte directory record, big-endian.

use std::fmt;

use thiserror::Error;

use crate::fat::ClusterNumber;
use crate::time::Timestamp;

pub const DIRECTORY_ENTRY_SIZE: usize = 0x40;
pub const MAXIMUM_FILE_NAME_LENGTH: usize = 42;

/// FileNameLength values that are not name lengths (fatxp.h FATX_DIRENT_*).
const NEVER_USED: u8 = 0x00;
const DELETED: u8 = 0xe5;
const NEVER_USED2: u8 = 0xff;

const NAME_OFFSET: usize = 2;
const FIRST_CLUSTER_OFFSET: usize = 0x2c;
const SIZE_OFFSET: usize = 0x30;
const CREATED_OFFSET: usize = 0x34;
const MODIFIED_OFFSET: usize = 0x38;
const ACCESSED_OFFSET: usize = 0x3c;

/// Characters a file name may not hold, one bit per character code (fatsup.c
/// FatxFatIllegalTable): control characters and `" * + , / : ; < = > ? \ |`.
const ILLEGAL_CHARACTERS: [u32; 8] = [0xffff_ffff, 0xfc00_9c04, 0x1000_0000, 0x1000_0000, 0, 0, 0, 0];

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileAttributes(u8);

impl FileAttributes {
	pub const READ_ONLY: Self = Self(0x01);
	pub const HIDDEN: Self = Self(0x02);
	pub const SYSTEM: Self = Self(0x04);
	pub const DIRECTORY: Self = Self(0x10);
	pub const ARCHIVE: Self = Self(0x20);
	/// FATX_VALID_FILE_ATTRIBUTES.
	pub const VALID: Self = Self(0x37);

	pub const fn from_bits(bits: u8) -> Self {
		Self(bits)
	}

	pub const fn bits(self) -> u8 {
		self.0
	}

	pub const fn contains(self, other: Self) -> bool {
		self.0 & other.0 == other.0
	}

	pub const fn is_directory(self) -> bool {
		self.contains(Self::DIRECTORY)
	}

	/// Whether only attributes the driver accepts are set.
	pub const fn is_valid(self) -> bool {
		self.0 & !Self::VALID.0 == 0
	}
}

impl fmt::Debug for FileAttributes {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "FileAttributes({:#04x})", self.0)
	}
}

/// Where a directory record is: its cluster and its byte offset within the cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryLocation {
	pub cluster: ClusterNumber,
	pub offset: u32,
}

impl fmt::Display for EntryLocation {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "cluster {} offset {:#x}", self.cluster, self.offset)
	}
}

/// Why the driver would skip a record that is neither free nor deleted (dirctrl.c
/// FatxFindNextDirectoryEntry).
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum InvalidEntryReason {
	#[error("name length {length:#x} exceeds the maximum")]
	NameLength { length: u8 },
	#[error("name holds the illegal character {byte:#04x}")]
	IllegalCharacter { byte: u8 },
	#[error("name is . or ..")]
	DotName,
	#[error("attributes {attributes:#04x} hold unsupported bits")]
	Attributes { attributes: u8 },
	/// dirctrl.c:58-65: a name may not make the path longer than 250 bytes.
	#[error("a {length}-byte name makes the path too long; this directory allows {maximum}")]
	PathTooLong { length: u8, maximum: u8 },
}

/// A parsed directory record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryRecord {
	/// The name, one character per byte. A deleted record's name ends at the first 0x00 or 0xff
	/// byte because deletion overwrites the stored length.
	name: String,
	attributes: FileAttributes,
	first_cluster: Option<ClusterNumber>,
	size: u32,

	created: Timestamp,
	modified: Timestamp,
	accessed: Timestamp,

	location: EntryLocation,
	marked_deleted: bool,
	/// The name field bytes after the name are all 0x00 or all 0xff, as a driver or formatting
	/// tool leaves them.
	uniform_padding: bool,
}

impl EntryRecord {
	pub fn name(&self) -> &str {
		&self.name
	}

	pub const fn attributes(&self) -> FileAttributes {
		self.attributes
	}

	pub const fn is_directory(&self) -> bool {
		self.attributes.is_directory()
	}

	/// `None` when the record stores cluster 0: an empty file.
	pub const fn first_cluster(&self) -> Option<ClusterNumber> {
		self.first_cluster
	}

	pub const fn size(&self) -> u32 {
		self.size
	}

	pub const fn created(&self) -> Timestamp {
		self.created
	}

	pub const fn modified(&self) -> Timestamp {
		self.modified
	}

	pub const fn accessed(&self) -> Timestamp {
		self.accessed
	}

	pub const fn location(&self) -> EntryLocation {
		self.location
	}

	/// Whether the record's FileNameLength is the deleted marker 0xe5.
	pub const fn is_marked_deleted(&self) -> bool {
		self.marked_deleted
	}

	pub const fn has_uniform_padding(&self) -> bool {
		self.uniform_padding
	}

	pub(crate) fn has_valid_timestamps(&self) -> bool {
		self.created.is_valid() && self.modified.is_valid() && self.accessed.is_valid()
	}
}

/// What a 64-byte directory slot holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Slot {
	/// FileNameLength 0x00 or 0xff: no record here or after it.
	End,
	Live(EntryRecord),
	Deleted(EntryRecord),
	Invalid(InvalidEntryReason),
	/// A deleted record whose name or attributes cannot be recovered.
	UnreadableDeleted,
}

pub(crate) fn parse_slot(record: &[u8], location: EntryLocation) -> Slot {
	assert_eq!(record.len(), DIRECTORY_ENTRY_SIZE, "a directory slot is 64 bytes");

	let attributes = FileAttributes(record[1]);
	let name_field = &record[NAME_OFFSET..NAME_OFFSET + MAXIMUM_FILE_NAME_LENGTH];
	let (name_length, marked_deleted) = match record[0] {
		NEVER_USED | NEVER_USED2 => return Slot::End,
		DELETED => {
			let length = name_field.iter().position(|byte| matches!(*byte, NEVER_USED | NEVER_USED2));
			(length.unwrap_or(MAXIMUM_FILE_NAME_LENGTH), true)
		}
		length if usize::from(length) > MAXIMUM_FILE_NAME_LENGTH => {
			return Slot::Invalid(InvalidEntryReason::NameLength { length });
		}
		length => (usize::from(length), false),
	};

	let name = &name_field[..name_length];
	if let Err(reason) = validate_name(name).and_then(|()| validate_attributes(attributes)) {
		if marked_deleted {
			return Slot::UnreadableDeleted;
		}
		return Slot::Invalid(reason);
	}

	let padding = &name_field[name_length..];
	let uniform_padding =
		padding.iter().all(|byte| *byte == NEVER_USED) || padding.iter().all(|byte| *byte == NEVER_USED2);
	let record = EntryRecord {
		name: name.iter().map(|byte| char::from(*byte)).collect(),
		attributes,
		first_cluster: ClusterNumber::new(read_u32(record, FIRST_CLUSTER_OFFSET)),
		size: read_u32(record, SIZE_OFFSET),
		created: Timestamp::from_raw(read_u32(record, CREATED_OFFSET)),
		modified: Timestamp::from_raw(read_u32(record, MODIFIED_OFFSET)),
		accessed: Timestamp::from_raw(read_u32(record, ACCESSED_OFFSET)),
		location,
		marked_deleted,
		uniform_padding,
	};

	if marked_deleted { Slot::Deleted(record) } else { Slot::Live(record) }
}

/// What a raw slot holds, as fatsup.c and create.c test its FileNameLength.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotState<'a> {
	/// FileNameLength 0x00 or 0xff: this slot and those after it are free.
	End,
	/// FileNameLength 0xe5: free for a new record.
	Deleted,
	/// A record whose stored name is `name`. The driver compares only records with a name
	/// length of at most 42 bytes.
	Named { name: &'a [u8] },
	/// A record with a name length past 42 bytes, which no name matches.
	Unnamed,
}

pub(crate) fn slot_state(slot: &[u8]) -> SlotState<'_> {
	match slot[0] {
		NEVER_USED | NEVER_USED2 => SlotState::End,
		DELETED => SlotState::Deleted,
		length if usize::from(length) <= MAXIMUM_FILE_NAME_LENGTH => {
			SlotState::Named { name: &slot[NAME_OFFSET..NAME_OFFSET + usize::from(length)] }
		}
		_ => SlotState::Unnamed,
	}
}

/// The fields of a record that fatsup.c FatxUpdateDirectoryEntry writes. It copies only the
/// name's own bytes, so the rest of the name field keeps what the slot held before.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EntryFields<'a> {
	pub name: &'a [u8],
	pub attributes: FileAttributes,
	pub first_cluster: Option<ClusterNumber>,
	pub size: u32,
	pub created: Timestamp,
	pub modified: Timestamp,
	pub accessed: Timestamp,
}

impl<'a> EntryFields<'a> {
	/// The fields of a live record in `slot`.
	pub(crate) fn read(slot: &'a [u8]) -> Self {
		let name_length = usize::from(slot[0]).min(MAXIMUM_FILE_NAME_LENGTH);

		Self {
			name: &slot[NAME_OFFSET..NAME_OFFSET + name_length],
			attributes: FileAttributes(slot[1]),
			first_cluster: ClusterNumber::new(read_u32(slot, FIRST_CLUSTER_OFFSET)),
			size: read_u32(slot, SIZE_OFFSET),
			created: Timestamp::from_raw(read_u32(slot, CREATED_OFFSET)),
			modified: Timestamp::from_raw(read_u32(slot, MODIFIED_OFFSET)),
			accessed: Timestamp::from_raw(read_u32(slot, ACCESSED_OFFSET)),
		}
	}

	/// The slot with these fields stored over it. `name` is at most 42 bytes.
	pub(crate) fn store(&self, slot: &[u8]) -> [u8; DIRECTORY_ENTRY_SIZE] {
		let mut stored: [u8; DIRECTORY_ENTRY_SIZE] = slot.try_into().expect("a directory slot is 64 bytes");
		stored[0] = self.name.len() as u8;
		stored[1] = self.attributes.0;
		stored[NAME_OFFSET..NAME_OFFSET + self.name.len()].copy_from_slice(self.name);
		let first_cluster = self.first_cluster.map_or(0, ClusterNumber::get);
		stored[FIRST_CLUSTER_OFFSET..SIZE_OFFSET].copy_from_slice(&first_cluster.to_be_bytes());
		stored[SIZE_OFFSET..CREATED_OFFSET].copy_from_slice(&self.size.to_be_bytes());
		stored[CREATED_OFFSET..MODIFIED_OFFSET].copy_from_slice(&self.created.raw().to_be_bytes());
		stored[MODIFIED_OFFSET..ACCESSED_OFFSET].copy_from_slice(&self.modified.raw().to_be_bytes());
		stored[ACCESSED_OFFSET..].copy_from_slice(&self.accessed.raw().to_be_bytes());

		stored
	}
}

/// The slot with its FileNameLength set to the deleted marker, as fatsup.c FatxDeleteFile and
/// FatxMarkDirectoryEntryDeleted leave it. The name, first cluster and size stay.
pub(crate) fn mark_deleted(slot: &[u8]) -> [u8; DIRECTORY_ENTRY_SIZE] {
	let mut marked: [u8; DIRECTORY_ENTRY_SIZE] = slot.try_into().expect("a directory slot is 64 bytes");
	marked[0] = DELETED;

	marked
}

/// fatsup.c FatxIsValidFatFileName.
pub(crate) fn validate_name(name: &[u8]) -> Result<(), InvalidEntryReason> {
	if name.is_empty() {
		return Err(InvalidEntryReason::NameLength { length: 0 });
	}

	if name == b"." || name == b".." {
		return Err(InvalidEntryReason::DotName);
	}

	match name.iter().find(|byte| ILLEGAL_CHARACTERS[usize::from(**byte) / 32] & (1 << (**byte % 32)) != 0) {
		Some(byte) => Err(InvalidEntryReason::IllegalCharacter { byte: *byte }),
		None => Ok(()),
	}
}

fn validate_attributes(attributes: FileAttributes) -> Result<(), InvalidEntryReason> {
	if !attributes.is_valid() {
		return Err(InvalidEntryReason::Attributes { attributes: attributes.bits() });
	}

	Ok(())
}

fn read_u32(record: &[u8], offset: usize) -> u32 {
	u32::from_be_bytes([record[offset], record[offset + 1], record[offset + 2], record[offset + 3]])
}

#[cfg(test)]
mod tests {
	use super::*;

	fn location() -> EntryLocation {
		EntryLocation { cluster: ClusterNumber::new(1).unwrap(), offset: 0 }
	}

	fn record(name_length: u8, name: &[u8], padding: u8) -> [u8; DIRECTORY_ENTRY_SIZE] {
		let mut record = [padding; DIRECTORY_ENTRY_SIZE];
		record[0] = name_length;
		record[1] = 0;
		record[NAME_OFFSET..NAME_OFFSET + name.len()].copy_from_slice(name);
		record[FIRST_CLUSTER_OFFSET..].copy_from_slice(&[0; DIRECTORY_ENTRY_SIZE - FIRST_CLUSTER_OFFSET]);
		record
	}

	#[test]
	fn deleted_name_ends_at_padding() {
		let Slot::Deleted(entry) = parse_slot(&record(DELETED, b"save.bin", 0xff), location()) else {
			panic!("expected a deleted record");
		};

		assert_eq!(entry.name(), "save.bin");
		assert!(entry.is_marked_deleted());
		assert!(entry.has_uniform_padding());
	}

	#[test]
	fn live_name_uses_stored_length() {
		let Slot::Live(entry) = parse_slot(&record(4, b"save.bin", 0), location()) else {
			panic!("expected a live record");
		};

		assert_eq!(entry.name(), "save");
		assert!(!entry.has_uniform_padding());
	}

	#[test]
	fn driver_skips_illegal_names() {
		assert_eq!(
			parse_slot(&record(3, b"a:b", 0), location()),
			Slot::Invalid(InvalidEntryReason::IllegalCharacter { byte: b':' })
		);
		assert_eq!(parse_slot(&record(2, b"..", 0), location()), Slot::Invalid(InvalidEntryReason::DotName));
		assert_eq!(
			parse_slot(&record(43, b"a", 0), location()),
			Slot::Invalid(InvalidEntryReason::NameLength { length: 43 })
		);
		assert_eq!(parse_slot(&record(DELETED, b"", 0), location()), Slot::UnreadableDeleted);
	}

	#[test]
	fn end_markers() {
		assert_eq!(parse_slot(&[0; DIRECTORY_ENTRY_SIZE], location()), Slot::End);
		assert_eq!(parse_slot(&[0xff; DIRECTORY_ENTRY_SIZE], location()), Slot::End);
	}
}
