//! Fixed partition layouts that depend only on the size of the device.

use crate::device::DeviceKind;
use crate::error::LayoutError;
use crate::partition::Partition;
use crate::partition::PartitionMap;
use crate::units::ByteCount;
use crate::units::ByteOffset;
use crate::units::Lba;
use crate::units::SectorCount;
use crate::usb_storage::UsbStorageLayout;

/// First sector of Cache0 on a retail disk (`SATA_DISK_CACHE_PARTITION_START`). The sectors
/// before it hold the devkit table, the security sector and the logo bitmap.
pub const RETAIL_CACHE_START: Lba = Lba::new(0x400);
const RETAIL_CACHE_SECTORS: SectorCount = SectorCount::new(0x40_0000);
const RETAIL_DUMP_SECTORS: SectorCount = SectorCount::new(0x10_7180);
const RETAIL_SYSTEM_SECTORS: SectorCount = SectorCount::new(0x8_0000);
/// First sector of Partition1 on a retail disk: the sum of the fixed partitions before it.
pub const RETAIL_PARTITION1_START: Lba = Lba::new(0x98_7580);

/// The partitions the kernel carves from DumpPartition when no raw dump is present
/// (`SataDiskAssignDumpPartitionBounds`), as offset and length within it.
pub(crate) const DUMP_CHILDREN: [DumpChild; 4] = [
	DumpChild { name: "SystemURLCachePartition", offset: 0, length: 0x0600_0000, raw_only: true },
	DumpChild { name: "TitleURLCachePartition", offset: 0x0600_0000, length: 0x0200_0000, raw_only: true },
	DumpChild { name: "SystemExtPartition", offset: 0x0c00_0000, length: 0x0ce3_0000, raw_only: false },
	DumpChild { name: "SystemAuxPartition", offset: 0x18e3_0000, length: 0x0800_0000, raw_only: false },
];
/// The DumpPartition length that holds every carved partition.
pub(crate) const DUMP_CHILDREN_LENGTH: ByteCount = ByteCount::new(0x20e3_0000);

/// System partition length of a memory unit (`MASS_MEMORY_UNIT_SYSTEM_LENGTH`,
/// `SFCX_MU_SYSTEM_PARTITION_SIZE`).
pub const MEMORY_UNIT_SYSTEM_LENGTH: ByteCount = ByteCount::new(0x7f_f000);

/// Where the memory unit starts in the eMMC (`MMCX_MEMORY_UNIT_OFFSET`).
pub const MMC_MEMORY_UNIT_OFFSET: ByteOffset = ByteOffset::new(0x0300_0000);
/// Offset of the Storage partition in a built-in memory unit.
pub const BUILT_IN_STORAGE_OFFSET: ByteOffset = ByteOffset::new(0x20e3_0000);

/// Where the transfer cable exposes the hard disk (`MASS_CABLE_STARTING_OFFSET`): Partition1.
pub const TRANSFER_CABLE_OFFSET: ByteOffset = RETAIL_PARTITION1_START.byte_offset();

#[derive(Debug, Clone, Copy)]
pub(crate) struct DumpChild {
	pub name: &'static str,
	pub offset: u64,
	pub length: u64,
	pub raw_only: bool,
}

/// The sector counts of a hard disk: the device count IDENTIFY reports (PhysicalDisk), and the
/// addressable count the partitions use (Partition0), which is at most the device count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskSectors {
	device: SectorCount,
	addressable: SectorCount,
}

impl DiskSectors {
	/// The counts the kernel uses (`SataDiskAuthenticateDevice`): the security sector count, when
	/// the sector authenticates, and otherwise the device count. Authentication needs the count
	/// to be at most the device count and not the rejected value; the caller passes `None` when
	/// another check failed. The signature is assumed to verify.
	pub fn new(device: SectorCount, security: Option<SectorCount>) -> Self {
		let addressable = match security {
			Some(security) if security <= device && security.get() != crate::security::REJECTED_SECTOR_COUNT => {
				security
			}
			_ => device,
		};

		Self { device, addressable }
	}

	/// A disk without a usable security sector.
	pub const fn whole(device: SectorCount) -> Self {
		Self { device, addressable: device }
	}

