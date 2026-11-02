//! The root directory of the system file system, located as the kernel mount locates it
//! (`SfcxMountVolume`). The mount scans the logical blocks from the last to the first and keeps
//! the newest complete copy of the root directory. On big-block flash, the metadata of the block
//! that holds the copy also records where the file system lies in the flash.

use std::ops::Range;

use crate::checked::Checked;
use crate::error::SfcxError;
use crate::machine::Machine;
use crate::machine::Step;
use crate::machine::expect_len;
use crate::raw::BlockIndex;
use crate::raw::Geometry;
use crate::raw::RAW_SECTOR_SIZE;
use crate::raw::RawLayout;
use crate::raw::RawWarning;
use crate::raw::SECTOR_DATA_SIZE;
use crate::raw::SPARE_SIZE;
use crate::raw::SectorIndex;
use crate::raw::SectorRecord;
use crate::raw::SpareLayout;

/// File system data blocks are 16 KiB (`SFCX_DATA_BLOCK_SHIFT`).
pub const DATA_BLOCK_SHIFT: u32 = 14;
/// A root directory copy: the directory pages interleaved with the file allocation table pages
/// (`SFCX_ROOT_DIRECTORY_SIZE`).
pub const ROOT_DIRECTORY_SIZE: usize = 0x4000;
const ROOT_DIRECTORY_SECTORS: usize = ROOT_DIRECTORY_SIZE / SECTOR_DATA_SIZE;
const BLOCK_TYPE_MASK: u8 = 0x3f;
/// Block types of the first-sector metadata (`SFCX_BLOCK_TYPE_*`).
const BLOCK_TYPE_BIG_BLOCK_DATA: u8 = 42;
const BLOCK_TYPE_BIG_BLOCK_ROOT_DIRECTORY: u8 = 44;
/// Mobile file 0 of small-block and big-on-small-block flash.
const BLOCK_TYPE_ROOT_DIRECTORY: u8 = 48;

/// The sequence number of a root directory copy. A rewrite stores the copy with the next number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SequenceNumber(u32);

impl SequenceNumber {
	/// An erased page (`SFCX_INVALID_BLOCK`).
	const INVALID: Self = Self(u32::MAX);

	pub const fn new(value: u32) -> Self {
		Self(value)
	}

	pub const fn get(self) -> u32 {
		self.0
	}

	/// The sequence number of a sector's metadata (`SfcxExtractSequenceNumber`). Only the least
	/// significant byte moves between spare layouts.
	fn parse(layout: SpareLayout, spare: &[u8; SPARE_SIZE]) -> Self {
		let low = match layout {
			SpareLayout::SmallBlock => spare[2],
			SpareLayout::BigOnSmallBlock => spare[0],
			SpareLayout::BigBlock => spare[5],
		};
		Self(u32::from_le_bytes([low, spare[3], spare[4], spare[6]]))
	}
}

/// Where the system file system lies in the logical blocks of the flash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlashRegions {
	kernel_reserved_block_count: usize,
	file_system_first_block: BlockIndex,
	file_system_block_count: usize,

	/// Logical offset of data block 0.
	data_offset: u64,
	data_block_count: u16,
}

impl FlashRegions {
	/// The layout the mount assumes until a root directory names another: no reserved blocks
	/// and a file system over every logical block.
	fn whole_device(geometry: Geometry) -> Self {
		Self::new(geometry, 0, BlockIndex::new(0), geometry.logical_block_count())
	}

	fn new(
		geometry: Geometry,
		kernel_reserved_block_count: usize,
		file_system_first_block: BlockIndex,
		file_system_block_count: usize,
	) -> Self {
		let data_blocks = file_system_block_count << (geometry.block_shift() - DATA_BLOCK_SHIFT);
		// The kernel keeps the count in a USHORT. Big-block metadata names at most 1020 blocks of
		// at most 16 data blocks, and small-block flash has at most 3968 logical blocks.
		let data_block_count = u16::try_from(data_blocks).expect("data block count fits the kernel's USHORT");

		Self {
			kernel_reserved_block_count,
			file_system_first_block,
			file_system_block_count,
			data_offset: (file_system_first_block.get() as u64) << geometry.block_shift(),
			data_block_count,
		}
	}

