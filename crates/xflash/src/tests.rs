use sfcx::raw::RAW_SECTOR_SIZE;
use sfcx::raw::SECTOR_DATA_SIZE;
use sfcx::raw::SPARE_SIZE;
use sfcx::raw::sector_ecc;

use super::*;

const BLOCK_SIZE: usize = 0x4000;
const PHYSICAL_BLOCKS: usize = 1024;
const LOGICAL_BLOCKS: usize = 992;
const SECTORS_PER_BLOCK: usize = BLOCK_SIZE / SECTOR_DATA_SIZE;
const ROOT_BLOCK: usize = 900;
const ROOT_SEQUENCE: u32 = 5;
const BLOCK_TYPE_ROOT_DIRECTORY: u8 = 0x30;
const CB_A_OFFSET: usize = 0x8000;
const CB_B_OFFSET: usize = 0x8400;
const CB_B_SIZE: usize = 0x2000;

/// A raw 16 MiB small-block retail flash: a split CB, patch slots, and a root directory.
#[test]
fn raw_small_block_retail_info_and_listing() {
	let raw = raw_small_block(&retail_logical(true));
	let source = SliceSource(raw.as_slice());
	let image = FlashImage::open(&source, raw.len() as u64, FormatSelection::Auto).unwrap().deny_warnings().unwrap();

	insta::assert_snapshot!(info(&image, &source));
	let filesystem = image.filesystem(&source).unwrap().deny_warnings().unwrap();
	let mut listing = Vec::new();
	write_listing(&mut listing, &filesystem).unwrap();
	insta::assert_snapshot!(String::from_utf8(listing).unwrap());
	let entry = filesystem.find("extended.bin").unwrap();
	let file = image.read_file(&source, &filesystem, entry).unwrap().deny_warnings().unwrap();
	assert!(file.iter().take(BLOCK_SIZE).all(|byte| *byte == 0x51));
	assert!(file.iter().skip(BLOCK_SIZE).all(|byte| *byte == 0x60));

	let boot_region = image.boot_region();
	let chain = boot_region.chain();
	let cb_b = find_stage(chain, StageRole::CbB).unwrap();
	assert_eq!((cb_b.offset().get(), cb_b.size().get()), (CB_B_OFFSET, CB_B_SIZE));
	let errors = [find_stage(chain, StageRole::Cb).unwrap_err(), find_stage(chain, StageRole::Sd).unwrap_err()];
	insta::assert_snapshot!(errors.map(|error| error.to_string()).join("\n"));
	assert_eq!(parse_stage_role("cb_a").unwrap(), StageRole::CbA);
	insta::assert_snapshot!(parse_stage_role("XX").unwrap_err());
}

/// A logical retail flash with a single CB: its boot chain reads, but its root directory cannot
/// be found without spare areas.
#[test]
fn logical_retail_info_and_listing() {
	let logical = retail_logical(false);
	let source = SliceSource(logical.as_slice());
	let image =
		FlashImage::open(&source, logical.len() as u64, FormatSelection::Auto).unwrap().deny_warnings().unwrap();

	insta::assert_snapshot!(info(&image, &source));
	let cb = find_stage(image.boot_region().chain(), StageRole::Cb).unwrap();
	assert_eq!(cb.offset().get(), CB_A_OFFSET);
	insta::assert_snapshot!(image.filesystem(&source).unwrap_err());
}

fn info(image: &FlashImage, source: &SliceSource<&[u8]>) -> String {
	let patch_slots = image.patch_slots(source).map(|slots| slots.deny_warnings().unwrap());
	let mut out = Vec::new();
	write_info(&mut out, image, &patch_slots).unwrap();
	String::from_utf8(out).unwrap()
}

