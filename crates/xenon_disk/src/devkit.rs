//! The devkit partition table in sector 0 of Partition0 (`SATA_DISK_DEVKIT_TABLE`,
//! `drivers/sata/disk.c` SataDiskCreateDevkitPartitions). Every field is a big-endian u32.

use std::fmt;

use xenon_io::Checked;

use crate::device::DeviceKind;
use crate::error::DevkitTableError;
use crate::error::DiskWarning;
use crate::layout;
use crate::layout::DUMP_CHILDREN_LENGTH;
use crate::layout::DiskSectors;
use crate::layout::RETAIL_CACHE_START;
use crate::partition::Partition;
use crate::partition::PartitionMap;
use crate::units::ByteCount;
use crate::units::ByteOffset;
use crate::units::Lba;
use crate::units::SECTOR_SIZE;
use crate::units::SectorCount;

/// `SATA_DISK_DEVKIT_TABLE_VERSION`.
pub const DEVKIT_TABLE_VERSION: u32 = 0x0002_0000;
/// `SATA_DISK_DEVKIT_TABLE_MINIMUM`: the smallest `Minimum` field the kernel accepts.
pub const DEVKIT_TABLE_MINIMUM: u32 = 0x05f5_0001;
pub const DEVKIT_TABLE_SIZE: usize = 0x200;
/// Entries after the 8-byte header in one sector.
pub const DEVKIT_TABLE_ENTRIES: usize = 63;
/// Sectors before the first byte a partition may use: the table, the security sector at 0x2000
/// and the logo bitmap that ends at 0x2e00.
pub const DEVKIT_RESERVED_SECTORS: SectorCount = SectorCount::new(0x17);
/// `SATA_DISK_MINIMUM_PIX_STREAM`: the shortest PixStream the kernel splits PixDump for.
const MINIMUM_PIX_STREAM: u64 = 0x0140_0000;

/// A table entry that the kernel assigns to a device. Other entries are skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DevkitSlot {
	Partition1,
	SystemPartition,
	DumpPartition,
	PixDump,
	AltFlash,
	Cache0,
	Cache1,
}

impl DevkitSlot {
	pub const ALL: [Self; 7] = [
		Self::Partition1,
		Self::SystemPartition,
		Self::DumpPartition,
		Self::PixDump,
		Self::AltFlash,
		Self::Cache0,
		Self::Cache1,
	];

	/// The entry index (`DevkitPartitions[]` in SataDiskInitialize).
	pub const fn index(self) -> usize {
		match self {
			Self::Partition1 => 0,
			Self::SystemPartition => 1,
			Self::DumpPartition => 3,
			Self::PixDump => 4,
			Self::AltFlash => 7,
			Self::Cache0 => 8,
			Self::Cache1 => 9,
		}
	}

	/// The device the entry bounds.
	pub const fn name(self) -> &'static str {
		match self {
			Self::Partition1 => "Partition1",
			Self::SystemPartition => "SystemPartition",
			Self::DumpPartition => "DumpPartition",
			Self::PixDump => "PixDump",
			Self::AltFlash => "AltFlash",
			Self::Cache0 => "Cache0",
			Self::Cache1 => "Cache1",
		}
	}

	pub fn from_index(index: usize) -> Option<Self> {
		Self::ALL.into_iter().find(|slot| slot.index() == index)
	}

	/// The slot of the device `name`, compared without regard to case.
	pub fn from_name(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|slot| slot.name().eq_ignore_ascii_case(name))
	}

	/// Whether the kernel creates the device for raw opens only.
	const fn is_raw_only(self) -> bool {
		matches!(self, Self::DumpPartition)
	}
}

impl fmt::Display for DevkitSlot {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.name())
	}
}

/// One entry as stored. A count of zero leaves the device unassigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DevkitEntry {
	pub start: Lba,
	pub count: SectorCount,
}

impl DevkitEntry {
	pub const UNASSIGNED: Self = Self { start: Lba::new(0), count: SectorCount::new(0) };

	pub const fn is_assigned(&self) -> bool {
		self.count.get() != 0
	}

	pub const fn start_offset(&self) -> ByteOffset {
		self.start.byte_offset()
	}

	/// The first sector after the entry, or `None` past the 32-bit sector space.
	pub const fn end(&self) -> Option<Lba> {
		self.start.checked_add(self.count)
	}
}

/// The memory size of the console, which sets the length of MemDump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryDumpSize {
	/// `SATA_DISK_MEMORY_DUMP_LENGTH`.
	Standard,
	/// `SATA_DISK_LARGE_MEMORY_DUMP_LENGTH`, with XBOX_HW_FLAG_LARGE_MEMORY.
	Large,
}

impl MemoryDumpSize {
	pub const fn length(self) -> ByteCount {
		match self {
			Self::Standard => ByteCount::new(0x20e3_0000),
			Self::Large => ByteCount::new(0x40e3_0000),
		}
	}
}

