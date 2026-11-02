mod common;

use sfcx::ChainKind;
use sfcx::ClusterId;
use sfcx::FlashImage;
use sfcx::FormatSelection;
use sfcx::Layout;
use sfcx::LocateRoot;
use sfcx::ParseError;
use sfcx::SequenceNumber;
use sfcx::SfcxError;
use sfcx::io::SliceSource;
use sfcx::io::drive;
use sfcx::raw::BlockIndex;
use sfcx::raw::RawKind;

use common::RawImage;
use common::header;

const DATA_BLOCK_SIZE: usize = 0x4000;
const BOOT_REGION_SIZE: u32 = 0x8040;
const SMALL_PHYSICAL_BLOCKS: usize = 1024;
const SMALL_LOGICAL_BLOCKS: usize = 992;
const BIG_BLOCK_SIZE: usize = 0x2_0000;
const BIG_PHYSICAL_BLOCKS: usize = 512;
const BIG_LOGICAL_BLOCKS: usize = 480;
/// 480 logical blocks less 4 trailing reserved blocks and 384 file system blocks.
const BIG_FILE_SYSTEM_FIRST_BLOCK: usize = 92;
const ROOT_DIRECTORY: u8 = 0x30;
const BIG_BLOCK_DATA: u8 = 0x2a;
const BIG_BLOCK_ROOT_DIRECTORY: u8 = 0x2c;

/// The newest intact root directory copy of a small-block flash holds the file system, which
/// starts at logical block 0. Directory records end at the first zero byte, deleted records are
/// skipped, and files follow their cluster chains.
#[test]
fn small_block_root_directory_is_the_newest_intact_copy() {
	read_small_block_filesystem(RawKind::SmallBlock);
}

#[test]
fn big_on_small_block_root_directory_is_the_newest_intact_copy() {
	read_small_block_filesystem(RawKind::BigOnSmallBlock);
}

/// A big-block root directory copy may sit in any 16 KiB slot of a block. The metadata of the
/// block's first sector places the file system, so data blocks are numbered from the first file
/// system block. A copy in a block below the file system it names is not used.
#[test]
fn big_block_root_directory_names_the_file_system_blocks() {
	const ROOT_BLOCK: usize = 300;
	const ROOT_SLOT: usize = 3;
	const DATA_OFFSET: usize = BIG_FILE_SYSTEM_FIRST_BLOCK * BIG_BLOCK_SIZE;

	let mut logical = retail_logical_image(BIG_LOGICAL_BLOCKS * BIG_BLOCK_SIZE);
	let multi = pattern(2 * DATA_BLOCK_SIZE + 0x123, 3);
	let last = pattern(0x40, 4);
	let mut root = RootDirectory::new();
	root.file(&mut logical, DATA_OFFSET, 0, b"multi.bin", &[0x10, 0x11, 0x200], &multi);
	for index in 1..239 {
		root.deleted(index);
	}
	root.file(&mut logical, DATA_OFFSET, 239, b"last.bin", &[0x12], &last);
	// The bad block bitmap fills the last directory page. Read as a record, it would name a file.
	root.record_at(0x3e00, b"bitmap.bin", 0x13, 1);
	root.store(&mut logical, ROOT_BLOCK * BIG_BLOCK_SIZE + ROOT_SLOT * DATA_BLOCK_SIZE);
	RootDirectory::single(b"older.bin").store(&mut logical, 200 * BIG_BLOCK_SIZE);
	RootDirectory::single(b"below.bin").store(&mut logical, 50 * BIG_BLOCK_SIZE);

	let mut image = RawImage::new(RawKind::BigBlock, BIG_PHYSICAL_BLOCKS, &logical);
	file_system_block(&mut image, ROOT_BLOCK, BIG_BLOCK_DATA, 1);
	let first_sector = ROOT_SLOT * 32;
	image.tag(ROOT_BLOCK, first_sector..first_sector + 32, BIG_BLOCK_ROOT_DIRECTORY, 5);
	file_system_block(&mut image, 200, BIG_BLOCK_ROOT_DIRECTORY, 4);
	image.tag(200, 0..32, BIG_BLOCK_ROOT_DIRECTORY, 4);
	file_system_block(&mut image, 50, BIG_BLOCK_ROOT_DIRECTORY, 9);
	image.tag(50, 0..32, BIG_BLOCK_ROOT_DIRECTORY, 9);
	let raw = image.finish();
	let source = SliceSource(raw.as_slice());
	let image =
		FlashImage::open(&source, raw.len() as u64, FormatSelection::RawBigBlock).unwrap().deny_warnings().unwrap();

	let Layout::Raw(layout) = image.layout() else {
		panic!("a raw image has a raw layout");
	};
	let scan = drive(LocateRoot::new(layout), &source).unwrap().deny_warnings().unwrap();
	let regions = scan.regions();
	assert_eq!(regions.kernel_reserved_block_count(), 10);
	assert_eq!(regions.file_system_first_block(), BlockIndex::new(BIG_FILE_SYSTEM_FIRST_BLOCK));
	assert_eq!(regions.file_system_block_count(), 384);
	assert_eq!(regions.data_block_count(), 384 * 8);
	let located = scan.root().expect("a root directory copy");
	assert_eq!(located.block(), BlockIndex::new(ROOT_BLOCK));
	assert_eq!(located.sequence(), SequenceNumber::new(5));

	let filesystem = image.filesystem(&source).unwrap().deny_warnings().unwrap();
	let root_offset = (ROOT_BLOCK * BIG_BLOCK_SIZE + ROOT_SLOT * DATA_BLOCK_SIZE) as u64;
	assert_eq!(filesystem.root_range(), root_offset..root_offset + DATA_BLOCK_SIZE as u64);
	assert_eq!(filesystem.layout().data_offset(), DATA_OFFSET as u64);
	assert_eq!(filesystem.layout().file_clusters(), ClusterId::new(0)..ClusterId::new(384 * 8));
	assert_eq!(names(&filesystem), ["multi.bin", "last.bin"]);

	let entry = filesystem.find("multi.bin").unwrap();
	let cluster = |index: usize| (DATA_OFFSET + index * DATA_BLOCK_SIZE) as u64;
	assert_eq!(
		filesystem.file_ranges(entry).unwrap(),
		[cluster(0x10)..cluster(0x12), cluster(0x200)..cluster(0x200) + 0x123]
	);
	assert_eq!(image.read_file(&source, &filesystem, entry).unwrap().deny_warnings().unwrap(), multi);
	let entry = filesystem.find("LAST.BIN").unwrap();
	assert_eq!(image.read_file(&source, &filesystem, entry).unwrap().deny_warnings().unwrap(), last);

	let asynchronous = futures::executor::block_on(image.filesystem_async(&source)).unwrap();
	assert_eq!(asynchronous.deny_warnings().unwrap(), filesystem);
}

