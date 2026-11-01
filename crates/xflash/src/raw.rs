//! Raw NAND images: every 512-byte sector followed by its 16-byte spare area, in physical
//! order, as the flash controller stores them.

use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use thiserror::Error;

pub const SECTOR_DATA_SIZE: usize = 0x200;
pub const SPARE_SIZE: usize = 0x10;
pub const RAW_SECTOR_SIZE: usize = SECTOR_DATA_SIZE + SPARE_SIZE;

/// The ECC occupies the upper 26 bits of the little-endian word at this spare offset.
const SPARE_ECC_OFFSET: usize = 12;
const BLOCK_NUMBER_MASK: u16 = 0x0fff;
const GOOD_BLOCK_MARKER: u8 = 0xff;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SectorIndex(usize);

impl SectorIndex {
	pub const fn new(value: usize) -> Self {
		Self(value)
	}

	pub const fn get(self) -> usize {
		self.0
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BlockIndex(usize);

impl BlockIndex {
	pub const fn new(value: usize) -> Self {
		Self(value)
	}

	pub const fn get(self) -> usize {
		self.0
	}
}

/// Placement of the bad-block marker and block number in the spare area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpareLayout {
	SmallBlock,
	BigOnSmallBlock,
	BigBlock,
}

impl SpareLayout {
	pub fn is_bad_block(self, spare: &[u8; SPARE_SIZE]) -> bool {
		let offset = match self {
			Self::BigBlock => 0,
			Self::SmallBlock | Self::BigOnSmallBlock => 5,
		};
		spare[offset] != GOOD_BLOCK_MARKER
	}

	pub fn block_number(self, spare: &[u8; SPARE_SIZE]) -> BlockIndex {
		let low = match self {
			Self::SmallBlock => 0,
			Self::BigOnSmallBlock | Self::BigBlock => 1,
		};
		let number = u16::from_le_bytes([spare[low], spare[low + 1]]) & BLOCK_NUMBER_MASK;
		BlockIndex(number as usize)
	}
}

/// A flash controller configuration that the build 16547 kernel supports
/// (`SfcxInitializeHardware`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
	/// 16 KiB blocks, small-block spare layout.
	SmallBlock,
	/// 16 KiB blocks, big-on-small-block spare layout.
	BigOnSmallBlock,
	/// 128 KiB blocks, big-block spare layout.
	BigBlock,
	/// 256 KiB blocks, big-block spare layout.
	BigBlock256K,
}

impl RawKind {
	pub const ALL: [Self; 4] = [Self::SmallBlock, Self::BigOnSmallBlock, Self::BigBlock, Self::BigBlock256K];

	pub const fn layout(self) -> SpareLayout {
		match self {
			Self::SmallBlock => SpareLayout::SmallBlock,
			Self::BigOnSmallBlock => SpareLayout::BigOnSmallBlock,
			Self::BigBlock | Self::BigBlock256K => SpareLayout::BigBlock,
		}
	}

	const fn block_shift(self) -> u32 {
		match self {
			Self::SmallBlock | Self::BigOnSmallBlock => 14,
			Self::BigBlock => 17,
			Self::BigBlock256K => 18,
		}
	}

	/// The configured counts for a device of `physical_blocks` blocks. Big-block devices may
	/// have blocks past the configured count, which the kernel uses for the memory unit.
	const fn counts(self, physical_blocks: usize) -> Option<BlockCounts> {
		let (block_count, spare_block_count) = match (self, physical_blocks) {
			(Self::SmallBlock | Self::BigOnSmallBlock, 1024) => (1024, 32),
			(Self::SmallBlock, 2048) => (2048, 64),
			(Self::SmallBlock | Self::BigOnSmallBlock, 4096) => (4096, 128),
			(Self::BigBlock, 512..=4096) => (512, 32),
			(Self::BigBlock256K, 256..=4096) => (256, 16),
			_ => return None,
		};
		Some(BlockCounts { block_count, spare_block_count })
	}
}

