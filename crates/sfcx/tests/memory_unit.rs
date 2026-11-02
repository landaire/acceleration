use std::collections::BTreeMap;
use std::io;
use std::ops::Range;

use sfcx::AsyncMemoryUnitSource;
use sfcx::FlashImage;
use sfcx::FormatSelection;
use sfcx::MemoryUnit;
use sfcx::MemoryUnitSource;
use sfcx::MuBlockIndex;
use sfcx::OpenMemoryUnit;
use sfcx::RawWarning;
use sfcx::ReadAt;
use sfcx::io::SliceSource;
use sfcx::io::drive;
use sfcx::raw::BlockIndex;
use sfcx::raw::Geometry;
use sfcx::raw::RAW_SECTOR_SIZE;
use sfcx::raw::RawKind;
use sfcx::raw::RawLayout;
use sfcx::raw::SECTOR_DATA_SIZE;
use sfcx::raw::SPARE_SIZE;
use sfcx::raw::SectorIndex;
use sfcx::raw::sector_ecc;

const SECTORS_PER_BLOCK: usize = 0x100;
const SECTORS_PER_PAGE: usize = 8;
const PAGE_SIZE: u64 = 0x1000;
const MU_BLOCK_SIZE: u64 = 0x20000;
const CONFIGURED_BLOCKS: usize = 512;
/// 112 blocks past the configured flash: 7 reserved, 5 kept free, 100 for data.
const PHYSICAL_BLOCKS: usize = 624;
const DATA: u8 = 40;
const COMMIT: u8 = 41;

/// Pages are found through the page map of the last committed page: a later copy of a logical
/// page supersedes an earlier one, pages after the last commit are ignored, a secondary block
/// falls back to its primary block, the newest sequence number holds a logical block, and pages
/// that were never written read as erased.
#[test]
fn pages_resolve_as_the_kernel_reads_them() {
	let mut flash = Flash::default();

	// Logical block 0: logical page 0 written twice, then an uncommitted page.
	flash.page(600, 0, Page::primary(0, 1, DATA), &[0], 0xa0);
	flash.page(600, 1, Page::primary(0, 1, DATA), &[0, 1], 0xa1);
	flash.page(600, 2, Page::primary(0, 1, COMMIT), &[0, 1, 0], 0xa2);
	flash.page(600, 3, Page::primary(0, 1, DATA), &[0, 1, 0, 2], 0xee);

	// Logical block 1: a secondary block holds a newer copy of page 1.
	flash.page(601, 0, Page::primary(1, 10, DATA), &[0], 0xb0);
	flash.page(601, 1, Page::primary(1, 10, COMMIT), &[0, 1], 0xb1);
	flash.page(602, 0, Page::secondary(1, 11, 601, COMMIT), &[1], 0xc1);

	// Logical block 2: the older copy is scanned last.
	flash.page(603, 0, Page::primary(2, 7, COMMIT), &[0], 0xe0);
	flash.page(604, 0, Page::primary(2, 5, COMMIT), &[0], 0xd0);

	// A block marked bad is not scanned.
	flash.page(605, 0, Page::primary(3, 30, COMMIT), &[0], 0x55);
	flash.mark_bad(605);

	let unit = flash.mount().deny_warnings().unwrap();
	assert_eq!(unit.size(), 100 * MU_BLOCK_SIZE);
	assert_eq!(unit.partitions().data, 0..0x48_1000);
	assert_eq!(unit.partitions().system, 0x48_1000..0xc8_0000);

	let read = |page: u64| {
		let bytes = unit.read(&flash, page * PAGE_SIZE..(page + 1) * PAGE_SIZE).unwrap().deny_warnings().unwrap();
		assert!(bytes.iter().all(|byte| *byte == bytes[0]), "page {page:#x} is filled with one value");
		bytes[0]
	};
	assert_eq!(read(0), 0xa2);
	assert_eq!(read(1), 0xa1);
	assert_eq!(read(2), 0xff);
	assert_eq!(read(32), 0xb0);
	assert_eq!(read(33), 0xc1);
	assert_eq!(read(64), 0xe0);
	assert_eq!(read(96), 0xff);

	let span = PAGE_SIZE - 0x10..PAGE_SIZE * 2 + 0x10;
	let bytes = unit.read(&flash, span).unwrap().deny_warnings().unwrap();
	assert_eq!(bytes[..0x10], [0xa2; 0x10]);
	assert_eq!(bytes[0x10..0x1010], [0xa1; 0x1000]);
	assert_eq!(bytes[0x1010..], [0xff; 0x10]);
}

