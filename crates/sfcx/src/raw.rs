//! Raw NAND images: every 512-byte sector followed by its 16-byte spare area, in physical
//! order, as the flash controller stores them.

use std::fmt;
use std::ops::Range;

use crate::error::RawError;
use crate::error::SfcxError;
use crate::mu::MuBlockIndex;

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

	/// Offset of the sector's record in the raw image.
	pub const fn raw_offset(self) -> u64 {
		(self.0 * RAW_SECTOR_SIZE) as u64
	}

	/// The raw records of `count` sectors starting at this one.
	pub const fn records(self, count: usize) -> Range<u64> {
		let start = self.raw_offset();
		start..start + (count * RAW_SECTOR_SIZE) as u64
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
	const fn counts(self, physical_blocks: u64) -> Option<BlockCounts> {
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
	pub fn new(kind: RawKind, raw_size: u64) -> Result<Self, RawError> {
		let record = RAW_SECTOR_SIZE as u64;
		if raw_size == 0 || !raw_size.is_multiple_of(record) {
			return Err(RawError::PartialSector { size: raw_size });
		}

		let data_size = raw_size / record * SECTOR_DATA_SIZE as u64;
		let block_size = 1u64 << kind.block_shift();
		if !data_size.is_multiple_of(block_size) {
			return Err(RawError::UnsupportedGeometry { kind, data_size });
		}

		let physical_block_count = data_size / block_size;
		let counts = kind.counts(physical_block_count).ok_or(RawError::UnsupportedGeometry { kind, data_size })?;

		Ok(Self {
			kind,
			physical_block_count: physical_block_count as usize,
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
	pub const fn logical_size(&self) -> u64 {
		(self.logical_block_count() as u64) << self.kind.block_shift()
	}

	pub const fn block_shift(&self) -> u32 {
		self.kind.block_shift()
	}

	pub const fn sectors_per_block(&self) -> usize {
		self.block_size() / SECTOR_DATA_SIZE
	}

	pub const fn first_sector(&self, block: BlockIndex) -> SectorIndex {
		SectorIndex(block.0 * self.sectors_per_block())
	}

	/// The raw record of the first sector of `block`, which holds the block's metadata.
	pub const fn metadata_range(&self, block: BlockIndex) -> Range<u64> {
		let start = self.first_sector(block).raw_offset();
		start..start + RAW_SECTOR_SIZE as u64
	}
}

/// A condition the raw image reader tolerates, as the console's flash driver does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RawWarning {
	/// The stored ECC of a sector does not match its data and spare area. The sector is used as
	/// stored, including the block metadata in its spare area.
	EccMismatch { sector: SectorIndex, stored: Ecc, computed: Ecc },
	/// Two memory unit blocks hold the same logical block with the same sequence number. The
	/// block scanned first is used, as the kernel does.
	MuDuplicateSequence { logical: MuBlockIndex, used: BlockIndex, ignored: BlockIndex },
	/// A memory unit secondary block is the newest copy of its logical block, but the primary
	/// block it names does not hold that logical block. The kernel discards the mapping, so the
	/// whole logical block reads as erased.
	MuOrphanedSecondary { logical: MuBlockIndex, secondary: BlockIndex, primary: BlockIndex },
}

impl fmt::Display for RawWarning {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::EccMismatch { sector, stored, computed } => write!(
				formatter,
				"sector {:#x} (raw offset {:#x}) fails its ECC check: stored {:#09x}, computed {:#09x}",
				sector.0,
				sector.raw_offset(),
				stored.0,
				computed.0
			),
			Self::MuDuplicateSequence { logical, used, ignored } => write!(
				formatter,
				"memory unit blocks {:#x} and {:#x} both hold logical block {:#x} with the same sequence number; \
				 using {:#x}",
				used.0,
				ignored.0,
				logical.get(),
				used.0
			),
			Self::MuOrphanedSecondary { logical, secondary, primary } => write!(
				formatter,
				"memory unit secondary block {:#x} of logical block {:#x} names primary block {:#x}, which does \
				 not hold it; the logical block reads as erased",
				secondary.0,
				logical.get(),
				primary.0
			),
		}
	}
}

/// One raw record: a sector's data followed by its spare area.
#[derive(Debug, Clone, Copy)]
pub struct SectorRecord<'a> {
	pub data: &'a [u8; SECTOR_DATA_SIZE],
	pub spare: &'a [u8; SPARE_SIZE],
}