#[test]
fn flash_without_root_directory_is_an_error() {
	let logical = retail_logical_image(SMALL_LOGICAL_BLOCKS * DATA_BLOCK_SIZE);
	let raw = RawImage::new(RawKind::SmallBlock, SMALL_PHYSICAL_BLOCKS, &logical).finish();
	let source = SliceSource(raw.as_slice());
	let image =
		FlashImage::open(&source, raw.len() as u64, FormatSelection::RawSmallBlock).unwrap().deny_warnings().unwrap();

	let error = image.filesystem(&source).unwrap_err();
	assert!(
		matches!(error, SfcxError::Parse(ParseError::RootDirectoryNotFound { kind: RawKind::SmallBlock })),
		"{error:?}"
	);
}

/// Only block metadata locates the root directory of a retail flash, and a logical image has
/// none.
#[test]
fn logical_retail_image_needs_raw_image() {
	let logical = retail_logical_image(SMALL_LOGICAL_BLOCKS * DATA_BLOCK_SIZE);
	let source = SliceSource(logical.as_slice());
	let image =
		FlashImage::open(&source, logical.len() as u64, FormatSelection::Logical).unwrap().deny_warnings().unwrap();

	let error = image.filesystem(&source).unwrap_err();
	assert!(
		matches!(error, SfcxError::Parse(ParseError::RootDirectoryNeedsRawImage { kind: ChainKind::Retail })),
		"{error:?}"
	);
	assert_eq!(
		error.to_string(),
		"the root directory of a retail flash cannot be located without the block metadata in the spare areas; \
		 read a raw NAND image instead of a logical one"
	);
}