/// The search for the last written page probes the last page first, as the kernel does, so a
/// commit in the last page is found even when the pages before it are erased.
#[test]
fn commit_in_last_page_is_found_past_erased_pages() {
	let mut flash = Flash::default();
	flash.page(600, 0, Page::primary(0, 1, DATA), &[0], 0x10);
	let mut page_map = [0u8; 32];
	page_map[31] = 5;
	flash.page(600, 31, Page::primary(0, 1, COMMIT), &page_map, 0x15);

	let unit = flash.mount().deny_warnings().unwrap();
	let bytes = unit.read(&flash, 5 * PAGE_SIZE..5 * PAGE_SIZE + 2).unwrap().deny_warnings().unwrap();
	assert_eq!(bytes, [0x15; 2]);
}

/// Copies the kernel cannot order are reported. A secondary block whose primary block does not
/// hold its logical block is dropped, as the kernel mount drops it, and two copies with the same
/// sequence number resolve to the one scanned first.
#[test]
fn conflicting_blocks_warn() {
	let mut flash = Flash::default();
	flash.page(600, 0, Page::primary(4, 1, COMMIT), &[0], 0x40);
	flash.page(601, 0, Page::secondary(3, 2, 600, COMMIT), &[0], 0x30);
	flash.page(602, 0, Page::primary(6, 9, COMMIT), &[0], 0x60);
	flash.page(603, 0, Page::primary(6, 9, COMMIT), &[0], 0x61);

	let unit = flash.mount();
	assert_eq!(
		unit.warnings(),
		[
			RawWarning::MuDuplicateSequence {
				logical: MuBlockIndex::new(6),
				used: BlockIndex::new(602),
				ignored: BlockIndex::new(603),
			},
			RawWarning::MuOrphanedSecondary {
				logical: MuBlockIndex::new(3),
				secondary: BlockIndex::new(601),
				primary: BlockIndex::new(600),
			},
		]
	);
	let unit = unit.ignore_warnings();
	let block = |logical: u64| unit.read(&flash, logical * MU_BLOCK_SIZE..logical * MU_BLOCK_SIZE + 1).unwrap();
	assert_eq!(block(3).ignore_warnings(), [0xff]);
	assert_eq!(block(4).ignore_warnings(), [0x40]);
	assert_eq!(block(6).ignore_warnings(), [0x60]);
}

/// A data sector whose ECC does not match is returned as stored and reported by the read and by
/// both source adapters.
#[test]
fn data_sector_with_wrong_ecc_warns() {
	let mut flash = Flash::default();
	// The scan reads only the first sector of page 0 and all of the commit page.
	flash.page(600, 0, Page::primary(0, 1, DATA), &[0], 0x11);
	flash.page(600, 1, Page::primary(0, 1, COMMIT), &[0, 1], 0x12);
	let sector = 600 * SECTORS_PER_BLOCK + 3;
	flash.sectors.get_mut(&sector).unwrap()[7] ^= 0x01;

	let unit = flash.mount().deny_warnings().unwrap();
	let bytes = unit.read(&flash, 0..PAGE_SIZE).unwrap();
	let [RawWarning::EccMismatch { sector: reported, .. }] = bytes.warnings() else {
		panic!("expected one ECC mismatch, got {:?}", bytes.warnings());
	};
	assert_eq!(*reported, SectorIndex::new(sector));
	assert_eq!(bytes.value()[3 * SECTOR_DATA_SIZE + 7], 0x10);

	let source = MemoryUnitSource::new(&unit, &flash);
	for _ in 0..2 {
		let bytes = ReadAt::read_at(&source, &(), 0..PAGE_SIZE).unwrap();
		assert_eq!(bytes.as_ref()[3 * SECTOR_DATA_SIZE + 7], 0x10);
	}
	assert_eq!(source.take_warnings(), bytes.warnings());
	assert_eq!(source.warnings(), []);

	let source = AsyncMemoryUnitSource::new(&unit, &flash);
	let asynchronous = futures::executor::block_on(sfcx::AsyncReadAt::read_at(&source, &(), 0..PAGE_SIZE)).unwrap();
	assert!(asynchronous.as_ref() == bytes.value().as_slice());
	assert_eq!(source.warnings(), bytes.warnings());
}

