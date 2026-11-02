//! Synthetic flash images: a logical device and the raw NAND image that holds it.

#![allow(dead_code)]

use sfcx::raw::Geometry;
use sfcx::raw::RAW_SECTOR_SIZE;
use sfcx::raw::RawKind;
use sfcx::raw::SECTOR_DATA_SIZE;
use sfcx::raw::SPARE_SIZE;
use sfcx::raw::sector_ecc;

/// Offset in the spare area of the byte whose low six bits hold the block type.
pub const BLOCK_TYPE_OFFSET: usize = 12;

/// A raw image under construction: every logical block in its own physical block, the spare
/// blocks erased. Spare areas start erased; [`RawImage::finish`] writes block numbers into the
/// first sector of every logical block and the ECC of every sector that is not erased.
pub struct RawImage {
	geometry: Geometry,
	data: Vec<u8>,
	spares: Vec<[u8; SPARE_SIZE]>,
}

impl RawImage {
	/// A raw image of `physical_blocks` blocks of `kind` that holds `logical`, which must be the
	/// logical device size of that geometry.
	pub fn new(kind: RawKind, physical_blocks: usize, logical: &[u8]) -> Self {
		let block_size = 1usize << block_shift(kind);
		let sectors = physical_blocks * block_size / SECTOR_DATA_SIZE;
		let geometry = Geometry::new(kind, (sectors * RAW_SECTOR_SIZE) as u64).unwrap();
		assert_eq!(logical.len() as u64, geometry.logical_size(), "logical device size");

		let mut data = vec![0xffu8; sectors * SECTOR_DATA_SIZE];
		data[..logical.len()].copy_from_slice(logical);
		Self { geometry, data, spares: vec![[0xff; SPARE_SIZE]; sectors] }
	}

	pub const fn geometry(&self) -> Geometry {
		self.geometry
	}

	/// The spare area of sector `sector` of physical block `block`.
	pub fn spare(&mut self, block: usize, sector: usize) -> &mut [u8; SPARE_SIZE] {
		&mut self.spares[block * self.geometry.sectors_per_block() + sector]
	}

	/// Sets the block type and sequence number of every sector in `sectors` of `block`, in the
	/// spare layout of the image's kind.
	pub fn tag(&mut self, block: usize, sectors: std::ops::Range<usize>, block_type: u8, sequence: u32) {
		let kind = self.geometry.kind();
		for sector in sectors {
			let spare = self.spare(block, sector);
			spare[BLOCK_TYPE_OFFSET] = spare[BLOCK_TYPE_OFFSET] & 0xc0 | block_type & 0x3f;
			let bytes = sequence.to_le_bytes();
			let low = match kind {
				RawKind::SmallBlock => 2,
				RawKind::BigOnSmallBlock => 0,
				RawKind::BigBlock | RawKind::BigBlock256K => 5,
			};
			spare[low] = bytes[0];
			spare[3] = bytes[1];
			spare[4] = bytes[2];
			spare[6] = bytes[3];
		}
	}

	/// The raw image bytes.
	pub fn finish(mut self) -> Vec<u8> {
		let kind = self.geometry.kind();
		let sectors_per_block = self.geometry.sectors_per_block();
		for block in 0..self.geometry.logical_block_count() {
			let spare = &mut self.spares[block * sectors_per_block];
			let number = (block as u16).to_le_bytes();
			let low = match kind {
				RawKind::SmallBlock => 0,
				RawKind::BigOnSmallBlock | RawKind::BigBlock | RawKind::BigBlock256K => 1,
			};
			spare[low..low + 2].copy_from_slice(&number);
		}

		let mut raw = vec![0xffu8; self.spares.len() * RAW_SECTOR_SIZE];
		for (sector, spare) in self.spares.iter_mut().enumerate() {
			let data: &[u8; SECTOR_DATA_SIZE] =
				self.data[sector * SECTOR_DATA_SIZE..(sector + 1) * SECTOR_DATA_SIZE].try_into().unwrap();
			if data.iter().chain(spare.iter()).all(|byte| *byte == 0xff) {
				continue;
			}

			let ecc = sector_ecc(data, spare);
			let word = u32::from_le_bytes(spare[12..].try_into().unwrap()) & 0x3f | ecc.get() << 6;
			spare[12..].copy_from_slice(&word.to_le_bytes());
			let start = sector * RAW_SECTOR_SIZE;
			raw[start..start + SECTOR_DATA_SIZE].copy_from_slice(data);
			raw[start + SECTOR_DATA_SIZE..start + RAW_SECTOR_SIZE].copy_from_slice(spare);
		}

		raw
	}
}

const fn block_shift(kind: RawKind) -> u32 {
	match kind {
		RawKind::SmallBlock | RawKind::BigOnSmallBlock => 14,
		RawKind::BigBlock => 17,
		RawKind::BigBlock256K => 18,
	}
}

/// A boot loader header.
pub fn header(magic: [u8; 2], build: u16, flags: u16, size: u32) -> [u8; 16] {
	let mut header = [0u8; 16];
	header[..2].copy_from_slice(&magic);
	header[2..4].copy_from_slice(&build.to_be_bytes());
	header[6..8].copy_from_slice(&flags.to_be_bytes());
	header[12..].copy_from_slice(&size.to_be_bytes());
	header
}