/// Runs against a private raw NAND dump named by `SFCX_RAW_IMAGE`. The root directory that the
/// block metadata locates is the one the observed devkit profile presumes for the logical image.
#[test]
#[ignore = "requires SFCX_RAW_IMAGE"]
fn raw_dump_root_directory_matches_devkit_profile() {
	const DEVKIT_DATA_OFFSET: u64 = 0xb8_0000;
	const DEVKIT_ROOT_CLUSTER: u64 = 0x635;

	let path = std::env::var_os("SFCX_RAW_IMAGE").expect("SFCX_RAW_IMAGE is set");
	let raw = std::fs::read(path).unwrap();
	let source = SliceSource(raw.as_slice());
	let image = FlashImage::open(&source, raw.len() as u64, FormatSelection::Auto).unwrap().deny_warnings().unwrap();

	let Layout::Raw(layout) = image.layout() else {
		panic!("SFCX_RAW_IMAGE names a raw image");
	};
	let scan = drive(LocateRoot::new(layout), &source).unwrap().deny_warnings().unwrap();
	println!("{scan:#x?}");
	assert_eq!(scan.regions().file_system_first_block(), BlockIndex::new(0x5c));

	let filesystem = image.filesystem(&source).unwrap().deny_warnings().unwrap();
	assert_eq!(filesystem.layout().data_offset(), DEVKIT_DATA_OFFSET);
	assert_eq!(filesystem.root_range().start, DEVKIT_DATA_OFFSET + DEVKIT_ROOT_CLUSTER * DATA_BLOCK_SIZE as u64);
	let entry = filesystem.find("xbdm.xex").expect("xbdm.xex in the root directory");
	let xbdm = image.read_file(&source, &filesystem, entry).unwrap().deny_warnings().unwrap();
	assert_eq!(xbdm.len() as u64, entry.size());

	let logical = image.read(&source, 0..image.logical_size()).unwrap().deny_warnings().unwrap();
	let logical_source = SliceSource(logical.as_slice());
	let logical_image = FlashImage::open(&logical_source, logical.len() as u64, FormatSelection::Logical)
		.unwrap()
		.deny_warnings()
		.unwrap();
	let profile = logical_image.filesystem(&logical_source).unwrap().deny_warnings().unwrap();
	assert_eq!(profile.root_range(), filesystem.root_range());
	assert_eq!(profile.entries(), filesystem.entries());
	let entry = profile.find("xbdm.xex").unwrap();
	assert_eq!(logical_image.read_file(&logical_source, &profile, entry).unwrap().deny_warnings().unwrap(), xbdm);
}

/// A 16 KiB root directory block: allocation table pages and directory pages alternate.
struct RootDirectory(Vec<u8>);

impl RootDirectory {
	/// Every allocation entry free and every directory record zero.
	fn new() -> Self {
		let mut bytes = vec![0u8; DATA_BLOCK_SIZE];
		for page in (0..32).step_by(2) {
			for entry in bytes[page * 0x200..(page + 1) * 0x200].as_chunks_mut::<2>().0 {
				*entry = 0x1ffeu16.to_be_bytes();
			}
		}
		Self(bytes)
	}

	/// A directory with one one-byte file.
	fn single(name: &[u8]) -> Self {
		let mut root = Self::new();
		root.record(0, name, 0x20, 1);
		root.link(0x20, 0x1fff);
		root
	}

	fn link(&mut self, cluster: u16, next: u16) {
		let index = usize::from(cluster);
		let offset = index / 256 * 0x400 + index % 256 * 2;
		self.0[offset..offset + 2].copy_from_slice(&next.to_be_bytes());
	}

	fn record(&mut self, index: usize, name: &[u8], first_cluster: u16, size: u32) {
		self.record_at(index / 16 * 0x400 + 0x200 + index % 16 * 0x20, name, first_cluster, size);
	}

	fn record_at(&mut self, offset: usize, name: &[u8], first_cluster: u16, size: u32) {
		let record = &mut self.0[offset..offset + 0x20];
		record.fill(0);
		record[..name.len()].copy_from_slice(name);
		record[22..24].copy_from_slice(&first_cluster.to_be_bytes());
		record[24..28].copy_from_slice(&size.to_be_bytes());
		record[28..32].copy_from_slice(&(0x5000_0000 + offset as u32).to_be_bytes());
	}

	/// A deleted record. Its name starts with byte 5, which is not a valid name.
	fn deleted(&mut self, index: usize) {
		self.record(index, b"\x05eleted.bin", 0x7, 1);
	}

	/// Writes `contents` into the data blocks of `chain`, numbered from `data_offset`, and
	/// records the file.
	fn file(
		&mut self,
		logical: &mut [u8],
		data_offset: usize,
		index: usize,
		name: &[u8],
		chain: &[u16],
		contents: &[u8],
	) {
		for (position, chunk) in contents.chunks(DATA_BLOCK_SIZE).enumerate() {
			let start = data_offset + usize::from(chain[position]) * DATA_BLOCK_SIZE;
			logical[start..start + chunk.len()].copy_from_slice(chunk);
			self.link(chain[position], chain.get(position + 1).copied().unwrap_or(0x1fff));
		}
		self.record(index, name, chain[0], contents.len() as u32);
	}

	fn store(&self, logical: &mut [u8], offset: usize) {
		logical[offset..offset + DATA_BLOCK_SIZE].copy_from_slice(&self.0);
	}
}