/// The size of a requested partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestSize {
	/// A whole number of sectors, in bytes.
	Bytes(ByteCount),
	/// Every sector after the previous partitions.
	Rest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DevkitRequest {
	pub slot: DevkitSlot,
	pub size: RequestSize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevkitTable {
	minimum: u32,
	entries: [DevkitEntry; DEVKIT_TABLE_ENTRIES],
}

impl DevkitTable {
	/// An empty table. `minimum` must be at least [`DEVKIT_TABLE_MINIMUM`].
	pub fn new(minimum: u32) -> Result<Self, DevkitTableError> {
		check_minimum(minimum)?;

		Ok(Self { minimum, entries: [DevkitEntry::UNASSIGNED; DEVKIT_TABLE_ENTRIES] })
	}

	/// The table of sector 0, when the kernel accepts its header.
	pub fn parse(bytes: &[u8; DEVKIT_TABLE_SIZE]) -> Result<Self, DevkitTableError> {
		let word = |offset: usize| {
			u32::from_be_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
		};

		let version = word(0);
		if version != DEVKIT_TABLE_VERSION {
			return Err(DevkitTableError::VersionMismatch { version, expected: DEVKIT_TABLE_VERSION });
		}
		let minimum = word(4);
		check_minimum(minimum)?;

		let mut entries = [DevkitEntry::UNASSIGNED; DEVKIT_TABLE_ENTRIES];
		for (index, entry) in entries.iter_mut().enumerate() {
			let offset = 8 + index * 8;
			*entry = DevkitEntry { start: Lba::new(word(offset)), count: SectorCount::new(word(offset + 4)) };
		}

		Ok(Self { minimum, entries })
	}

	pub fn to_bytes(&self) -> [u8; DEVKIT_TABLE_SIZE] {
		let mut bytes = [0; DEVKIT_TABLE_SIZE];
		bytes[0..4].copy_from_slice(&DEVKIT_TABLE_VERSION.to_be_bytes());
		bytes[4..8].copy_from_slice(&self.minimum.to_be_bytes());
		for (index, entry) in self.entries.iter().enumerate() {
			let offset = 8 + index * 8;
			bytes[offset..offset + 4].copy_from_slice(&entry.start.get().to_be_bytes());
			bytes[offset + 4..offset + 8].copy_from_slice(&entry.count.get().to_be_bytes());
		}

		bytes
	}

	pub const fn minimum(&self) -> u32 {
		self.minimum
	}

	pub const fn entries(&self) -> &[DevkitEntry; DEVKIT_TABLE_ENTRIES] {
		&self.entries
	}

	/// The entry of `slot`, when the kernel assigns it.
	pub fn assigned(&self, slot: DevkitSlot) -> Option<DevkitEntry> {
		Some(self.entries[slot.index()]).filter(DevkitEntry::is_assigned)
	}

	/// Sets the entry of `slot`; `None` stores an unassigned entry.
	pub fn set(&mut self, slot: DevkitSlot, entry: Option<DevkitEntry>) {
		self.entries[slot.index()] = entry.unwrap_or(DevkitEntry::UNASSIGNED);
	}

	/// A table that places `requests` in order from [`RETAIL_CACHE_START`], each start rounded up
	/// to `alignment` sectors, on a disk of `disk` sectors. Partitions start where the retail
	/// layout starts Cache0, which leaves the security sector, the logo bitmap and the rest of
	/// the first 512 KiB as the retail layout does.
	pub fn pack(
		minimum: u32,
		disk: SectorCount,
		alignment: SectorCount,
		requests: &[DevkitRequest],
	) -> Result<Self, DevkitTableError> {
		if !alignment.get().is_power_of_two() {
			return Err(DevkitTableError::InvalidAlignment { alignment });
		}

		let mut table = Self::new(minimum)?;
		let mut next = u64::from(RETAIL_CACHE_START.get());
		for (index, request) in requests.iter().enumerate() {
			let slot = request.slot;
			if table.assigned(slot).is_some() {
				return Err(DevkitTableError::DuplicateSlot { slot });
			}

			let start = next.next_multiple_of(u64::from(alignment.get()));
			let count = match request.size {
				RequestSize::Bytes(size) => {
					if size.get() == 0 || !size.get().is_multiple_of(SECTOR_SIZE) {
						return Err(DevkitTableError::UnalignedSize { slot, size });
					}
					size.get() / SECTOR_SIZE
				}
				RequestSize::Rest => {
					if index + 1 != requests.len() {
						return Err(DevkitTableError::RestNotLast { slot });
					}
					u64::from(disk.get()).saturating_sub(start)
				}
			};

			let end = start + count;
			if count == 0 {
				return Err(DevkitTableError::NoSectorsLeft { slot });
			}
			if end > u64::from(disk.get()) {
				return Err(DevkitTableError::NoSpace { slot, end, disk });
			}

			// Both fit in 32 bits: they are at most the disk's sector count.
			let entry = DevkitEntry { start: Lba::new(start as u32), count: SectorCount::new(count as u32) };
			table.set(slot, Some(entry));
			next = end;
		}

		table.validate(disk)?;

		Ok(table)
	}

	/// The first problem of [`Self::problems`].
	pub fn validate(&self, disk: SectorCount) -> Result<(), DevkitTableError> {
		match self.problems(disk).into_iter().next() {
			Some(problem) => Err(problem),
			None => Ok(()),
		}
	}

	/// The assigned entries that would not lay out a disk of `disk` sectors: those over the
	/// reserved sectors, past the end, over another entry, and a DumpPartition too short for the
	/// partitions the kernel carves from it.
	pub fn problems(&self, disk: SectorCount) -> Vec<DevkitTableError> {
		let assigned: Vec<(DevkitSlot, DevkitEntry)> =
			DevkitSlot::ALL.into_iter().filter_map(|slot| Some((slot, self.assigned(slot)?))).collect();
		let mut problems = Vec::new();

		for (slot, entry) in &assigned {
			if entry.start.get() < DEVKIT_RESERVED_SECTORS.get() {
				problems.push(DevkitTableError::OverlapsReserved {
					slot: *slot,
					start: entry.start,
					reserved: DEVKIT_RESERVED_SECTORS,
				});
			}
			if entry.end().is_none_or(|end| end.get() > disk.get()) {
				problems.push(DevkitTableError::OutOfBounds {
					slot: *slot,
					start: entry.start,
					count: entry.count,
					disk,
				});
			}
		}

		for (index, (first, first_entry)) in assigned.iter().enumerate() {
			for (second, second_entry) in &assigned[index + 1..] {
				if overlaps(first_entry, second_entry) {
					problems.push(DevkitTableError::Overlap { first: *first, second: *second });
				}
			}
		}

		if let Some(dump) = self.assigned(DevkitSlot::DumpPartition) {
			let length = dump.count.bytes();
			if length < DUMP_CHILDREN_LENGTH {
				problems.push(DevkitTableError::DumpPartitionTooSmall { length, required: DUMP_CHILDREN_LENGTH });
			}
		}

		problems
	}

	/// The partitions the kernel creates from this table on `disk` (`SataDiskInitialize` with
	/// KERNEL_DEVKIT). The problems of [`Self::problems`] are warnings: the kernel assigns the
	/// entries without checking them. The cache partitions are carved from an assigned
	/// DumpPartition whatever its length, as the kernel does once it accepts the table; a short
	/// DumpPartition leaves them extending past its end.
	pub fn partition_map(&self, disk: DiskSectors, memory: MemoryDumpSize) -> Checked<PartitionMap, DiskWarning> {
		let warnings: Vec<DiskWarning> =
			self.problems(disk.device()).into_iter().map(DiskWarning::DevkitTable).collect();
		let mut partitions = layout::whole_disk(disk.device(), disk.addressable());

		for slot in DevkitSlot::ALL {
			let Some(entry) = self.assigned(slot) else {
				continue;
			};

			let partition = layout::sectors(slot.name(), entry.start, entry.count);
			let partition = if slot.is_raw_only() { partition.raw() } else { partition };
			match slot {
				DevkitSlot::DumpPartition => partitions.extend(layout::dump_children(&partition)),
				DevkitSlot::PixDump => partitions.extend(pix_dump_children(&partition, memory)),
				_ => {
					// No partitions inside this one.
				}
			}
			partitions.push(partition);
		}

		Checked::new(PartitionMap::new(DeviceKind::DevkitHdd, disk.device().bytes(), partitions), warnings)
	}
}

/// PixStream and MemDump, when PixDump holds MemDump and at least the minimum PixStream
/// (`SataDiskCreatePixDumpPartitions`).
fn pix_dump_children(pix_dump: &Partition, memory: MemoryDumpSize) -> Vec<Partition> {
	let memory_length = memory.length().get();
	let Some(stream_length) = pix_dump.length().get().checked_sub(memory_length) else {
		return Vec::new();
	};
	if stream_length < MINIMUM_PIX_STREAM {
		return Vec::new();
	}

	let start = pix_dump.start().get();
	vec![
		Partition::new("PixStream", ByteOffset::new(start), ByteCount::new(stream_length)),
		Partition::new("MemDump", ByteOffset::new(start + stream_length), ByteCount::new(memory_length)),
	]
}

fn check_minimum(minimum: u32) -> Result<(), DevkitTableError> {
	if minimum < DEVKIT_TABLE_MINIMUM {
		return Err(DevkitTableError::MinimumTooLow { minimum, required: DEVKIT_TABLE_MINIMUM });
	}

	Ok(())
}

fn overlaps(first: &DevkitEntry, second: &DevkitEntry) -> bool {
	let first_end = u64::from(first.start.get()) + u64::from(first.count.get());
	let second_end = u64::from(second.start.get()) + u64::from(second.count.get());

	u64::from(first.start.get()) < second_end && u64::from(second.start.get()) < first_end
}
