//! Named partitions of a device, as the kernel creates their device objects.

use std::ops::Range;

use crate::device::DeviceKind;
use crate::units::ByteCount;
use crate::units::ByteOffset;
use crate::units::SECTOR_SIZE;
use crate::units::byte_range;

/// How the FATX driver mounts the volume of a partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VolumeKind {
	/// A partition of fixed size.
	Standard,
	/// A partition in files that can grow (FILE_DEVICE_PIAF_ALTERNATE_PARTITION). The FATX driver
	/// requires 16 KiB clusters and a 32-bit FAT for it (`fs/fatx/mountvol.c`).
	Alternate,
}

/// One partition device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
	/// The last component of the kernel device name, such as `Partition1`.
	name: &'static str,
	start: ByteOffset,
	length: ByteCount,
	/// The device only accepts raw opens (DO_RAW_MOUNT_ONLY); no file system mounts it.
	raw_only: bool,
	volume: VolumeKind,
}

impl Partition {
	pub const fn new(name: &'static str, start: ByteOffset, length: ByteCount) -> Self {
		Self { name, start, length, raw_only: false, volume: VolumeKind::Standard }
	}

	pub const fn raw(self) -> Self {
		Self { raw_only: true, ..self }
	}

	pub const fn alternate(self) -> Self {
		Self { volume: VolumeKind::Alternate, ..self }
	}

	pub const fn name(&self) -> &'static str {
		self.name
	}

	pub const fn start(&self) -> ByteOffset {
		self.start
	}

	pub const fn length(&self) -> ByteCount {
		self.length
	}

	pub const fn is_raw_only(&self) -> bool {
		self.raw_only
	}

	pub const fn volume(&self) -> VolumeKind {
		self.volume
	}

	/// The bytes of the partition in its device.
	pub fn range(&self) -> Range<u64> {
		byte_range(self.start, self.length)
	}
}

/// The partitions of one device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionMap {
	device: DeviceKind,
	length: ByteCount,
	partitions: Vec<Partition>,
}

impl PartitionMap {
	/// A map of `partitions`, ordered by start and, at one start, longest first.
	pub fn new(device: DeviceKind, length: ByteCount, mut partitions: Vec<Partition>) -> Self {
		partitions.sort_by_key(|partition| (partition.start, std::cmp::Reverse(partition.length)));

		Self { device, length, partitions }
	}

	pub const fn device(&self) -> DeviceKind {
		self.device
	}

	/// The bytes of the device that the map covers.
	pub const fn length(&self) -> ByteCount {
		self.length
	}

	pub const fn sector_size(&self) -> u64 {
		SECTOR_SIZE
	}

	pub const fn sector_count(&self) -> u64 {
		self.length.get() / SECTOR_SIZE
	}

	pub fn partitions(&self) -> &[Partition] {
		&self.partitions
	}

	/// The partition named `name`. Kernel object names compare without regard to case.
	pub fn partition(&self, name: &str) -> Option<&Partition> {
		self.partitions.iter().find(|partition| partition.name.eq_ignore_ascii_case(name))
	}
}