/// The logical device of a small-block retail flash. A split CB has CB_A and CB_B; otherwise one
/// CB takes their place.
fn retail_logical(split: bool) -> Vec<u8> {
	let mut logical = vec![0xffu8; LOGICAL_BLOCKS * BLOCK_SIZE];
	logical[..0x70000].fill(0);
	logical[..16].copy_from_slice(&header(*b"\xff\x4f", 1888, 0, 0x70000));
	logical[8..12].copy_from_slice(&0x8000u32.to_be_bytes());
	let fields = [(0x64, 0x7_0000u32), (0x6c, 0x4000), (0x70, 0x1_0000), (0x78, 0x3000), (0x7c, 0x1000)];
	for (offset, value) in fields {
		logical[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
	}
	logical[0x1000..0x4000].fill(0x5c);

	let stages: &[([u8; 2], u16, u16, usize)] = if split {
		&[
			(*b"CB", 9188, 0x0800, 0x400),
			(*b"CB", 9188, 0, CB_B_SIZE),
			(*b"CD", 9188, 0, 0x1000),
			(*b"CE", 1888, 0, 0x5004),
		]
	} else {
		&[(*b"CB", 1940, 0, 0x2400), (*b"CD", 1940, 0, 0x1000), (*b"CE", 1888, 0, 0x5004)]
	};
	let mut offset = CB_A_OFFSET;
	for (name, build, flags, size) in stages.iter().copied() {
		logical[offset..offset + 16].copy_from_slice(&header(name, build, flags, size as u32));
		offset += size.next_multiple_of(16);
	}
	logical[0x70000..0x70010].copy_from_slice(&header(*b"CF", 17559, 0, 0x230));
	logical[0x70230..0x70240].copy_from_slice(&header(*b"CG", 17559, 0, 0x4000));

	let root = ROOT_BLOCK * BLOCK_SIZE;
	logical[root..root + BLOCK_SIZE].fill(0);
	let mut fat = [0x1ffeu16; 0x1000];
	fat[0x50] = 0x1fff;
	fat[0x51] = 0x60;
	fat[0x60] = 0x1fff;
	fat[0x70] = 0x1fff;
	for (index, link) in fat.iter().enumerate() {
		let at = root + index / 0x100 * 0x400 + index % 0x100 * 2;
		logical[at..at + 2].copy_from_slice(&link.to_be_bytes());
	}
	let entries = [("crl.bin", 0x50u16, 0x123u32), ("\x05eleted.bin", 0x70, 0x10), ("extended.bin", 0x51, 0x5000)];
	for (slot, (name, cluster, size)) in entries.into_iter().enumerate() {
		let at = root + 0x200 + slot * 0x20;
		let record = &mut logical[at..at + 0x20];
		record[..name.len()].copy_from_slice(name.as_bytes());
		record[22..24].copy_from_slice(&cluster.to_be_bytes());
		record[24..28].copy_from_slice(&size.to_be_bytes());
		record[28..32].copy_from_slice(&0x3a21_6000u32.to_be_bytes());
	}
	for (cluster, fill) in [(0x50, 0x50), (0x51, 0x51), (0x60, 0x60)] {
		logical[cluster * BLOCK_SIZE..(cluster + 1) * BLOCK_SIZE].fill(fill);
	}
	logical
}

/// The raw image of `logical`: block numbers in the first sector of every logical block, and the
/// root directory block tagged in every sector.
fn raw_small_block(logical: &[u8]) -> Vec<u8> {
	let mut raw = vec![0xffu8; PHYSICAL_BLOCKS * SECTORS_PER_BLOCK * RAW_SECTOR_SIZE];
	for block in 0..LOGICAL_BLOCKS {
		for index in 0..SECTORS_PER_BLOCK {
			let sector = block * SECTORS_PER_BLOCK + index;
			let data: &[u8; SECTOR_DATA_SIZE] =
				logical[sector * SECTOR_DATA_SIZE..(sector + 1) * SECTOR_DATA_SIZE].try_into().unwrap();
			let mut spare = [0xffu8; SPARE_SIZE];
			if index == 0 {
				spare[..2].copy_from_slice(&(block as u16).to_le_bytes());
			}
			if block == ROOT_BLOCK {
				let sequence = ROOT_SEQUENCE.to_le_bytes();
				(spare[2], spare[3], spare[4], spare[6]) = (sequence[0], sequence[1], sequence[2], sequence[3]);
				spare[9] = 0;
				spare[12] = BLOCK_TYPE_ROOT_DIRECTORY;
			}
			if data.iter().chain(&spare).all(|byte| *byte == 0xff) {
				continue;
			}

			let ecc = sector_ecc(data, &spare);
			let word = u32::from_le_bytes(spare[12..].try_into().unwrap()) & 0x3f | ecc.get() << 6;
			spare[12..].copy_from_slice(&word.to_le_bytes());
			let start = sector * RAW_SECTOR_SIZE;
			raw[start..start + SECTOR_DATA_SIZE].copy_from_slice(data);
			raw[start + SECTOR_DATA_SIZE..start + RAW_SECTOR_SIZE].copy_from_slice(&spare);
		}
	}
	raw
}

fn header(magic: [u8; 2], build: u16, flags: u16, size: u32) -> [u8; 16] {
	let mut header = [0u8; 16];
	header[..2].copy_from_slice(&magic);
	header[2..4].copy_from_slice(&build.to_be_bytes());
	header[6..8].copy_from_slice(&flags.to_be_bytes());
	header[12..].copy_from_slice(&size.to_be_bytes());
	header
}