	pub const fn kernel_reserved_block_count(&self) -> usize {
		self.kernel_reserved_block_count
	}

	pub const fn file_system_first_block(&self) -> BlockIndex {
		self.file_system_first_block
	}

	pub const fn file_system_block_count(&self) -> usize {
		self.file_system_block_count
	}

	/// Logical offset of file system data block 0.
	pub const fn data_offset(&self) -> u64 {
		self.data_offset
	}

	/// Number of 16 KiB data blocks in the file system (`DataBlockCount`).
	pub const fn data_block_count(&self) -> u16 {
		self.data_block_count
	}
}

/// A complete root directory copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootLocation {
	block: BlockIndex,
	offset: u64,
	sequence: SequenceNumber,
}

impl RootLocation {
	/// The logical block that holds the copy.
	pub const fn block(&self) -> BlockIndex {
		self.block
	}

	/// Logical offset of the copy.
	pub const fn offset(&self) -> u64 {
		self.offset
	}

	pub const fn range(&self) -> Range<u64> {
		self.offset..self.offset + ROOT_DIRECTORY_SIZE as u64
	}

	pub const fn sequence(&self) -> SequenceNumber {
		self.sequence
	}
}

/// The outcome of the mount scan: the flash regions and the newest complete root directory
/// copy, if the flash has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootScan {
	regions: FlashRegions,
	root: Option<RootLocation>,
}

impl RootScan {
	pub const fn regions(&self) -> FlashRegions {
		self.regions
	}

	/// The copy the mount adopts. The kernel treats a copy with sequence number zero as no root
	/// directory (`SfcxMountVolume`).
	pub fn root(&self) -> Option<RootLocation> {
		self.root.filter(|root| root.sequence != SequenceNumber(0))
	}
}

/// Scans a raw image for the root directory as `SfcxMountVolume` does. A logical block that is
/// bad is read from the spare block that replaces it.
///
/// On small-block and big-on-small-block flash, a root directory copy fills a block whose
/// first sector has block type 48 (`SfcxInspectSmallMobileBlock`); the file system covers every
/// logical block. On big-block flash, a block whose first sector has block type 42 or 44 names
/// the flash regions in its metadata, and each of its 16 KiB slots whose first sector has block
/// type 44 may hold a copy (`SfcxInspectLargeDataBlock`). Either way, a copy qualifies when its
/// sequence number is at least the adopted one and every one of its sectors carries the same
/// sequence number and block type.
#[derive(Debug)]
pub struct LocateRoot<'a> {
	layout: &'a RawLayout,
	spare_layout: SpareLayout,
	state: LocateState,
	regions: FlashRegions,
	root: Option<RootLocation>,
	warnings: Vec<RawWarning>,
	requested: bool,
}

#[derive(Debug)]
enum LocateState {
	/// Reading the first sector of a logical block.
	Block {
		block: BlockIndex,
		physical: BlockIndex,
	},
	/// Reading the first sector of a root directory slot.
	Slot {
		candidate: Candidate,
		slot: usize,
	},
	/// Reading the remaining sectors of a slot whose first sector qualifies.
	Verify {
		candidate: Candidate,
		slot: usize,
		sequence: SequenceNumber,
	},
	/// Every logical block is scanned; the next step returns the scan.
	Done,
	Finished,
}

/// A logical block that may hold root directory copies, and the regions a copy in it names.
#[derive(Debug, Clone, Copy)]
struct Candidate {
	block: BlockIndex,
	physical: BlockIndex,
	regions: FlashRegions,
}

/// Big-block metadata of the first sector of a file system block (`BigBlockRootDirectory`).
#[derive(Debug, Clone, Copy)]
struct RegionMetadata {
	kernel_reserved_block_count: usize,
	file_system_block_count: usize,
	trailing_reserved_block_count: usize,
}

impl<'a> LocateRoot<'a> {
	pub fn new(layout: &'a RawLayout) -> Self {
		let geometry = layout.geometry();
		let mut scan = Self {
			layout,
			spare_layout: geometry.kind().layout(),
			state: LocateState::Done,
			regions: FlashRegions::whole_device(geometry),
			root: None,
			warnings: Vec::new(),
			requested: false,
		};
		scan.next_block(BlockIndex::new(geometry.logical_block_count()));

		scan
	}

