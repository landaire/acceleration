mod common;

use common::render;
use xenon_disk::ByteCount;
use xenon_disk::DeviceKind;
use xenon_disk::DiskSectors;
use xenon_disk::LayoutError;
use xenon_disk::MmcSectorLimit;
use xenon_disk::SectorCount;
use xenon_disk::layout;

/// A 250 GB disk.
const DISK_SECTORS: SectorCount = SectorCount::new(0x1d1c_5970);

#[test]
fn retail_disk_uses_the_security_sector_count() {
	let map = layout::retail_hdd(DiskSectors::new(DISK_SECTORS, Some(SectorCount::new(0x1d1c_0000)))).unwrap();

	insta::assert_snapshot!(render(&map));
}

#[test]
fn retail_disk_ignores_a_security_count_past_the_device() {
	let disk = DiskSectors::new(DISK_SECTORS, Some(SectorCount::new(DISK_SECTORS.get() + 1)));
	let map = layout::retail_hdd(disk).unwrap();

	assert_eq!(map.partition("Partition1").unwrap().range().end, DISK_SECTORS.bytes().get());
}

#[test]
fn retail_disk_needs_a_sector_after_the_system_partition() {
	let error = layout::retail_hdd(DiskSectors::whole(SectorCount::new(0x98_7580))).unwrap_err();

	assert_eq!(
		error,
		LayoutError::TooSmall {
			device: DeviceKind::RetailHdd,
			length: ByteCount::new(0x1_30eb_0000),
			minimum: ByteCount::new(0x1_30eb_0200),
		}
	);
}

#[test]
fn transfer_cable_exposes_partition1_up_to_the_cable_count() {
	let map = layout::transfer_cable(DISK_SECTORS, SectorCount::new(0x1d1c_0000)).unwrap();

	insta::assert_snapshot!(render(&map), @r"
	hard disk through the transfer cable of 0x3a38b2e000 bytes (0x1d1c5970 sectors)
	TransferCable            0x00130eb0000..0x03a38000000 0x03907150000
	");
}

#[test]
fn usb_memory_unit_rounds_a_standard_size_down() {
	let map = layout::usb_memory_unit(SectorCount::new(0x1_e000 + 0x100)).unwrap();

	insta::assert_snapshot!(render(&map), @r"
	USB memory unit of 0x3c20000 bytes (0x1e100 sectors)
	Mu0System                0x00000000000..0x000007ff000 0x000007ff000
	Mu0                      0x000007ff000..0x00003c00000 0x00003401000
	");
}

#[test]
fn usb_memory_unit_size_limits() {
	assert_eq!(
		layout::usb_memory_unit(SectorCount::new(0x5fff)),
		Err(LayoutError::TooSmall {
			device: DeviceKind::UsbMemoryUnit,
			length: ByteCount::new(0xbf_fe00),
			minimum: ByteCount::new(0xc0_0000),
		})
	);
	assert_eq!(
		layout::usb_memory_unit(SectorCount::new(0x10_0001)),
		Err(LayoutError::TooLarge {
			device: DeviceKind::UsbMemoryUnit,
			length: ByteCount::new(0x2000_0200),
			maximum: ByteCount::new(0x2000_0000),
		})
	);
}

#[test]
fn sfc_memory_unit_puts_the_system_partition_last() {
	let map = layout::sfc_memory_unit(ByteCount::new(0x1aca_0000)).unwrap();

	assert_eq!(map.partition("BuiltInMuSfc").unwrap().range(), 0..0x1a4a_1000);
	assert_eq!(map.partition("builtinmusfcsystem").unwrap().range(), 0x1a4a_1000..0x1aca_0000);
}

#[test]
fn sfc_memory_unit_is_whole_blocks() {
	assert_eq!(
		layout::sfc_memory_unit(ByteCount::new(0x1aca_1000)),
		Err(LayoutError::Unaligned {
			device: DeviceKind::SfcMemoryUnit,
			length: ByteCount::new(0x1aca_1000),
			alignment: ByteCount::new(0x4000),
		})
	);
}

#[test]
fn mmc_memory_unit_starts_48_mib_into_the_emmc() {
	let map = layout::mmc_memory_unit(SectorCount::new(0x40_0000), MmcSectorLimit::Unlimited).unwrap();

	insta::assert_snapshot!(render(&map));
}

#[test]
fn mmc_memory_unit_applies_the_sector_limit() {
	let limit = MmcSectorLimit::from_console_info(0x20_0000);
	let map = layout::mmc_memory_unit(SectorCount::new(0x40_0000), limit).unwrap();

	assert_eq!(map.partition("Storage").unwrap().range(), 0x03000000 + 0x20e3_0000..0x0300_0000 + 0x4000_0000);
}

#[test]
fn mmc_memory_unit_disabled_by_the_console_has_no_partitions() {
	let limit = MmcSectorLimit::from_console_info(u32::MAX);

	assert_eq!(
		layout::mmc_memory_unit(SectorCount::new(0x40_0000), limit),
		Err(LayoutError::Disabled { device: DeviceKind::MmcMemoryUnit })
	);
	assert_eq!(MmcSectorLimit::from_console_info(0), MmcSectorLimit::Unlimited);
}

#[test]
fn built_in_memory_unit_needs_the_minimum_sectors() {
	assert_eq!(
		layout::usb_built_in_memory_unit(SectorCount::new(0x40_0000), Some(SectorCount::new(0x17_ffff))),
		Err(LayoutError::TooSmall {
			device: DeviceKind::UsbBuiltInMemoryUnit,
			length: ByteCount::new(0x2fff_fe00),
			minimum: ByteCount::new(0x3000_0000),
		})
	);
}

#[test]
fn usb_built_in_memory_unit_layout() {
	let map = layout::usb_built_in_memory_unit(SectorCount::new(0x3a_0000), None).unwrap();

	insta::assert_snapshot!(render(&map));
}

#[test]
fn devkit_disks_have_no_fixed_layout() {
	assert_eq!(
		layout::for_device(DeviceKind::DevkitHdd, DISK_SECTORS.bytes()),
		Err(LayoutError::NoFixedLayout { device: DeviceKind::DevkitHdd })
	);
}