/// The newest intact root directory names the system flash blocks between the kernel reserved
/// blocks and the file system, which the memory unit also uses.
#[test]
fn root_directory_adds_system_flash_blocks() {
	let mut flash = Flash::default();
	flash.root_directory(100, 0, 1, None);
	flash.page(20, 0, Page::primary(5, 1, COMMIT), &[0], 0x20);

	let unit = flash.mount().deny_warnings().unwrap();
	// 82 blocks between block 10 and block 92 join the 105 usable blocks past the configured size.
	assert_eq!(unit.size(), (82 + 105 - 5) * MU_BLOCK_SIZE);
	let bytes = unit.read(&flash, 5 * MU_BLOCK_SIZE..5 * MU_BLOCK_SIZE + 4).unwrap().deny_warnings().unwrap();
	assert_eq!(bytes, [0x20; 4]);

	let unit = futures::executor::block_on(async_mount(&flash)).deny_warnings().unwrap();
	assert_eq!(unit.size(), (82 + 105 - 5) * MU_BLOCK_SIZE);
}

/// A root directory slot whose sectors disagree on the sequence number is not used.
#[test]
fn torn_root_directory_is_ignored() {
	let mut flash = Flash::default();
	flash.root_directory(100, 2, 1, Some(7));
	flash.page(20, 0, Page::primary(5, 1, COMMIT), &[0], 0x20);

	let unit = flash.mount().deny_warnings().unwrap();
	assert_eq!(unit.size(), 100 * MU_BLOCK_SIZE);
	let bytes = unit.read(&flash, 5 * MU_BLOCK_SIZE..5 * MU_BLOCK_SIZE + 4).unwrap().deny_warnings().unwrap();
	assert_eq!(bytes, [0xff; 4]);
}

/// Runs against a private raw NAND dump named by `SFCX_RAW_IMAGE`. The memory unit mounts and
/// both of its partitions hold a FATX volume.
#[test]
#[ignore = "requires SFCX_RAW_IMAGE"]
fn raw_dump_memory_unit_partitions_hold_fatx() {
	let path = std::env::var_os("SFCX_RAW_IMAGE").expect("SFCX_RAW_IMAGE is set");
	let raw = std::fs::read(path).unwrap();
	let source = SliceSource(raw.as_slice());

	let image = FlashImage::open(&source, raw.len() as u64, FormatSelection::Auto).unwrap().deny_warnings().unwrap();
	let unit = image.memory_unit(&source).unwrap().deny_warnings().unwrap();
	let partitions = unit.partitions();
	for range in [partitions.data, partitions.system] {
		let header = unit.read(&source, range.start..range.start + 4).unwrap().deny_warnings().unwrap();
		assert_eq!(header, *b"XTAF", "partition at {:#x}", range.start);
	}
}

/// A sparse raw image: sectors that were never written read as erased.
#[derive(Default)]
struct Flash {
	sectors: BTreeMap<usize, [u8; RAW_SECTOR_SIZE]>,
}

#[derive(Clone, Copy)]
struct Page {
	logical_block: u16,
	source_block: u16,
	sequence: u64,
	page_type: u8,
}

impl Page {
	fn primary(logical_block: u16, sequence: u64, page_type: u8) -> Self {
		Self { logical_block, source_block: 0, sequence, page_type }
	}

	fn secondary(logical_block: u16, sequence: u64, primary: u16, page_type: u8) -> Self {
		Self { logical_block, source_block: primary, sequence, page_type }
	}
}

impl Flash {
	const RAW_SIZE: u64 = (PHYSICAL_BLOCKS * SECTORS_PER_BLOCK * RAW_SECTOR_SIZE) as u64;

	/// Writes a memory unit page filled with `fill`. `page_map` holds the logical page of every
	/// physical page up to this one.
	fn page(&mut self, block: usize, page: usize, header: Page, page_map: &[u8], fill: u8) {
		let mut packed = [0u8; 20];
		for (index, logical) in page_map.iter().enumerate() {
			let bit = index * 5;
			let value = u16::from(*logical) << (bit % 8);
			packed[bit / 8] |= value as u8;
			if bit % 8 > 3 {
				packed[bit / 8 + 1] |= (value >> 8) as u8;
			}
		}

		for index in 0..SECTORS_PER_PAGE {
			let mut spare = [0u8; SPARE_SIZE];
			if index == 0 {
				spare[0] = 0xff;
				spare[2] = ((header.logical_block >> 8) as u8) << 4;
				spare[3] = header.logical_block as u8;
				spare[4] = header.source_block as u8;
				spare[5] = (header.source_block >> 8) as u8 & 0x0f;
				spare[7..12].copy_from_slice(&header.sequence.to_le_bytes()[..5]);
			} else {
				let start = (index - 1) * 12;
				let end = (start + 12).min(packed.len());
				if start < end {
					spare[..end - start].copy_from_slice(&packed[start..end]);
				}
			}
			spare[12] = header.page_type;
			self.write(block * SECTORS_PER_BLOCK + page * SECTORS_PER_PAGE + index, [fill; SECTOR_DATA_SIZE], spare);
		}
	}