impl fmt::Display for RawKind {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		let name = match self {
			Self::SmallBlock => "small block",
			Self::BigOnSmallBlock => "big-on-small block",
			Self::BigBlock => "big block",
			Self::BigBlock256K => "big block with 256 KiB blocks",
		};
		formatter.write_str(name)
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlockCounts {
	block_count: usize,
	spare_block_count: usize,
}

/// Block geometry of a raw image. Blocks `[block_count - spare_block_count, block_count)` are
/// spares that replace bad logical blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
	kind: RawKind,
	physical_block_count: usize,
	block_count: usize,
	spare_block_count: usize,
}

impl Geometry {
	/// The geometry of `kind` for a raw image of `raw_size` bytes.
	pub fn new(kind: RawKind, raw_size: usize) -> Result<Self, RawError> {
		if raw_size == 0 || !raw_size.is_multiple_of(RAW_SECTOR_SIZE) {
			return Err(RawError::PartialSector { size: raw_size });
		}

		let data_size = raw_size / RAW_SECTOR_SIZE * SECTOR_DATA_SIZE;
		let block_size = 1usize << kind.block_shift();
		if !data_size.is_multiple_of(block_size) {
			return Err(RawError::UnsupportedGeometry { kind, data_size });
		}

		let physical_block_count = data_size / block_size;
		let counts = kind.counts(physical_block_count).ok_or(RawError::UnsupportedGeometry { kind, data_size })?;

		Ok(Self {
			kind,
			physical_block_count,
			block_count: counts.block_count,
			spare_block_count: counts.spare_block_count,
		})
	}

	pub const fn kind(&self) -> RawKind {
		self.kind
	}

	pub const fn block_size(&self) -> usize {
		1 << self.kind.block_shift()
	}

	pub const fn physical_block_count(&self) -> usize {
		self.physical_block_count
	}

	pub const fn block_count(&self) -> usize {
		self.block_count
	}

	pub const fn spare_block_count(&self) -> usize {
		self.spare_block_count
	}

	pub const fn logical_block_count(&self) -> usize {
		self.block_count - self.spare_block_count
	}

	/// Size of the logical flash device: the logical blocks without their spare areas.
	pub const fn logical_size(&self) -> usize {
		self.logical_block_count() << self.kind.block_shift()
	}

	const fn sectors_per_block(&self) -> usize {
		self.block_size() / SECTOR_DATA_SIZE
	}
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum RawError {
	#[error("raw image size {size:#x} is not a whole number of 0x210-byte sectors")]
	PartialSector { size: usize },
	#[error("{data_size:#x} bytes of flash data do not form a supported {kind} device")]
	UnsupportedGeometry { kind: RawKind, data_size: usize },
	#[error("the raw image matches no supported flash geometry")]
	UnknownGeometry,
	#[error("the raw image matches the {first} and {second} geometries equally")]
	AmbiguousGeometry { first: RawKind, second: RawKind },
	#[error("logical range {offset:#x}+{len:#x} extends past the {size:#x}-byte logical device")]
	LogicalRangeOutOfBounds { offset: usize, len: usize, size: usize },
	#[error("logical block {block:?} is bad and no spare block replaces it")]
	UntranslatedBlock { block: BlockIndex },
}

/// A condition the raw image reader tolerates, as the console's flash driver does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawWarning {
	/// The stored ECC of a sector does not match its data and spare area.
	EccMismatch { sector: SectorIndex },
	/// A spare block fails its ECC check, so its replacement record is ignored.
	UnreadableSpareBlock { block: BlockIndex },
}

impl fmt::Display for RawWarning {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::EccMismatch { sector } => {
				write!(
					formatter,
					"sector {:#x} (raw offset {:#x}) fails its ECC check",
					sector.0,
					sector.0 * RAW_SECTOR_SIZE
				)
			}
			Self::UnreadableSpareBlock { block } => {
				write!(formatter, "spare block {:#x} fails its ECC check and is ignored", block.0)
			}
		}
	}
}

/// Logical bytes read from a raw image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalRead {
	pub bytes: Vec<u8>,
	pub warnings: Vec<RawWarning>,
}

/// A raw image with the bad-block state its spare areas record.
pub struct RawImage<'a> {
	bytes: &'a [u8],
	geometry: Geometry,
	bad_blocks: Vec<bool>,
	/// Entry `i` names the logical block that block `block_count - 1 - i` replaces.
	spare_map: Vec<Option<BlockIndex>>,
	warnings: Vec<RawWarning>,
}