	pub const fn device(&self) -> SectorCount {
		self.device
	}

	pub const fn addressable(&self) -> SectorCount {
		self.addressable
	}
}

/// The layout the kernel gives a device of `kind` and `length` bytes with no other input. The
/// optional inputs of each layout take their absent value: no security sector limit, no
/// key vault sector limit, and a transfer cable that reports the whole disk.
pub fn for_device(kind: DeviceKind, length: ByteCount) -> Result<PartitionMap, LayoutError> {
	if kind == DeviceKind::UsbStorage {
		return Ok(UsbStorageLayout::from_logical_length(length)?.partition_map());
	}
	if kind == DeviceKind::SfcMemoryUnit {
		return sfc_memory_unit(length);
	}

	let sectors = SectorCount::from_bytes(length).ok_or(LayoutError::NotWholeSectors { length })?;
	match kind {
		DeviceKind::RetailHdd => retail_hdd(DiskSectors::whole(sectors)),
		DeviceKind::UsbMemoryUnit => usb_memory_unit(sectors),
		DeviceKind::MmcMemoryUnit => mmc_memory_unit(sectors, MmcSectorLimit::Unlimited),
		DeviceKind::UsbBuiltInMemoryUnit => usb_built_in_memory_unit(sectors, None),
		DeviceKind::TransferCable => transfer_cable(sectors, sectors),
		DeviceKind::DevkitHdd | DeviceKind::UsbStorage | DeviceKind::SfcMemoryUnit => {
			Err(LayoutError::NoFixedLayout { device: kind })
		}
	}
}

/// The retail hard disk layout (`SataDiskInitialize`) of `disk`.
///
/// The cache partitions inside DumpPartition are included, as the kernel creates them when no
/// raw memory dump is stored there.
pub fn retail_hdd(disk: DiskSectors) -> Result<PartitionMap, LayoutError> {
	let DiskSectors { device, addressable } = disk;
	if addressable.get() <= RETAIL_PARTITION1_START.get() {
		return Err(LayoutError::TooSmall {
			device: DeviceKind::RetailHdd,
			length: addressable.bytes(),
			minimum: SectorCount::new(RETAIL_PARTITION1_START.get() + 1).bytes(),
		});
	}

	let mut partitions = whole_disk(device, addressable);
	let mut start = RETAIL_CACHE_START;
	for name in ["Cache0", "Cache1"] {
		partitions.push(sectors(name, start, RETAIL_CACHE_SECTORS));
		start = advance(start, RETAIL_CACHE_SECTORS);
	}

	let dump = sectors("DumpPartition", start, RETAIL_DUMP_SECTORS).raw();
	partitions.extend(dump_children(&dump));
	partitions.push(dump);
	start = advance(start, RETAIL_DUMP_SECTORS);

	partitions.push(sectors("SystemPartition", start, RETAIL_SYSTEM_SECTORS));
	start = advance(start, RETAIL_SYSTEM_SECTORS);

	partitions.push(sectors("Partition1", start, SectorCount::new(addressable.get() - start.get())));

	Ok(PartitionMap::new(DeviceKind::RetailHdd, device.bytes(), partitions))
}

/// A retail disk through the transfer cable: one partition from Partition1 to the `cable`
/// sector count of the security block, on a device of `device` sectors (`MassConfigureTransferCable`).
pub fn transfer_cable(device: SectorCount, cable: SectorCount) -> Result<PartitionMap, LayoutError> {
	let minimum = SectorCount::new(RETAIL_PARTITION1_START.get());
	if cable < minimum {
		return Err(LayoutError::TooSmall {
			device: DeviceKind::TransferCable,
			length: cable.bytes(),
			minimum: minimum.bytes(),
		});
	}
	if device < cable {
		return Err(LayoutError::TooSmall {
			device: DeviceKind::TransferCable,
			length: device.bytes(),
			minimum: cable.bytes(),
		});
	}

	let length = ByteCount::new(cable.bytes().get() - TRANSFER_CABLE_OFFSET.get());
	let partitions = vec![Partition::new("TransferCable", TRANSFER_CABLE_OFFSET, length)];

	Ok(PartitionMap::new(DeviceKind::TransferCable, device.bytes(), partitions))
}

