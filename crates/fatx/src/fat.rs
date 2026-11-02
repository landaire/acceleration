//! Cluster numbers and the file allocation table.

use std::fmt;
use std::num::NonZeroU32;

/// A data cluster number. Data clusters run from 1; FAT entry 0 is reserved.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClusterNumber(NonZeroU32);

impl ClusterNumber {
	/// `None` for 0, which a directory entry stores when a file has no clusters.
	pub const fn new(value: u32) -> Option<Self> {
		match NonZeroU32::new(value) {
			Some(value) => Some(Self(value)),
			None => None,
		}
	}

	pub const fn get(self) -> u32 {
		self.0.get()
	}

	/// The physically following cluster.
	pub fn following(self) -> Option<Self> {
		self.0.checked_add(1).map(Self)
	}
}

impl fmt::Display for ClusterNumber {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:#x}", self.0)
	}
}

impl fmt::Debug for ClusterNumber {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "ClusterNumber({:#x})", self.0)
	}
}

/// Physically consecutive clusters, all with numbers that fit a `u32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClusterRun {
	first: ClusterNumber,
	count: u32,
}

impl ClusterRun {
	/// `None` when the run would pass the largest cluster number.
	pub fn new(first: ClusterNumber, count: u32) -> Option<Self> {
		let last = u64::from(first.get()) + u64::from(count);

		(last <= u64::from(u32::MAX) + 1).then_some(Self { first, count })
	}

	pub const fn first(&self) -> ClusterNumber {
		self.first
	}

	pub const fn count(&self) -> u32 {
		self.count
	}

	pub fn contains(&self, cluster: ClusterNumber) -> bool {
		cluster >= self.first && u64::from(cluster.get()) < u64::from(self.first.get()) + u64::from(self.count)
	}

	pub fn clusters(&self) -> impl Iterator<Item = ClusterNumber> + use<> {
		let first = u64::from(self.first.get());
		(first..first + u64::from(self.count))
			.map(|cluster| ClusterNumber::new(cluster as u32).expect("runs start at 1"))
	}

	/// Extends the run by the cluster after it.
	pub(crate) fn extend(&mut self) {
		self.count = self.count.checked_add(1).expect("a run never passes the largest cluster number");
	}
}

/// FAT entry width, chosen from the partition length at mount time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatWidth {
	Fat16,
	Fat32,
}

impl FatWidth {
	pub const fn entry_size(self) -> u64 {
		match self {
			Self::Fat16 => 2,
			Self::Fat32 => 4,
		}
	}

	/// Stores `entry` big-endian in `out`, which holds one entry. A 16-bit link must name a
	/// cluster below 0xfff0, which a 16-bit FAT always has.
	pub(crate) fn encode(self, entry: FatEntry, out: &mut [u8]) {
		let value = match entry {
			FatEntry::Free => 0,
			FatEntry::Last => u32::MAX,
			FatEntry::Link(next) => next,
		};

		match self {
			Self::Fat16 => out.copy_from_slice(&(value as u16).to_be_bytes()),
			Self::Fat32 => out.copy_from_slice(&value.to_be_bytes()),
		}
	}
}

/// A decoded FAT entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatEntry {
	Free,
	/// The last cluster of a chain.
	Last,
	/// The next cluster of a chain. The value may not name a data cluster.
	Link(u32),
}

/// The FAT entries of the volume's data clusters, as stored: big-endian.
#[derive(Clone, PartialEq, Eq)]
pub struct Fat {
	width: FatWidth,
	bytes: Vec<u8>,
}

impl Fat {
	pub(crate) fn new(width: FatWidth, bytes: Vec<u8>) -> Self {
		Self { width, bytes }
	}

	pub const fn width(&self) -> FatWidth {
		self.width
	}

	/// Number of entries, including reserved entry 0.
	pub fn len(&self) -> u64 {
		self.bytes.len() as u64 / self.width.entry_size()
	}

	pub fn is_empty(&self) -> bool {
		self.bytes.is_empty()
	}

	/// The entry of `cluster`, or `None` past the end of the table.
	pub fn entry(&self, cluster: ClusterNumber) -> Option<FatEntry> {
		decode_entry(self.width, &self.bytes, cluster)
	}

	pub fn is_free(&self, cluster: ClusterNumber) -> bool {
		self.entry(cluster) == Some(FatEntry::Free)
	}

	/// The table as stored.
	pub(crate) fn bytes(&self) -> &[u8] {
		&self.bytes
	}

	/// Replaces the entry of a cluster the table covers.
	pub(crate) fn set(&mut self, cluster: ClusterNumber, entry: FatEntry) {
		self.set_index(cluster.get() as usize, entry);
	}

	/// Replaces reserved entry 0.
	pub(crate) fn set_reserved(&mut self, entry: FatEntry) {
		self.set_index(0, entry);
	}

	fn set_index(&mut self, index: usize, entry: FatEntry) {
		let size = self.width.entry_size() as usize;
		let offset = index * size;
		self.width.encode(entry, &mut self.bytes[offset..offset + size]);
	}
}

impl fmt::Debug for Fat {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("Fat").field("width", &self.width).field("entries", &self.len()).finish()
	}
}

/// The entry of `cluster` in a FAT stored as `table`, or `None` past the end of the table.
pub(crate) fn decode_entry(width: FatWidth, table: &[u8], cluster: ClusterNumber) -> Option<FatEntry> {
	let size = width.entry_size() as usize;
	let offset = usize::try_from(cluster.get()).ok()?.checked_mul(size)?;

	table.get(offset..offset + size).map(|bytes| decode_bytes(width, bytes))
}

/// The entry stored in `bytes`, one entry of `width`.
pub(crate) fn decode_bytes(width: FatWidth, bytes: &[u8]) -> FatEntry {
	const FREE: u32 = 0;
	const LAST: u32 = u32::MAX;
	const RESERVED_16: u16 = 0xfff0;

	let value = match width {
		FatWidth::Fat16 => {
			let value = u16::from_be_bytes([bytes[0], bytes[1]]);
			// fatxp.h FatxReadFatEntry: reserved 16-bit values sign-extend to 32 bits.
			if value >= RESERVED_16 { value as i16 as i32 as u32 } else { u32::from(value) }
		}
		FatWidth::Fat32 => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
	};

	match value {
		FREE => FatEntry::Free,
		LAST => FatEntry::Last,
		next => FatEntry::Link(next),
	}
}
