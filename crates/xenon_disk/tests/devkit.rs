mod common;

use common::render;
use xenon_disk::ByteCount;
use xenon_disk::DevkitEntry;
use xenon_disk::DevkitRequest;
use xenon_disk::DevkitSlot;
use xenon_disk::DevkitTable;
use xenon_disk::DevkitTableError;
use xenon_disk::DiskSectors;
use xenon_disk::DiskWarning;
use xenon_disk::Lba;
use xenon_disk::MemoryDumpSize;
use xenon_disk::RequestSize;
use xenon_disk::SectorCount;
use xenon_disk::devkit::DEVKIT_TABLE_MINIMUM;

/// The first 16 bytes of sector 0 of a devkit disk. The rest of the sector is not known and is
/// zero here.
const KNOWN_TABLE_PREFIX: [u8; 16] =
	[0x00, 0x02, 0x00, 0x00, 0x53, 0x08, 0x00, 0x03, 0x01, 0x68, 0x75, 0x80, 0x1b, 0x31, 0xe3, 0xf0];
/// The sector count Partition1 of the known table ends at.
const KNOWN_DISK: SectorCount = SectorCount::new(0x0168_7580 + 0x1b31_e3f0);

fn known_sector() -> [u8; 0x200] {
	let mut sector = [0; 0x200];
	sector[..16].copy_from_slice(&KNOWN_TABLE_PREFIX);
	sector
}

fn entry(start: u32, count: u32) -> Option<DevkitEntry> {
	Some(DevkitEntry { start: Lba::new(start), count: SectorCount::new(count) })
}

#[test]
fn parses_the_known_devkit_table() {
	let table = DevkitTable::parse(&known_sector()).unwrap();

	assert_eq!(table.minimum(), 0x5308_0003);
	assert_eq!(table.assigned(DevkitSlot::Partition1), entry(0x0168_7580, 0x1b31_e3f0));
	assert_eq!(table.assigned(DevkitSlot::SystemPartition), None);
	assert_eq!(table.to_bytes(), known_sector());
}