/// A USB memory unit of `device` sectors, as `MassReadCapacityComplete` sizes it: the system
/// partition first, then the data partition to the end.
pub fn usb_memory_unit(device: SectorCount) -> Result<PartitionMap, LayoutError> {
	const MINIMUM: SectorCount = SectorCount::new(0x6000);
	const ROUND: u32 = 0x1_e000;
	const ROUND_RANGE: u32 = 0x1fff;
	const MAXIMUM: SectorCount = SectorCount::new(0x10_0000);

	if device < MINIMUM {
		return Err(LayoutError::TooSmall {
			device: DeviceKind::UsbMemoryUnit,
			length: device.bytes(),
			minimum: MINIMUM.bytes(),
		});
	}

	// Sizes slightly above the standard size are rounded down to it.
	let sectors = if device.get().wrapping_sub(ROUND) <= ROUND_RANGE { SectorCount::new(ROUND) } else { device };
	if sectors > MAXIMUM {
		return Err(LayoutError::TooLarge {
			device: DeviceKind::UsbMemoryUnit,
			length: device.bytes(),
			maximum: MAXIMUM.bytes(),
		});
	}

	let data_start = ByteOffset::new(MEMORY_UNIT_SYSTEM_LENGTH.get());
	let partitions = vec![
		Partition::new("Mu0System", ByteOffset::new(0), MEMORY_UNIT_SYSTEM_LENGTH),
		Partition::new("Mu0", data_start, ByteCount::new(sectors.bytes().get() - data_start.get())),
	];

	Ok(PartitionMap::new(DeviceKind::UsbMemoryUnit, device.bytes(), partitions))
}

/// The NAND memory unit of `length` bytes (`SfcxMuMountStorage`): the data partition, then the
/// system partition at the end. sfcx reads the memory unit with the same split.
pub fn sfc_memory_unit(length: ByteCount) -> Result<PartitionMap, LayoutError> {
	const MINIMUM: ByteCount = ByteCount::new(0xc0_0000);
	// The memory unit is whole flash blocks; the smallest block, of small-block NAND, is 16 KiB.
	const BLOCK: ByteCount = ByteCount::new(0x4000);

	if length < MINIMUM {
		return Err(LayoutError::TooSmall { device: DeviceKind::SfcMemoryUnit, length, minimum: MINIMUM });
	}
	if !length.get().is_multiple_of(BLOCK.get()) {
		return Err(LayoutError::Unaligned { device: DeviceKind::SfcMemoryUnit, length, alignment: BLOCK });
	}

	let system_start = length.get() - MEMORY_UNIT_SYSTEM_LENGTH.get();
	let partitions = vec![
		Partition::new("BuiltInMuSfc", ByteOffset::new(0), ByteCount::new(system_start)),
		Partition::new("BuiltInMuSfcSystem", ByteOffset::new(system_start), MEMORY_UNIT_SYSTEM_LENGTH),
	];

	Ok(PartitionMap::new(DeviceKind::SfcMemoryUnit, length, partitions))
}

/// `HvConsoleInfo->MemoryUnitSectorLimit`, which `MmcxMountMemoryUnit` reads as three states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmcSectorLimit {
	/// 0: the memory unit takes the rest of the eMMC.
	Unlimited,
	/// The memory unit holds at most this many sectors.
	Sectors(SectorCount),
	/// MAXULONG: the memory unit is securely disabled and no partition is created.
	Disabled,
}

impl MmcSectorLimit {
	/// The state the console info field `value` encodes.
	pub const fn from_console_info(value: u32) -> Self {
		match value {
			0 => Self::Unlimited,
			u32::MAX => Self::Disabled,
			sectors => Self::Sectors(SectorCount::new(sectors)),
		}
	}
}