impl<'a> RawImage<'a> {
	/// Reads the bad-block markers and the spare-block records the way the kernel mount scan
	/// does.
	pub fn parse(bytes: &'a [u8], geometry: Geometry) -> Result<Self, RawError> {
		let expected = Geometry::new(geometry.kind, bytes.len())?;
		if expected != geometry {
			return Err(RawError::UnsupportedGeometry {
				kind: geometry.kind,
				data_size: bytes.len() / RAW_SECTOR_SIZE * SECTOR_DATA_SIZE,
			});
		}

		let layout = geometry.kind.layout();
		let mut image = Self {
			bytes,
			geometry,
			bad_blocks: vec![false; geometry.block_count],
			spare_map: vec![None; geometry.spare_block_count],
			warnings: Vec::new(),
		};
		for block in (0..geometry.block_count).rev() {
			let sector = image.first_sector(BlockIndex(block));
			let spare = image.spare(sector);
			if layout.is_bad_block(spare) {
				image.bad_blocks[block] = true;
				continue;
			}
			if block < geometry.logical_block_count() {
				continue;
			}

			if !image.sector_ecc_matches(sector) {
				image.warnings.push(RawWarning::UnreadableSpareBlock { block: BlockIndex(block) });
				continue;
			}
			let replaced = layout.block_number(spare);
			if replaced.0 < geometry.logical_block_count() {
				image.spare_map[geometry.block_count - 1 - block] = Some(replaced);
			}
		}

		Ok(image)
	}

	/// Detects the geometry whose block-number fields agree with the most block positions.
	pub fn detect(bytes: &'a [u8]) -> Result<Self, RawError> {
		if bytes.is_empty() || !bytes.len().is_multiple_of(RAW_SECTOR_SIZE) {
			return Err(RawError::PartialSector { size: bytes.len() });
		}

		let mut best: Option<(Geometry, usize)> = None;
		let mut tied: Option<RawKind> = None;
		for kind in RawKind::ALL {
			let Ok(geometry) = Geometry::new(kind, bytes.len()) else {
				continue;
			};
			let score = numbered_block_count(bytes, geometry);
			if score == 0 {
				continue;
			}

			match best {
				Some((_, best_score)) if score < best_score => {}
				Some((_, best_score)) if score == best_score => tied = Some(kind),
				_ => {
					best = Some((geometry, score));
					tied = None;
				}
			}
		}

		let Some((geometry, _)) = best else {
			return Err(RawError::UnknownGeometry);
		};
		if let Some(first) = tied {
			return Err(RawError::AmbiguousGeometry { first, second: geometry.kind });
		}

		Self::parse(bytes, geometry)
	}

	pub const fn geometry(&self) -> Geometry {
		self.geometry
	}

	/// Spare blocks whose records are ignored.
	pub fn warnings(&self) -> &[RawWarning] {
		&self.warnings
	}

	/// The physical block that holds logical block `block`, or `None` for a block outside the
	/// logical device or a bad block that no spare replaces.
	pub fn translate(&self, block: BlockIndex) -> Option<BlockIndex> {
		if block.0 >= self.geometry.logical_block_count() {
			return None;
		}
		if !self.bad_blocks[block.0] {
			return Some(block);
		}

		self.spare_map.iter().enumerate().find_map(|(index, replaced)| {
			let physical = self.geometry.block_count - 1 - index;
			(*replaced == Some(block) && !self.bad_blocks[physical]).then_some(BlockIndex(physical))
		})
	}

	/// Reads `len` logical bytes at `offset`, stripping spare areas, replacing bad blocks, and
	/// checking the ECC of every sector read.
	pub fn read_logical(&self, offset: usize, len: usize) -> Result<LogicalRead, RawError> {
		let size = self.geometry.logical_size();
		if offset.checked_add(len).is_none_or(|end| end > size) {
			return Err(RawError::LogicalRangeOutOfBounds { offset, len, size });
		}

		let mut bytes = Vec::with_capacity(len);
		let mut warnings = Vec::new();
		let mut position = offset;
		while position < offset + len {
			let logical_block = BlockIndex(position / self.geometry.block_size());
			let physical_block =
				self.translate(logical_block).ok_or(RawError::UntranslatedBlock { block: logical_block })?;
			let within_block = position % self.geometry.block_size();
			let sector = SectorIndex(self.first_sector(physical_block).0 + within_block / SECTOR_DATA_SIZE);
			let within_sector = position % SECTOR_DATA_SIZE;
			let count = (SECTOR_DATA_SIZE - within_sector).min(offset + len - position);

			if !self.sector_ecc_matches(sector) {
				warnings.push(RawWarning::EccMismatch { sector });
			}
			bytes.extend_from_slice(&self.data(sector)[within_sector..within_sector + count]);
			position += count;
		}

		Ok(LogicalRead { bytes, warnings })
	}

