use std::io;
use std::ops::Range;
use std::task::Poll;

use sfcx::AsyncReadAt;
use sfcx::FlashImage;
use sfcx::FormatSelection;
use sfcx::ImageFormat;
use sfcx::MemoryUnitError;
use sfcx::ParseError;
use sfcx::RawError;
use sfcx::RawWarning;
use sfcx::SfcxError;
use sfcx::io::SliceSource;
use sfcx::raw::BlockIndex;
use sfcx::raw::RAW_SECTOR_SIZE;
use sfcx::raw::RawKind;
use sfcx::raw::SECTOR_DATA_SIZE;
use sfcx::raw::SPARE_SIZE;
use sfcx::raw::SectorIndex;
use sfcx::raw::sector_ecc;

const BLOCK_SIZE: usize = 0x20000;
const PHYSICAL_BLOCKS: usize = 512;
const LOGICAL_BLOCKS: usize = 480;
const SECTORS_PER_BLOCK: usize = BLOCK_SIZE / SECTOR_DATA_SIZE;
const BAD_BLOCK: usize = 2;
const REPLACEMENT_BLOCK: usize = PHYSICAL_BLOCKS - 1;
const BOOT_REGION_SIZE: usize = 0x8040;

/// A raw 64 MiB big-block image of `logical`, with logical block 2 marked bad and held by the last
/// spare block, as the console's flash driver remaps it.
#[test]
fn raw_big_block_image_detects_and_remaps_bad_block() {
	let logical = logical_image();
	let raw = raw_big_block_image(&logical);
	let source = SliceSource(raw.as_slice());

	let image = FlashImage::open(&source, raw.len() as u64, FormatSelection::Auto).unwrap();
	assert_eq!(image.warnings(), []);
	let image = image.ignore_warnings();
	let ImageFormat::Raw(geometry) = image.format() else {
		panic!("raw image detected as {}", image.format());
	};
	assert_eq!(geometry.kind(), RawKind::BigBlock);
	assert_eq!(image.boot_region().bytes(), &logical[..BOOT_REGION_SIZE]);

	let device = image.read(&source, 0..image.logical_size()).unwrap();
	assert_eq!(device.warnings(), []);
	assert!(device.value().as_slice() == logical.as_slice());

	let image = FlashImage::open(&source, raw.len() as u64, FormatSelection::RawBigBlock).unwrap().ignore_warnings();
	assert_eq!(image.logical_size(), logical.len() as u64);
	assert!(matches!(FlashImage::open(&source, raw.len() as u64, FormatSelection::Logical), Err(SfcxError::Parse(_))));

	let logical_source = SliceSource(logical.as_slice());
	let image = FlashImage::open(&logical_source, logical.len() as u64, FormatSelection::Auto).unwrap();
	assert_eq!(image.value().format(), ImageFormat::Logical);
}

/// The asynchronous API reads the same bytes from a source that suspends on every read.
#[test]
fn async_reads_match_sync_reads() {
	let logical = logical_image();
	let raw = raw_big_block_image(&logical);
	let source = SuspendingSource(raw.clone());
	let size = raw.len() as u64;

	let image = futures::executor::block_on(FlashImage::open_async(&source, size, FormatSelection::Auto))
		.unwrap()
		.deny_warnings()
		.unwrap();
	let range = (BAD_BLOCK * BLOCK_SIZE - 0x100) as u64..(BAD_BLOCK * BLOCK_SIZE + 0x300) as u64;
	let bytes = futures::executor::block_on(image.read_async(&source, range.clone())).unwrap().deny_warnings().unwrap();
	assert_eq!(bytes, logical[range.start as usize..range.end as usize]);
}

/// A sector whose ECC does not match is read and reported, not rejected.
#[test]
fn raw_sector_with_wrong_ecc_warns() {
	let logical = logical_image();
	let mut raw = raw_big_block_image(&logical);
	raw[RAW_SECTOR_SIZE + 3] ^= 0x10;
	let source = SliceSource(raw.as_slice());

	let image = FlashImage::open(&source, raw.len() as u64, FormatSelection::Auto).unwrap();
	let [RawWarning::EccMismatch { sector, stored, computed }] = image.warnings() else {
		panic!("expected one ECC mismatch, got {:?}", image.warnings());
	};
	assert_eq!(*sector, SectorIndex::new(1));
	assert_eq!(stored.get(), 0x03ff_ffff);
	assert_ne!(computed, stored);
	assert_eq!(image.value().boot_region().bytes()[SECTOR_DATA_SIZE + 3], 0xff ^ 0x10);
	assert!(image.deny_warnings().is_err());
}