/// The whole eMMC of `device` sectors with the memory unit at [`MMC_MEMORY_UNIT_OFFSET`]
/// (`MmcxMountMemoryUnit`).
pub fn mmc_memory_unit(device: SectorCount, limit: MmcSectorLimit) -> Result<PartitionMap, LayoutError> {
	let limit = match limit {
		MmcSectorLimit::Unlimited => None,
		MmcSectorLimit::Sectors(sectors) => Some(sectors),
		MmcSectorLimit::Disabled => return Err(LayoutError::Disabled { device: DeviceKind::MmcMemoryUnit }),
	};

	let offset_sectors = (MMC_MEMORY_UNIT_OFFSET.get() >> 9) as u32;
	let unit = SectorCount::new(device.get().saturating_sub(offset_sectors));
	let partitions = built_in_partitions(DeviceKind::MmcMemoryUnit, MMC_MEMORY_UNIT_OFFSET, unit, limit)?;

	Ok(PartitionMap::new(DeviceKind::MmcMemoryUnit, device.bytes(), partitions))
}

/// The built-in USB memory unit of `device` sectors (`MassBuiltInMemoryUnitPartitionTable`).
/// `limit` is `HvConsoleInfo->KeyVault28` when it is set.
pub fn usb_built_in_memory_unit(device: SectorCount, limit: Option<SectorCount>) -> Result<PartitionMap, LayoutError> {
	let partitions = built_in_partitions(DeviceKind::UsbBuiltInMemoryUnit, ByteOffset::new(0), device, limit)?;

	Ok(PartitionMap::new(DeviceKind::UsbBuiltInMemoryUnit, device.bytes(), partitions))
}

/// The partitions of a built-in memory unit of `unit` sectors at `base`. The eMMC and the USB
/// built-in memory unit share this table.
fn built_in_partitions(
	device: DeviceKind,
	base: ByteOffset,
	unit: SectorCount,
	limit: Option<SectorCount>,
) -> Result<Vec<Partition>, LayoutError> {
	const MINIMUM: SectorCount = SectorCount::new(0x18_0000);

	let unit = match limit {
		Some(limit) if limit < unit => limit,
		_ => unit,
	};
	if unit < MINIMUM {
		return Err(LayoutError::TooSmall { device, length: unit.bytes(), minimum: MINIMUM.bytes() });
	}

	let at = |offset: u64| ByteOffset::new(base.get() + offset);
	let storage_length = ByteCount::new(unit.bytes().get() - BUILT_IN_STORAGE_OFFSET.get());

	Ok(vec![
		Partition::new("Storage", at(BUILT_IN_STORAGE_OFFSET.get()), storage_length),
		Partition::new("StorageSystem", at(0x0800_0000), ByteCount::new(0x0400_0000)),
		Partition::new("SystemAuxPartition", at(0), ByteCount::new(0x0800_0000)),
		Partition::new("TitleURLCachePartition", at(0x0c00_0000), ByteCount::new(0x0200_0000)).raw(),
		Partition::new("SystemURLCachePartition", at(0x0e00_0000), ByteCount::new(0x0600_0000)).raw(),
		Partition::new("SystemExtPartition", at(0x1400_0000), ByteCount::new(0x0ce3_0000)),
		Partition::new("ReservationPartition", at(0), ByteCount::new(BUILT_IN_STORAGE_OFFSET.get())).raw(),
	])
}

/// PhysicalDisk, the `device` sectors IDENTIFY reports, and Partition0, the `addressable`
/// sectors the partitions use.
pub(crate) fn whole_disk(device: SectorCount, addressable: SectorCount) -> Vec<Partition> {
	vec![
		Partition::new("PhysicalDisk", ByteOffset::new(0), device.bytes()).raw(),
		Partition::new("Partition0", ByteOffset::new(0), addressable.bytes()).raw(),
	]
}

/// The cache partitions inside `dump`, at their fixed offsets whatever its length.
pub(crate) fn dump_children(dump: &Partition) -> impl Iterator<Item = Partition> {
	let start = dump.start().get();

	DUMP_CHILDREN.iter().map(move |child| {
		let partition = Partition::new(child.name, ByteOffset::new(start + child.offset), ByteCount::new(child.length));
		if child.raw_only { partition.raw() } else { partition }
	})
}

pub(crate) fn sectors(name: &'static str, start: Lba, count: SectorCount) -> Partition {
	Partition::new(name, start.byte_offset(), count.bytes())
}

/// The sector after `count` sectors from `start`. Fixed layouts stay far below 2^32 sectors.
fn advance(start: Lba, count: SectorCount) -> Lba {
	Lba::new(start.get() + count.get())
}