	fn first_sector(&self, block: BlockIndex) -> SectorIndex {
		SectorIndex(block.0 * self.geometry.sectors_per_block())
	}

	fn data(&self, sector: SectorIndex) -> &'a [u8; SECTOR_DATA_SIZE] {
		data(self.bytes, sector)
	}

	fn spare(&self, sector: SectorIndex) -> &'a [u8; SPARE_SIZE] {
		spare(self.bytes, sector)
	}

	fn sector_ecc_matches(&self, sector: SectorIndex) -> bool {
		sector_ecc_matches(self.data(sector), self.spare(sector))
	}
}

/// The 26-bit ECC of a sector over its 512 data bytes and the first 102 bits of its spare area,
/// as `SfcxEncodeECC` computes it: bits enter least significant first, polynomial 0x06954559.
pub fn sector_ecc(data: &[u8; SECTOR_DATA_SIZE], spare: &[u8; SPARE_SIZE]) -> u32 {
	const POLYNOMIAL: u32 = 0x0695_4559;
	const SPARE_COVERED_BITS: usize = SPARE_ECC_OFFSET * 8 + 6;

	let bits = data
		.iter()
		.flat_map(|byte| (0..8).map(move |bit| byte >> bit & 1))
		.chain(spare.iter().flat_map(|byte| (0..8).map(move |bit| byte >> bit & 1)).take(SPARE_COVERED_BITS));
	let mut value = 0u32;
	for bit in bits {
		if (u32::from(bit) ^ value) & 1 == 0 {
			value ^= POLYNOMIAL;
		}
		value >>= 1;
	}

	!value & 0x03ff_ffff
}

/// The ECC a spare area stores.
pub fn stored_ecc(spare: &[u8; SPARE_SIZE]) -> u32 {
	let word = u32::from_le_bytes(spare[SPARE_ECC_OFFSET..].try_into().expect("four ECC bytes"));
	word >> 6
}

/// An erased sector holds all ones and no ECC.
fn sector_ecc_matches(data: &[u8; SECTOR_DATA_SIZE], spare: &[u8; SPARE_SIZE]) -> bool {
	let erased = data.iter().chain(spare).all(|byte| *byte == 0xff);
	erased || sector_ecc(data, spare) == stored_ecc(spare)
}

fn data(bytes: &[u8], sector: SectorIndex) -> &[u8; SECTOR_DATA_SIZE] {
	let start = sector.0 * RAW_SECTOR_SIZE;
	bytes[start..start + SECTOR_DATA_SIZE].try_into().expect("whole sector")
}

fn spare(bytes: &[u8], sector: SectorIndex) -> &[u8; SPARE_SIZE] {
	let start = sector.0 * RAW_SECTOR_SIZE + SECTOR_DATA_SIZE;
	bytes[start..start + SPARE_SIZE].try_into().expect("whole spare area")
}

/// Good, ECC-valid logical blocks whose block-number field equals their position under
/// `geometry`.
fn numbered_block_count(bytes: &[u8], geometry: Geometry) -> usize {
	let layout = geometry.kind.layout();
	(0..geometry.logical_block_count())
		.filter(|block| {
			let sector = SectorIndex(block * geometry.sectors_per_block());
			let (data, spare) = (data(bytes, sector), spare(bytes, sector));
			let erased = data.iter().chain(spare).all(|byte| *byte == 0xff);
			!erased
				&& !layout.is_bad_block(spare)
				&& sector_ecc(data, spare) == stored_ecc(spare)
				&& layout.block_number(spare).0 == *block
		})
		.count()
}