	fn pending_range(&self) -> Option<Range<u64>> {
		let geometry = self.layout.geometry();
		match &self.state {
			LocateState::Block { physical, .. } => Some(geometry.metadata_range(*physical)),
			LocateState::Slot { candidate, slot } => Some(slot_sector(geometry, candidate.physical, *slot).records(1)),
			LocateState::Verify { candidate, slot, .. } => {
				let first = slot_sector(geometry, candidate.physical, *slot);
				Some(SectorIndex::new(first.get() + 1).records(ROOT_DIRECTORY_SECTORS - 1))
			}
			LocateState::Done | LocateState::Finished => None,
		}
	}

	fn adopted_sequence(&self) -> SequenceNumber {
		self.root.map_or(SequenceNumber(0), |root| root.sequence)
	}

	fn root_block_type(&self) -> u8 {
		match self.spare_layout {
			SpareLayout::BigBlock => BLOCK_TYPE_BIG_BLOCK_ROOT_DIRECTORY,
			SpareLayout::SmallBlock | SpareLayout::BigOnSmallBlock => BLOCK_TYPE_ROOT_DIRECTORY,
		}
	}

	/// Moves to the next translatable logical block below `before`, or to the end of the scan.
	fn next_block(&mut self, before: BlockIndex) {
		self.state = (0..before.get())
			.rev()
			.map(BlockIndex::new)
			.find_map(|block| Some(LocateState::Block { block, physical: self.layout.translate(block)? }))
			.unwrap_or(LocateState::Done);
	}

	fn next_slot(&mut self, candidate: Candidate, slot: usize) {
		let slots = self.layout.geometry().block_size() / ROOT_DIRECTORY_SIZE;
		if slot + 1 < slots {
			self.state = LocateState::Slot { candidate, slot: slot + 1 };
			return;
		}

		self.next_block(candidate.block);
	}

	/// Checks the first sector of a logical block for the block types that may hold a root
	/// directory copy.
	fn inspect_block(&mut self, block: BlockIndex, physical: BlockIndex, spare: &[u8; SPARE_SIZE]) {
		let block_type = spare[12] & BLOCK_TYPE_MASK;
		let regions = match self.spare_layout {
			SpareLayout::BigBlock => {
				if !matches!(block_type, BLOCK_TYPE_BIG_BLOCK_DATA | BLOCK_TYPE_BIG_BLOCK_ROOT_DIRECTORY) {
					return self.next_block(block);
				}
				let Some(regions) = RegionMetadata::parse(spare).regions(self.layout.geometry(), block, self.regions)
				else {
					return self.next_block(block);
				};
				regions
			}
			SpareLayout::SmallBlock | SpareLayout::BigOnSmallBlock => {
				if block_type != BLOCK_TYPE_ROOT_DIRECTORY {
					return self.next_block(block);
				}
				self.regions
			}
		};

		self.inspect_slot(Candidate { block, physical, regions }, 0, spare);
	}

	/// Checks the first sector of a root directory slot. A small-block copy must also fill its
	/// block: no reserved pages (`SfcxInspectSmallMobileBlock`).
	fn inspect_slot(&mut self, candidate: Candidate, slot: usize, spare: &[u8; SPARE_SIZE]) {
		const RESERVED_PAGES_OFFSET: usize = 9;

		let sequence = SequenceNumber::parse(self.spare_layout, spare);
		let qualifies = spare[12] & BLOCK_TYPE_MASK == self.root_block_type()
			&& sequence >= self.adopted_sequence()
			&& match self.spare_layout {
				SpareLayout::BigBlock => true,
				SpareLayout::SmallBlock | SpareLayout::BigOnSmallBlock => {
					sequence != SequenceNumber::INVALID && spare[RESERVED_PAGES_OFFSET] == 0
				}
			};
		if qualifies {
			self.state = LocateState::Verify { candidate, slot, sequence };
			return;
		}

		self.next_slot(candidate, slot);
	}