#[test]
fn bad_block_without_spare_is_an_error() {
	let logical = logical_image();
	let mut raw = raw_big_block_image(&logical);
	let replacement_spare = REPLACEMENT_BLOCK * SECTORS_PER_BLOCK * RAW_SECTOR_SIZE + SECTOR_DATA_SIZE;
	raw[replacement_spare] = 0;
	let source = SliceSource(raw.as_slice());

	let image = FlashImage::open(&source, raw.len() as u64, FormatSelection::RawBigBlock).unwrap().ignore_warnings();
	let range = (BAD_BLOCK * BLOCK_SIZE) as u64..(BAD_BLOCK * BLOCK_SIZE + 1) as u64;
	assert!(matches!(
		image.read(&source, range),
		Err(SfcxError::Raw(RawError::UntranslatedBlock { block })) if block == BlockIndex::new(BAD_BLOCK)
	));
}

#[test]
fn unrecognized_image_reports_both_formats() {
	let data = vec![0u8; 0x1000];
	let source = SliceSource(data.as_slice());

	assert!(matches!(
		FlashImage::open(&source, data.len() as u64, FormatSelection::Auto),
		Err(SfcxError::Unrecognized {
			logical: ParseError::UnsupportedMagic { .. },
			raw: RawError::PartialSector { size: 0x1000 }
		})
	));
}

/// A logical image has no spare areas, so its memory unit cannot be found.
#[test]
fn logical_image_has_no_memory_unit() {
	let logical = logical_image();
	let source = SliceSource(logical.as_slice());
	let image = FlashImage::open(&source, logical.len() as u64, FormatSelection::Logical).unwrap().ignore_warnings();

	assert!(matches!(image.memory_unit(&source), Err(SfcxError::MemoryUnit(MemoryUnitError::LogicalImage))));
}

/// The flash header locates the SMC firmware between itself and the end of the boot region; a
/// location outside that range is reported on request without failing the open.
#[test]
fn smc_location_is_bounded_by_the_boot_region() {
	let mut logical = logical_image();
	let mut set_smc = |size: u32, offset: u32| {
		logical[0x78..0x7c].copy_from_slice(&size.to_be_bytes());
		logical[0x7c..0x80].copy_from_slice(&offset.to_be_bytes());
		let source = SliceSource(logical.as_slice());
		let image =
			FlashImage::open(&source, logical.len() as u64, FormatSelection::Logical).unwrap().ignore_warnings();
		image.boot_region().smc().map(|smc| smc.map(|smc| (smc.offset().get(), smc.size().get())))
	};

	assert_eq!(set_smc(0x3000, 0x1000), Ok(Some((0x1000, 0x3000))));
	assert_eq!(set_smc(0, 0x1000), Ok(None));
	assert_eq!(set_smc(0x40, 0x40), Err(ParseError::SmcOutOfBounds { offset: 0x40, size: 0x40 }));
	let end = BOOT_REGION_SIZE as u32;
	assert_eq!(set_smc(0x41, end - 0x40), Err(ParseError::SmcOutOfBounds { offset: end as usize - 0x40, size: 0x41 }));
	assert_eq!(set_smc(0x40, end - 0x40), Ok(Some((end as usize - 0x40, 0x40))));
}

/// Runs against a private raw NAND dump named by `SFCX_RAW_IMAGE`. Every sector read passes the
/// ECC check, which the console computed, and the file system parses as the observed devkit
/// profile.
#[test]
#[ignore = "requires SFCX_RAW_IMAGE"]
fn raw_dump_opens_without_warnings() {
	let path = std::env::var_os("SFCX_RAW_IMAGE").expect("SFCX_RAW_IMAGE is set");
	let raw = std::fs::read(path).unwrap();
	let source = SliceSource(raw.as_slice());

	let image = FlashImage::open(&source, raw.len() as u64, FormatSelection::Auto).unwrap().deny_warnings().unwrap();
	assert!(matches!(image.format(), ImageFormat::Raw(geometry) if geometry.kind() == RawKind::BigBlock));
	let filesystem = image.filesystem(&source).unwrap().deny_warnings().unwrap();
	let entry = filesystem.find("xbdm.xex").expect("xbdm.xex in the root directory");
	let file = image.read_file(&source, &filesystem, entry).unwrap().deny_warnings().unwrap();
	assert_eq!(file.len() as u64, entry.size());
}