	/// Writes a root directory copy in slot `slot` of logical block `block`: 10 kernel reserved
	/// blocks, 384 file system blocks, and 4 trailing reserved blocks. `torn` gives one sector a
	/// different sequence number.
	fn root_directory(&mut self, block: usize, slot: usize, sequence: u32, torn: Option<usize>) {
		for index in 0..32 {
			let sequence = if torn == Some(index) { sequence + 1 } else { sequence }.to_le_bytes();
			let mut spare = [0u8; SPARE_SIZE];
			spare[0] = 0xff;
			spare[1..3].copy_from_slice(&(block as u16).to_le_bytes());
			spare[3] = sequence[1];
			spare[4] = sequence[2];
			spare[5] = sequence[0];
			spare[6] = sequence[3];
			spare[7] = 10;
			// 384 blocks, in units of four.
			spare[8] = 96;
			spare[9] = 4;
			spare[12] = 44;
			self.write(block * SECTORS_PER_BLOCK + slot * 32 + index, [0; SECTOR_DATA_SIZE], spare);
		}
		if slot != 0 {
			// The kernel inspects the slots of a block whose first sector is a data block.
			let mut spare = [0u8; SPARE_SIZE];
			spare[0] = 0xff;
			spare[7] = 10;
			spare[8] = 96;
			spare[9] = 4;
			spare[12] = 42;
			self.write(block * SECTORS_PER_BLOCK, [0; SECTOR_DATA_SIZE], spare);
		}
	}

	fn mark_bad(&mut self, block: usize) {
		self.sectors.get_mut(&(block * SECTORS_PER_BLOCK)).unwrap()[SECTOR_DATA_SIZE] = 0;
	}

	fn write(&mut self, sector: usize, data: [u8; SECTOR_DATA_SIZE], mut spare: [u8; SPARE_SIZE]) {
		let ecc = sector_ecc(&data, &spare);
		let word = u32::from_le_bytes(spare[12..].try_into().unwrap()) & 0x3f | ecc.get() << 6;
		spare[12..].copy_from_slice(&word.to_le_bytes());

		let mut record = [0u8; RAW_SECTOR_SIZE];
		record[..SECTOR_DATA_SIZE].copy_from_slice(&data);
		record[SECTOR_DATA_SIZE..].copy_from_slice(&spare);
		self.sectors.insert(sector, record);
	}

	fn layout(&self) -> RawLayout {
		let geometry = Geometry::new(RawKind::BigBlock, Self::RAW_SIZE).unwrap();
		let metadata: Vec<[u8; RAW_SECTOR_SIZE]> = (0..CONFIGURED_BLOCKS)
			.map(|block| {
				let range = geometry.metadata_range(BlockIndex::new(block));
				let bytes = ReadAt::read_at(self, &(), range).unwrap();
				bytes.as_ref().try_into().unwrap()
			})
			.collect();
		RawLayout::new(geometry, &metadata)
	}

	fn mount(&self) -> sfcx::Checked<MemoryUnit> {
		let layout = self.layout();
		drive(OpenMemoryUnit::new(&layout).unwrap(), self).unwrap()
	}
}

impl ReadAt for Flash {
	fn read_at(&self, _context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		assert!(range.end <= Self::RAW_SIZE, "read {range:#x?} past the image");
		assert_eq!(range.start % RAW_SECTOR_SIZE as u64, 0, "reads cover whole records");

		let first = (range.start / RAW_SECTOR_SIZE as u64) as usize;
		let count = (range.end - range.start) as usize / RAW_SECTOR_SIZE;
		let mut bytes = vec![0xff; count * RAW_SECTOR_SIZE];
		for (sector, record) in self.sectors.range(first..first + count) {
			let start = (sector - first) * RAW_SECTOR_SIZE;
			bytes[start..start + RAW_SECTOR_SIZE].copy_from_slice(record);
		}
		Ok(bytes)
	}
}

async fn async_mount(flash: &Flash) -> sfcx::Checked<MemoryUnit> {
	let layout = flash.layout();
	sfcx::io::drive_async(OpenMemoryUnit::new(&layout).unwrap(), flash).await.unwrap()
}