impl<'a> SectorRecord<'a> {
	/// Splits a 0x210-byte record.
	pub fn new(record: &'a [u8; RAW_SECTOR_SIZE]) -> Self {
		let (data, spare) = record.split_at(SECTOR_DATA_SIZE);
		Self { data: data.try_into().expect("512 data bytes"), spare: spare.try_into().expect("16 spare bytes") }
	}

	/// An erased sector holds all ones and no ECC.
	pub fn is_erased(&self) -> bool {
		self.data.iter().chain(self.spare).all(|byte| *byte == 0xff)
	}

	/// The stored and computed ECC when they differ. An erased sector has no ECC to check.
	pub fn ecc_mismatch(&self) -> Option<EccMismatch> {
		if self.is_erased() {
			return None;
		}

		let stored = stored_ecc(self.spare);
		let computed = sector_ecc(self.data, self.spare);
		(stored != computed).then_some(EccMismatch { stored, computed })
	}
}

/// The 26-bit error-correcting code of a sector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ecc(u32);

impl Ecc {
	pub const fn get(self) -> u32 {
		self.0
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EccMismatch {
	pub stored: Ecc,
	pub computed: Ecc,
}

impl EccMismatch {
	pub const fn warning(self, sector: SectorIndex) -> RawWarning {
		RawWarning::EccMismatch { sector, stored: self.stored, computed: self.computed }
	}
}

/// The bad-block state that the spare areas of a raw image record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLayout {
	geometry: Geometry,
	bad_blocks: Vec<bool>,
	/// Entry `i` names the logical block that block `block_count - 1 - i` replaces.
	spare_map: Vec<Option<BlockIndex>>,
	warnings: Vec<RawWarning>,
}

impl RawLayout {
	/// Reads the bad-block markers and the spare-block records the way the kernel mount scan
	/// does. `metadata` holds the first-sector record of every configured block in order. A
	/// good block whose metadata sector fails its ECC check is used as stored and reported.
	pub fn new(geometry: Geometry, metadata: &[[u8; RAW_SECTOR_SIZE]]) -> Self {
		assert_eq!(metadata.len(), geometry.block_count, "one metadata record per configured block");

		let layout = geometry.kind.layout();
		let mut raw_layout = Self {
			geometry,
			bad_blocks: vec![false; geometry.block_count],
			spare_map: vec![None; geometry.spare_block_count],
			warnings: Vec::new(),
		};
		for (block, record) in metadata.iter().enumerate() {
			let record = SectorRecord::new(record);
			if layout.is_bad_block(record.spare) {
				raw_layout.bad_blocks[block] = true;
				continue;
			}
			if let Some(mismatch) = record.ecc_mismatch() {
				raw_layout.warnings.push(mismatch.warning(geometry.first_sector(BlockIndex(block))));
			}
			if block < geometry.logical_block_count() {
				continue;
			}

			let replaced = layout.block_number(record.spare);
			if replaced.0 < geometry.logical_block_count() {
				raw_layout.spare_map[geometry.block_count - 1 - block] = Some(replaced);
			}
		}

		raw_layout
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

	/// Spare blocks that replace no logical block and are not bad, in the order the kernel
	/// allocates them (`SfcxAllocateSpareBlockReplacement`).
	pub fn unassigned_spares(&self) -> impl Iterator<Item = BlockIndex> {
		self.spare_map.iter().enumerate().filter_map(|(index, replaced)| {
			let physical = self.geometry.block_count - 1 - index;
			(replaced.is_none() && !self.bad_blocks[physical]).then_some(BlockIndex(physical))
		})
	}

	/// The raw reads that cover the logical range, one per physically contiguous run.
	pub(crate) fn runs(&self, range: Range<u64>) -> Result<Vec<RawRun>, SfcxError> {
		let block_size = self.geometry.block_size() as u64;
		let mut runs = Vec::new();
		let mut position = range.start;
		while position < range.end {
			let logical_block = BlockIndex((position / block_size) as usize);
			let physical_block =
				self.translate(logical_block).ok_or(RawError::UntranslatedBlock { block: logical_block })?;
			let within_block = (position % block_size) as usize;
			let len = (block_size - within_block as u64).min(range.end - position) as usize;
			let skip = within_block % SECTOR_DATA_SIZE;
			let first_sector =
				SectorIndex(self.geometry.first_sector(physical_block).0 + within_block / SECTOR_DATA_SIZE);

			runs.push(RawRun::new(first_sector, skip, len));
			position += len as u64;
		}

		Ok(runs)
	}
}

/// A physically contiguous raw read and the logical bytes it yields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawRun {
	pub raw: Range<u64>,
	first_sector: SectorIndex,
	/// Data bytes of the first sector before the logical range.
	skip: usize,
	len: usize,
}

impl RawRun {
	/// The records that hold `len` data bytes starting `skip` bytes into `first_sector`.
	pub fn new(first_sector: SectorIndex, skip: usize, len: usize) -> Self {
		let sectors = (skip + len).div_ceil(SECTOR_DATA_SIZE);
		Self { raw: first_sector.records(sectors), first_sector, skip, len }
	}