/// Runs against a private raw NAND dump named by `SFCX_RAW_IMAGE`. The synchronous and
/// asynchronous VFS return the bytes that `read_file` returns.
#[cfg(feature = "async-vfs")]
#[test]
#[ignore = "requires SFCX_RAW_IMAGE"]
fn vfs_reads_match_read_file() {
	use futures::AsyncReadExt;
	use sfcx::IntegrityPolicy;
	use sfcx::sfcx_vfs::SfcxVfs;
	use vfs::FileSystem;
	use vfs::async_vfs::AsyncFileSystem;

	let path = std::env::var_os("SFCX_RAW_IMAGE").expect("SFCX_RAW_IMAGE is set");
	let raw = std::fs::read(path).unwrap();
	let size = raw.len() as u64;
	let source = SliceSource(raw);
	let image = FlashImage::open(&source, size, FormatSelection::Auto).unwrap().deny_warnings().unwrap();
	let filesystem = image.filesystem(&source).unwrap().deny_warnings().unwrap();
	let entry = filesystem.find("xbdm.xex").unwrap().clone();
	let expected = image.read_file(&source, &filesystem, &entry).unwrap().deny_warnings().unwrap();
	let vfs = SfcxVfs::new(source, image, filesystem, IntegrityPolicy::Deny);

	let mut sync = Vec::new();
	FileSystem::open_file(&vfs, "/XBDM.XEX").unwrap().read_to_end(&mut sync).unwrap();
	assert!(sync == expected);

	let asynchronous = futures::executor::block_on(async {
		let mut data = Vec::new();
		AsyncFileSystem::open_file(&vfs, "/xbdm.xex").await.unwrap().read_to_end(&mut data).await.unwrap();
		data
	});
	assert!(asynchronous == expected);
}

/// Suspends once before every read, so the asynchronous driver must poll again.
struct SuspendingSource(Vec<u8>);

impl AsyncReadAt for SuspendingSource {
	async fn read_at(&self, _context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		let mut suspended = false;
		futures::future::poll_fn(|context| {
			if suspended {
				return Poll::Ready(());
			}
			suspended = true;
			context.waker().wake_by_ref();
			Poll::Pending
		})
		.await;

		Ok(self.0[range.start as usize..range.end as usize].to_vec())
	}
}

/// A logical 60 MiB image: a boot region of four header-only stages and a marker in every block.
fn logical_image() -> Vec<u8> {
	let mut logical = vec![0xffu8; LOGICAL_BLOCKS * BLOCK_SIZE];
	logical[..16].copy_from_slice(&header(*b"\xff\x4f", 0x8000, BOOT_REGION_SIZE as u32));
	for (index, name) in [*b"SB", *b"SC", *b"SD", *b"SE"].into_iter().enumerate() {
		let offset = 0x8000 + index * 0x10;
		logical[offset..offset + 16].copy_from_slice(&header(name, 0, 0x10));
	}
	for block in 1..LOGICAL_BLOCKS {
		logical[block * BLOCK_SIZE..block * BLOCK_SIZE + 4].copy_from_slice(&(block as u32).to_be_bytes());
	}
	logical
}

fn header(magic: [u8; 2], entry: u32, size: u32) -> [u8; 16] {
	let mut header = [0u8; 16];
	header[..2].copy_from_slice(&magic);
	header[8..12].copy_from_slice(&entry.to_be_bytes());
	header[12..].copy_from_slice(&size.to_be_bytes());
	header
}

fn raw_big_block_image(logical: &[u8]) -> Vec<u8> {
	let mut raw = vec![0xffu8; PHYSICAL_BLOCKS * SECTORS_PER_BLOCK * RAW_SECTOR_SIZE];
	for physical in 0..PHYSICAL_BLOCKS {
		let source = match physical {
			BAD_BLOCK => None,
			REPLACEMENT_BLOCK => Some(BAD_BLOCK),
			block if block < LOGICAL_BLOCKS => Some(block),
			_ => None,
		};
		for index in 0..SECTORS_PER_BLOCK {
			let sector = physical * SECTORS_PER_BLOCK + index;
			let mut data = [0xffu8; SECTOR_DATA_SIZE];
			if let Some(block) = source {
				let offset = block * BLOCK_SIZE + index * SECTOR_DATA_SIZE;
				data.copy_from_slice(&logical[offset..offset + SECTOR_DATA_SIZE]);
			}
			let mut spare = [0xffu8; SPARE_SIZE];
			if index == 0 {
				if physical == BAD_BLOCK {
					spare[0] = 0;
				}
				let number = source.unwrap_or(physical) as u16;
				spare[1..3].copy_from_slice(&number.to_le_bytes());
			}
			if data.iter().chain(&spare).all(|byte| *byte == 0xff) {
				continue;
			}

			let ecc = sector_ecc(&data, &spare);
			let word = u32::from_le_bytes(spare[12..].try_into().unwrap()) & 0x3f | ecc.get() << 6;
			spare[12..].copy_from_slice(&word.to_le_bytes());
			let start = sector * RAW_SECTOR_SIZE;
			raw[start..start + SECTOR_DATA_SIZE].copy_from_slice(&data);
			raw[start + SECTOR_DATA_SIZE..start + RAW_SECTOR_SIZE].copy_from_slice(&spare);
		}
	}
	raw
}