	fn check_ecc(&mut self, first: SectorIndex, records: &[[u8; RAW_SECTOR_SIZE]]) {
		for (index, record) in records.iter().enumerate() {
			if let Some(mismatch) = SectorRecord::new(record).ecc_mismatch() {
				self.warnings.push(mismatch.warning(SectorIndex::new(first.get() + index)));
			}
		}
	}
}

impl Machine for LocateRoot<'_> {
	type Output = Checked<RootScan>;
	type Error = SfcxError;

	fn step(&mut self) -> Result<Step<Self::Output>, SfcxError> {
		assert!(!self.requested, "LocateRoot stepped before the requested read was fed");
		if let Some(range) = self.pending_range() {
			self.requested = true;
			return Ok(Step::Read(range));
		}

		assert!(matches!(self.state, LocateState::Done), "LocateRoot stepped after completion");
		self.state = LocateState::Finished;
		let mut warnings = std::mem::take(&mut self.warnings);
		warnings.sort_unstable();
		warnings.dedup();

		Ok(Step::Done(Checked::new(RootScan { regions: self.regions, root: self.root }, warnings)))
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), SfcxError> {
		assert!(self.requested, "LocateRoot fed without a requested read");
		self.requested = false;

		let range = self.pending_range().expect("a requested read is pending");
		expect_len(&range, bytes)?;
		let records = bytes.as_chunks::<RAW_SECTOR_SIZE>().0;
		self.check_ecc(SectorIndex::new((range.start / RAW_SECTOR_SIZE as u64) as usize), records);
		let first = SectorRecord::new(&records[0]);

		match std::mem::replace(&mut self.state, LocateState::Done) {
			LocateState::Block { block, physical } => self.inspect_block(block, physical, first.spare),
			LocateState::Slot { candidate, slot } => self.inspect_slot(candidate, slot, first.spare),
			LocateState::Verify { candidate, slot, sequence } => {
				let root_type = self.root_block_type();
				let intact = records.iter().all(|record| {
					let spare = SectorRecord::new(record).spare;
					SequenceNumber::parse(self.spare_layout, spare) == sequence
						&& spare[12] & BLOCK_TYPE_MASK == root_type
				});
				if intact {
					let block_offset = (candidate.block.get() as u64) << self.layout.geometry().block_shift();
					self.root = Some(RootLocation {
						block: candidate.block,
						offset: block_offset + (slot * ROOT_DIRECTORY_SIZE) as u64,
						sequence,
					});
					self.regions = candidate.regions;
				}

				self.next_slot(candidate, slot);
			}
			LocateState::Done | LocateState::Finished => unreachable!("no read is pending once the scan is done"),
		}

		Ok(())
	}
}

impl RegionMetadata {
	fn parse(spare: &[u8; SPARE_SIZE]) -> Self {
		Self {
			kernel_reserved_block_count: usize::from(spare[7]),
			// Recorded in units of four blocks.
			file_system_block_count: usize::from(spare[8]) * 4,
			trailing_reserved_block_count: usize::from(spare[9]),
		}
	}

	/// The flash regions this metadata names for logical block `block`, when the kernel would
	/// accept them given the regions adopted so far.
	fn regions(&self, geometry: Geometry, block: BlockIndex, adopted: FlashRegions) -> Option<FlashRegions> {
		let logical_blocks = geometry.logical_block_count();
		let reserved =
			self.trailing_reserved_block_count + self.file_system_block_count + self.kernel_reserved_block_count;
		if reserved > logical_blocks {
			return None;
		}

		let file_system_first_block =
			BlockIndex::new(logical_blocks - self.trailing_reserved_block_count - self.file_system_block_count);
		let accepted = block >= file_system_first_block
			&& self.kernel_reserved_block_count != 0
			&& self.file_system_block_count != 0
			&& file_system_first_block >= adopted.file_system_first_block;

		accepted.then(|| {
			FlashRegions::new(
				geometry,
				self.kernel_reserved_block_count,
				file_system_first_block,
				self.file_system_block_count,
			)
		})
	}
}

fn slot_sector(geometry: Geometry, block: BlockIndex, slot: usize) -> SectorIndex {
	SectorIndex::new(geometry.first_sector(block).get() + slot * ROOT_DIRECTORY_SECTORS)
}