	/// Extends this run by `next` when `next` starts at the data byte where this run ends.
	pub fn try_append(&mut self, next: &RawRun) -> bool {
		let end = self.first_sector.0 * SECTOR_DATA_SIZE + self.skip + self.len;
		if end != next.first_sector.0 * SECTOR_DATA_SIZE + next.skip {
			return false;
		}

		*self = Self::new(self.first_sector, self.skip, self.len + next.len);
		true
	}

	/// Appends the run's logical bytes from its raw records, checking each sector's ECC.
	pub fn strip(&self, raw: &[u8], out: &mut Vec<u8>, warnings: &mut Vec<RawWarning>) {
		let mut skip = self.skip;
		let mut remaining = self.len;
		for (index, record) in raw.as_chunks::<RAW_SECTOR_SIZE>().0.iter().enumerate() {
			let record = SectorRecord::new(record);
			if let Some(mismatch) = record.ecc_mismatch() {
				warnings.push(mismatch.warning(SectorIndex(self.first_sector.0 + index)));
			}

			let count = (SECTOR_DATA_SIZE - skip).min(remaining);
			out.extend_from_slice(&record.data[skip..skip + count]);
			remaining -= count;
			skip = 0;
		}
	}
}

/// Good logical blocks whose block-number field equals their position under `geometry`.
/// `metadata` holds the first-sector record of every logical block. ECC is not considered, so a
/// failed integrity check does not change which geometry an image has.
pub fn numbered_block_count(geometry: Geometry, metadata: &[[u8; RAW_SECTOR_SIZE]]) -> usize {
	let layout = geometry.kind.layout();
	metadata
		.iter()
		.enumerate()
		.filter(|(block, record)| {
			let record = SectorRecord::new(record);
			!record.is_erased() && !layout.is_bad_block(record.spare) && layout.block_number(record.spare).0 == *block
		})
		.count()
}

/// The 26-bit ECC of a sector over its 512 data bytes and the first 102 bits of its spare area,
/// as `SfcxEncodeECC` computes it: bits enter least significant first, polynomial 0x06954559.
pub fn sector_ecc(data: &[u8; SECTOR_DATA_SIZE], spare: &[u8; SPARE_SIZE]) -> Ecc {
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

	Ecc(!value & 0x03ff_ffff)
}

/// The ECC a spare area stores.
pub fn stored_ecc(spare: &[u8; SPARE_SIZE]) -> Ecc {
	let word = u32::from_le_bytes(spare[SPARE_ECC_OFFSET..].try_into().expect("four ECC bytes"));
	Ecc(word >> 6)
}