fn read_small_block_filesystem(kind: RawKind) {
	const NEWEST_BLOCK: usize = 900;

	let mut logical = retail_logical_image(SMALL_LOGICAL_BLOCKS * DATA_BLOCK_SIZE);
	let multi = pattern(3 * DATA_BLOCK_SIZE + 100, 1);
	let small = pattern(10, 2);
	let mut root = RootDirectory::new();
	root.file(&mut logical, 0, 0, b"multi.bin", &[100, 101, 205, 50], &multi);
	root.deleted(1);
	root.file(&mut logical, 0, 2, b"small.bin", &[300], &small);
	// Record 3 stays zero and ends the directory.
	root.record(4, b"after.bin", 0x20, 1);
	root.store(&mut logical, NEWEST_BLOCK * DATA_BLOCK_SIZE);
	RootDirectory::single(b"older.bin").store(&mut logical, 950 * DATA_BLOCK_SIZE);
	RootDirectory::single(b"torn.bin").store(&mut logical, 800 * DATA_BLOCK_SIZE);

	let mut image = RawImage::new(kind, SMALL_PHYSICAL_BLOCKS, &logical);
	small_block_root(&mut image, NEWEST_BLOCK, 2);
	small_block_root(&mut image, 950, 1);
	small_block_root(&mut image, 800, 3);
	image.tag(800, 7..8, ROOT_DIRECTORY, 4);
	let raw = image.finish();
	let source = SliceSource(raw.as_slice());
	let selection = match kind {
		RawKind::SmallBlock => FormatSelection::RawSmallBlock,
		_ => FormatSelection::RawBigOnSmallBlock,
	};
	let image = FlashImage::open(&source, raw.len() as u64, selection).unwrap().deny_warnings().unwrap();

	let filesystem = image.filesystem(&source).unwrap().deny_warnings().unwrap();
	let root_offset = (NEWEST_BLOCK * DATA_BLOCK_SIZE) as u64;
	assert_eq!(filesystem.root_range(), root_offset..root_offset + DATA_BLOCK_SIZE as u64);
	assert_eq!(filesystem.layout().data_offset(), 0);
	assert_eq!(names(&filesystem), ["multi.bin", "small.bin"]);

	let entry = filesystem.find("multi.bin").unwrap();
	let cluster = |index: u64| index * DATA_BLOCK_SIZE as u64;
	assert_eq!(
		filesystem.file_ranges(entry).unwrap(),
		[cluster(100)..cluster(102), cluster(205)..cluster(206), cluster(50)..cluster(50) + 100]
	);
	assert_eq!(image.read_file(&source, &filesystem, entry).unwrap().deny_warnings().unwrap(), multi);
	let entry = filesystem.find("small.bin").unwrap();
	assert_eq!(image.read_file(&source, &filesystem, entry).unwrap().deny_warnings().unwrap(), small);
}

/// Tags every sector of `block` as a root directory copy that fills the block.
fn small_block_root(image: &mut RawImage, block: usize, sequence: u32) {
	const RESERVED_PAGES: usize = 9;

	image.tag(block, 0..32, ROOT_DIRECTORY, sequence);
	for sector in 0..32 {
		image.spare(block, sector)[RESERVED_PAGES] = 0;
	}
}

/// Tags the first sector of a big block with the file system regions: 10 kernel reserved
/// blocks, 384 file system blocks and 4 trailing reserved blocks.
fn file_system_block(image: &mut RawImage, block: usize, block_type: u8, sequence: u32) {
	image.tag(block, 0..1, block_type, sequence);
	let spare = image.spare(block, 0);
	spare[7] = 10;
	// In units of four blocks.
	spare[8] = 96;
	spare[9] = 4;
}

/// A logical device of `size` bytes that starts with a retail boot region: the flash header,
/// then CB, CD and CE stages.
fn retail_logical_image(size: usize) -> Vec<u8> {
	let mut logical = vec![0xffu8; size];
	logical[..16].fill(0);
	logical[..2].copy_from_slice(&[0xff, 0x4f]);
	logical[8..12].copy_from_slice(&0x8000u32.to_be_bytes());
	logical[12..16].copy_from_slice(&BOOT_REGION_SIZE.to_be_bytes());
	for (index, name) in [*b"CB", *b"CD", *b"CE"].into_iter().enumerate() {
		let offset = 0x8000 + index * 0x10;
		logical[offset..offset + 16].copy_from_slice(&header(name, 17559, 0, 0x10));
	}
	logical
}

fn pattern(len: usize, seed: usize) -> Vec<u8> {
	(0..len).map(|index| ((index * 7 + index / 251 + seed * 13) % 256) as u8).collect()
}

fn names(filesystem: &sfcx::Filesystem) -> Vec<&str> {
	filesystem.entries().iter().map(|entry| entry.name()).collect()
}