#[test]
fn known_table_lays_out_partition1_to_the_end_of_the_disk() {
	let table = DevkitTable::parse(&known_sector()).unwrap();
	let map = table.partition_map(DiskSectors::whole(KNOWN_DISK), MemoryDumpSize::Standard);

	assert!(map.warnings().is_empty());
	insta::assert_snapshot!(render(map.value()), @r"
	development kit hard disk of 0x3934b2e000 bytes (0x1c9a5970 sectors)
	PhysicalDisk             0x00000000000..0x03934b2e000 0x03934b2e000 raw
	Partition0               0x00000000000..0x03934b2e000 0x03934b2e000 raw
	Partition1               0x002d0eb0000..0x03934b2e000 0x03663c7e000
	");
}

#[test]
fn every_entry_round_trips() {
	let mut sector = [0; 0x200];
	sector[..8].copy_from_slice(&KNOWN_TABLE_PREFIX[..8]);
	for (index, byte) in sector[8..].iter_mut().enumerate() {
		*byte = (index * 7 + 3) as u8;
	}

	let table = DevkitTable::parse(&sector).unwrap();

	assert_eq!(table.to_bytes(), sector);
	assert_eq!(table.entries()[62].start, Lba::new(u32::from_be_bytes([0x93, 0x9a, 0xa1, 0xa8])));
}

#[test]
fn rejects_another_version() {
	let mut sector = known_sector();
	sector[1] = 0x01;

	assert_eq!(
		DevkitTable::parse(&sector),
		Err(DevkitTableError::VersionMismatch { version: 0x0001_0000, expected: 0x0002_0000 })
	);
}

#[test]
fn rejects_a_minimum_below_the_kernel_minimum() {
	let mut sector = known_sector();
	sector[4..8].copy_from_slice(&0x05f5_0000u32.to_be_bytes());

	assert_eq!(
		DevkitTable::parse(&sector),
		Err(DevkitTableError::MinimumTooLow { minimum: 0x05f5_0000, required: DEVKIT_TABLE_MINIMUM })
	);
}

#[test]
fn rejects_a_blank_sector() {
	assert_eq!(
		DevkitTable::parse(&[0; 0x200]),
		Err(DevkitTableError::VersionMismatch { version: 0, expected: 0x0002_0000 })
	);
}

#[test]
fn packs_requests_in_order() {
	let requests = [
		DevkitRequest { slot: DevkitSlot::Cache0, size: RequestSize::Bytes(ByteCount::new(0x8000_0000)) },
		DevkitRequest { slot: DevkitSlot::Cache1, size: RequestSize::Bytes(ByteCount::new(0x8000_0000)) },
		DevkitRequest { slot: DevkitSlot::DumpPartition, size: RequestSize::Bytes(ByteCount::new(0x20e3_0000)) },
		DevkitRequest { slot: DevkitSlot::SystemPartition, size: RequestSize::Bytes(ByteCount::new(0x1000_0000)) },
		DevkitRequest { slot: DevkitSlot::PixDump, size: RequestSize::Bytes(ByteCount::new(0x6000_0000)) },
		DevkitRequest { slot: DevkitSlot::AltFlash, size: RequestSize::Bytes(ByteCount::new(0x0400_0200)) },
		DevkitRequest { slot: DevkitSlot::Partition1, size: RequestSize::Rest },
	];
	let table = DevkitTable::pack(DEVKIT_TABLE_MINIMUM, KNOWN_DISK, SectorCount::new(8), &requests).unwrap();

	let round_trip = DevkitTable::parse(&table.to_bytes()).unwrap();
	assert_eq!(round_trip, table);

	let map = table.partition_map(DiskSectors::whole(KNOWN_DISK), MemoryDumpSize::Standard);
	assert!(map.warnings().is_empty());
	insta::assert_snapshot!(render(map.value()));
}

#[test]
fn large_memory_moves_the_memory_dump() {
	let mut table = DevkitTable::new(DEVKIT_TABLE_MINIMUM).unwrap();
	table.set(DevkitSlot::PixDump, entry(0x400, 0x30_0000));

	let map = table.partition_map(DiskSectors::whole(KNOWN_DISK), MemoryDumpSize::Large).into_parts().0;

	assert_eq!(map.partition("PixStream").unwrap().range(), 0x8_0000..0x8_0000 + 0x6000_0000 - 0x40e3_0000);
	assert_eq!(map.partition("MemDump").unwrap().range().end, 0x8_0000 + 0x6000_0000);
}

#[test]
fn a_short_dump_partition_still_holds_its_cache_partitions() {
	let mut table = DevkitTable::new(DEVKIT_TABLE_MINIMUM).unwrap();
	table.set(DevkitSlot::DumpPartition, entry(0x400, 0x8_0000));

	let (map, warnings) = table.partition_map(DiskSectors::whole(KNOWN_DISK), MemoryDumpSize::Standard).into_parts();

	assert_eq!(
		warnings,
		[DiskWarning::DevkitTable(DevkitTableError::DumpPartitionTooSmall {
			length: ByteCount::new(0x1000_0000),
			required: ByteCount::new(0x20e3_0000),
		})]
	);
	assert_eq!(map.partition("SystemAuxPartition").unwrap().range(), 0x8_0000 + 0x18e3_0000..0x8_0000 + 0x20e3_0000);
	assert_eq!(map.partition("SystemURLCachePartition").unwrap().range(), 0x8_0000..0x8_0000 + 0x0600_0000);
}

#[test]
fn a_short_pix_dump_is_not_split() {
	let mut table = DevkitTable::new(DEVKIT_TABLE_MINIMUM).unwrap();
	table.set(DevkitSlot::PixDump, entry(0x400, (0x20e3_0000 + 0x13f_fe00) / 0x200));

	let map = table.partition_map(DiskSectors::whole(KNOWN_DISK), MemoryDumpSize::Standard).into_parts().0;

	assert!(map.partition("PixStream").is_none());
	assert!(map.partition("MemDump").is_none());
}

#[test]
fn pack_rejects_invalid_requests() {
	let pack = |requests: &[DevkitRequest]| {
		DevkitTable::pack(DEVKIT_TABLE_MINIMUM, KNOWN_DISK, SectorCount::new(8), requests).unwrap_err()
	};
	let bytes = |slot, size| DevkitRequest { slot, size: RequestSize::Bytes(ByteCount::new(size)) };
	let rest = |slot| DevkitRequest { slot, size: RequestSize::Rest };

	assert_eq!(
		pack(&[bytes(DevkitSlot::Cache0, 0x1001)]),
		DevkitTableError::UnalignedSize { slot: DevkitSlot::Cache0, size: ByteCount::new(0x1001) }
	);
	assert_eq!(
		pack(&[bytes(DevkitSlot::Cache0, 0)]),
		DevkitTableError::UnalignedSize { slot: DevkitSlot::Cache0, size: ByteCount::new(0) }
	);
	assert_eq!(
		pack(&[bytes(DevkitSlot::Cache0, 0x1000), bytes(DevkitSlot::Cache0, 0x1000)]),
		DevkitTableError::DuplicateSlot { slot: DevkitSlot::Cache0 }
	);
	assert_eq!(
		pack(&[rest(DevkitSlot::Partition1), bytes(DevkitSlot::Cache0, 0x1000)]),
		DevkitTableError::RestNotLast { slot: DevkitSlot::Partition1 }
	);
	assert_eq!(
		pack(&[bytes(DevkitSlot::Partition1, KNOWN_DISK.bytes().get())]),
		DevkitTableError::NoSpace {
			slot: DevkitSlot::Partition1,
			end: 0x400 + u64::from(KNOWN_DISK.get()),
			disk: KNOWN_DISK
		}
	);
	assert_eq!(
		pack(&[bytes(DevkitSlot::Partition1, (u64::from(KNOWN_DISK.get()) - 0x400) * 0x200), rest(DevkitSlot::Cache0)]),
		DevkitTableError::NoSectorsLeft { slot: DevkitSlot::Cache0 }
	);
	assert_eq!(
		pack(&[bytes(DevkitSlot::DumpPartition, 0x1000_0000)]),
		DevkitTableError::DumpPartitionTooSmall {
			length: ByteCount::new(0x1000_0000),
			required: ByteCount::new(0x20e3_0000)
		}
	);
	assert_eq!(
		DevkitTable::pack(DEVKIT_TABLE_MINIMUM, KNOWN_DISK, SectorCount::new(3), &[]).unwrap_err(),
		DevkitTableError::InvalidAlignment { alignment: SectorCount::new(3) }
	);
	assert_eq!(
		DevkitTable::pack(0x0500_0000, KNOWN_DISK, SectorCount::new(8), &[]).unwrap_err(),
		DevkitTableError::MinimumTooLow { minimum: 0x0500_0000, required: DEVKIT_TABLE_MINIMUM }
	);
}

#[test]
fn validation_finds_overlaps_bounds_and_the_reserved_sectors() {
	let mut table = DevkitTable::new(DEVKIT_TABLE_MINIMUM).unwrap();
	table.set(DevkitSlot::Partition1, entry(0x10, 0x100));
	table.set(DevkitSlot::SystemPartition, entry(0x80, 0x100));
	table.set(DevkitSlot::Cache0, entry(KNOWN_DISK.get() - 1, 2));

	assert_eq!(
		table.problems(KNOWN_DISK),
		[
			DevkitTableError::OverlapsReserved {
				slot: DevkitSlot::Partition1,
				start: Lba::new(0x10),
				reserved: SectorCount::new(0x17),
			},
			DevkitTableError::OutOfBounds {
				slot: DevkitSlot::Cache0,
				start: Lba::new(KNOWN_DISK.get() - 1),
				count: SectorCount::new(2),
				disk: KNOWN_DISK,
			},
			DevkitTableError::Overlap { first: DevkitSlot::Partition1, second: DevkitSlot::SystemPartition },
		]
	);
	assert_eq!(table.validate(KNOWN_DISK), Err(table.problems(KNOWN_DISK)[0]));

	let warnings = table.partition_map(DiskSectors::whole(KNOWN_DISK), MemoryDumpSize::Standard).into_parts().1;
	assert_eq!(warnings.len(), 3);
	assert!(matches!(warnings[2], DiskWarning::DevkitTable(DevkitTableError::Overlap { .. })));
}

#[test]
fn slots_are_found_by_name_and_skipped_entries_have_none() {
	assert_eq!(DevkitSlot::from_name("pixdump"), Some(DevkitSlot::PixDump));
	assert_eq!(DevkitSlot::from_name("Partition0"), None);
	for skipped in [2, 5, 6, 10, 62] {
		assert_eq!(DevkitSlot::from_index(skipped), None);
	}
}
