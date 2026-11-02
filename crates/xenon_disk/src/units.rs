//! Sector and byte quantities. The kernel addresses every disk and memory unit in 512-byte
//! sectors with 32-bit sector numbers (`SATA_DISK_SECTOR_SHIFT`, `MASS_SECTOR_SHIFT`).

use std::fmt;
use std::ops::Range;

/// Bytes per sector of every device the layouts describe.
pub const SECTOR_SIZE: u64 = 0x200;
const SECTOR_SHIFT: u32 = 9;

/// A sector number from the start of a device.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Lba(u32);

impl Lba {
	pub const fn new(value: u32) -> Self {
		Self(value)
	}

	pub const fn get(self) -> u32 {
		self.0
	}

	pub const fn byte_offset(self) -> ByteOffset {
		ByteOffset((self.0 as u64) << SECTOR_SHIFT)
	}

	/// The sector `count` sectors after this one, when it is addressable.
	pub const fn checked_add(self, count: SectorCount) -> Option<Self> {
		match self.0.checked_add(count.0) {
			Some(value) => Some(Self(value)),
			None => None,
		}
	}
}

impl fmt::Debug for Lba {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Lba({:#x})", self.0)
	}
}

impl fmt::Display for Lba {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:#x}", self.0)
	}
}

/// A number of sectors.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectorCount(u32);

impl SectorCount {
	pub const fn new(value: u32) -> Self {
		Self(value)
	}

	pub const fn get(self) -> u32 {
		self.0
	}

	pub const fn bytes(self) -> ByteCount {
		ByteCount((self.0 as u64) << SECTOR_SHIFT)
	}

	/// The sectors of `bytes`, when it is a whole number of sectors that a 32-bit count holds.
	pub fn from_bytes(bytes: ByteCount) -> Option<Self> {
		if !bytes.0.is_multiple_of(SECTOR_SIZE) {
			return None;
		}

		u32::try_from(bytes.0 >> SECTOR_SHIFT).ok().map(Self)
	}
}

impl fmt::Debug for SectorCount {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "SectorCount({:#x})", self.0)
	}
}

impl fmt::Display for SectorCount {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:#x}", self.0)
	}
}

/// A byte offset from the start of a device or image.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteOffset(u64);

impl ByteOffset {
	pub const fn new(value: u64) -> Self {
		Self(value)
	}

	pub const fn get(self) -> u64 {
		self.0
	}

	pub const fn checked_add(self, count: ByteCount) -> Option<Self> {
		match self.0.checked_add(count.0) {
			Some(value) => Some(Self(value)),
			None => None,
		}
	}
}

impl fmt::Debug for ByteOffset {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "ByteOffset({:#x})", self.0)
	}
}

impl fmt::Display for ByteOffset {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:#x}", self.0)
	}
}

/// A number of bytes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteCount(u64);

impl ByteCount {
	pub const fn new(value: u64) -> Self {
		Self(value)
	}

	pub const fn get(self) -> u64 {
		self.0
	}
}

impl fmt::Debug for ByteCount {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "ByteCount({:#x})", self.0)
	}
}

impl fmt::Display for ByteCount {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:#x}", self.0)
	}
}

macro_rules! lower_hex {
	($($quantity:ty),*) => {
		$(
			impl fmt::LowerHex for $quantity {
				fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
					fmt::LowerHex::fmt(&self.0, f)
				}
			}
		)*
	};
}

lower_hex!(Lba, SectorCount, ByteOffset, ByteCount);

/// The byte range of `length` bytes from `start`, saturated at the end of the address space.
pub(crate) fn byte_range(start: ByteOffset, length: ByteCount) -> Range<u64> {
	start.0..start.0.saturating_add(length.0)
}
