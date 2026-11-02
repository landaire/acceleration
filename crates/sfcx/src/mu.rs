//! The memory unit that the kernel keeps in the flash blocks outside the system file system
//! (`mu.c`): the blocks between the kernel reserved blocks and the file system, and every block
//! past the configured flash size. The memory unit is a block device of 4 KiB pages. A logical
//! block of the memory unit is held by a primary physical block and, while it is rewritten, by a
//! secondary block with newer copies of some of its pages. The spare areas of the last committed
//! page of a physical block map its physical pages to logical pages.
//!
//! [`FlashImage::memory_unit`] scans the blocks as the kernel mount does and returns a
//! [`MemoryUnit`], which translates memory unit offsets to raw reads. [`MemoryUnitSource`] and
//! [`AsyncMemoryUnitSource`] present the memory unit as a [`ReadAt`] or [`AsyncReadAt`] source.

use std::io;
use std::iter;
use std::ops::Range;
use std::sync::Mutex;
use std::sync::PoisonError;

use crate::checked::Checked;
use crate::error::MemoryUnitError;
use crate::error::SfcxError;
use crate::image::FlashImage;
use crate::image::Layout;
use crate::io::AsyncReadAt;
use crate::io::ReadAt;
use crate::io::drive;
use crate::io::drive_async;
use crate::machine::Machine;
use crate::machine::Step;
use crate::machine::expect_len;
use crate::raw::BlockIndex;
use crate::raw::Geometry;
use crate::raw::RAW_SECTOR_SIZE;
use crate::raw::RawKind;
use crate::raw::RawLayout;
use crate::raw::RawRun;
use crate::raw::RawWarning;
use crate::raw::SECTOR_DATA_SIZE;
use crate::raw::SPARE_SIZE;
use crate::raw::SectorIndex;
use crate::raw::SectorRecord;
use crate::raw::SpareLayout;

pub const MU_PAGE_SIZE: usize = 0x1000;
/// The system partition is the last 8 MiB less a page of the memory unit (`SFCX_MU_SYSTEM_PARTITION_SIZE`).
pub const SYSTEM_PARTITION_SIZE: u64 = 0x7f_f000;
const MU_PAGE_SHIFT: u32 = 12;
const SECTORS_PER_PAGE: usize = MU_PAGE_SIZE / SECTOR_DATA_SIZE;
const PAGE_TYPE_MASK: u8 = 0x3f;
const PAGE_TYPE_DATA: u8 = 40;
const PAGE_TYPE_COMMIT: u8 = 41;
const BLOCK_NUMBER_MASK: usize = 0x0fff;
/// `SFCX_ROOT_DIRECTORY_SIZE`: a root directory and its file allocation table.
const ROOT_SLOT_SIZE: usize = 0x4000;
const ROOT_SLOT_SECTORS: usize = ROOT_SLOT_SIZE / SECTOR_DATA_SIZE;

/// A block of the memory unit's logical address space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MuBlockIndex(usize);

impl MuBlockIndex {
	pub const fn new(value: usize) -> Self {
		Self(value)
	}

	pub const fn get(self) -> usize {
		self.0
	}
}

/// The two partition devices the kernel creates over the memory unit (`SfcxMuMountStorage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryUnitPartitions {
	/// `\Device\BuiltInMuSfc`: everything before the system partition.
	pub data: Range<u64>,
	/// `\Device\BuiltInMuSfcSystem`: the last [`SYSTEM_PARTITION_SIZE`] bytes.
	pub system: Range<u64>,
}

/// A mounted memory unit: its size and where each of its pages is in the raw image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryUnit {
	size: u64,
	/// First sector of the physical page that holds each logical page, or `None` for a page that
	/// was never written.
	pages: Vec<Option<SectorIndex>>,
}

impl MemoryUnit {
	pub const fn size(&self) -> u64 {
		self.size
	}

	pub fn partitions(&self) -> MemoryUnitPartitions {
		let system_start = self.size - SYSTEM_PARTITION_SIZE;
		MemoryUnitPartitions { data: 0..system_start, system: system_start..self.size }
	}

	/// A machine that reads memory unit `ranges` and returns their bytes in order.
	pub fn reader<I>(&self, ranges: I) -> Result<ReadMemoryUnit, SfcxError>
	where
		I: IntoIterator<Item = Range<u64>>,
	{
		ReadMemoryUnit::new(self, ranges)
	}

	pub fn read<R>(&self, source: &R, range: Range<u64>) -> Result<Checked<Vec<u8>>, SfcxError>
	where
		R: ReadAt,
	{
		drive(self.reader(iter::once(range))?, source)
	}

	pub async fn read_async<R>(&self, source: &R, range: Range<u64>) -> Result<Checked<Vec<u8>>, SfcxError>
	where
		R: AsyncReadAt,
	{
		drive_async(self.reader(iter::once(range))?, source).await
	}
}

impl FlashImage {
	/// A machine that scans the raw image for the memory unit.
	pub fn memory_unit_scanner(&self) -> Result<OpenMemoryUnit<'_>, SfcxError> {
		let Layout::Raw(layout) = self.layout() else {
			return Err(MemoryUnitError::LogicalImage.into());
		};

		OpenMemoryUnit::new(layout)
	}

	/// Mounts the memory unit of a raw image.
	pub fn memory_unit<R>(&self, source: &R) -> Result<Checked<MemoryUnit>, SfcxError>
	where
		R: ReadAt,
	{
		drive(self.memory_unit_scanner()?, source)
	}

	pub async fn memory_unit_async<R>(&self, source: &R) -> Result<Checked<MemoryUnit>, SfcxError>
	where
		R: AsyncReadAt,
	{
		drive_async(self.memory_unit_scanner()?, source).await
	}
}

/// The memory unit of a raw image as a [`ReadAt`] source. Failed integrity checks do not fail a
/// read; they are collected, without duplicates, for [`MemoryUnitSource::warnings`].
#[derive(Debug)]
pub struct MemoryUnitSource<'a, S> {
	unit: &'a MemoryUnit,
	source: &'a S,
	warnings: Mutex<Vec<RawWarning>>,
}

impl<'a, S> MemoryUnitSource<'a, S> {
	/// Reads `unit` from `source`, the raw image it was scanned from.
	pub fn new(unit: &'a MemoryUnit, source: &'a S) -> Self {
		Self { unit, source, warnings: Mutex::new(Vec::new()) }
	}

	pub fn warnings(&self) -> Vec<RawWarning> {
		self.warnings.lock().unwrap_or_else(PoisonError::into_inner).clone()
	}

	pub fn take_warnings(&self) -> Vec<RawWarning> {
		std::mem::take(&mut *self.warnings.lock().unwrap_or_else(PoisonError::into_inner))
	}
}

impl<S> ReadAt for MemoryUnitSource<'_, S>
where
	S: ReadAt,
{
	fn read_at(&self, _context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		let bytes = self.unit.read(self.source, range).map_err(io::Error::other)?;
		Ok(collect_warnings(&self.warnings, bytes))
	}
}

/// The memory unit of a raw image as an [`AsyncReadAt`] source. Failed integrity checks are
/// collected as [`MemoryUnitSource`] collects them.
#[derive(Debug)]
pub struct AsyncMemoryUnitSource<'a, S> {
	unit: &'a MemoryUnit,
	source: &'a S,
	warnings: Mutex<Vec<RawWarning>>,
}

impl<'a, S> AsyncMemoryUnitSource<'a, S> {
	/// Reads `unit` from `source`, the raw image it was scanned from.
	pub fn new(unit: &'a MemoryUnit, source: &'a S) -> Self {
		Self { unit, source, warnings: Mutex::new(Vec::new()) }
	}

	pub fn warnings(&self) -> Vec<RawWarning> {
		self.warnings.lock().unwrap_or_else(PoisonError::into_inner).clone()
	}

	pub fn take_warnings(&self) -> Vec<RawWarning> {
		std::mem::take(&mut *self.warnings.lock().unwrap_or_else(PoisonError::into_inner))
	}
}

impl<S> AsyncReadAt for AsyncMemoryUnitSource<'_, S>
where
	S: AsyncReadAt,
{
	async fn read_at(&self, _context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		let bytes = self.unit.read_async(self.source, range).await.map_err(io::Error::other)?;
		Ok(collect_warnings(&self.warnings, bytes))
	}
}

/// Reads memory unit ranges. Pages that were never written read as erased (all ones), as
/// `SfcxMuStartRead` returns them.
#[derive(Debug)]
pub struct ReadMemoryUnit {
	pieces: Vec<Piece>,
	next: usize,
	bytes: Vec<u8>,
	warnings: Vec<RawWarning>,
	requested: bool,
	finished: bool,
}

#[derive(Debug)]
enum Piece {
	Erased(usize),
	Raw(RawRun),
}

impl ReadMemoryUnit {
	fn new<I>(unit: &MemoryUnit, ranges: I) -> Result<Self, SfcxError>
	where
		I: IntoIterator<Item = Range<u64>>,
	{
		let mut pieces: Vec<Piece> = Vec::new();
		let mut total = 0u64;
		for range in ranges {
			if range.start > range.end || range.end > unit.size {
				return Err(MemoryUnitError::RangeOutOfBounds { range, size: unit.size }.into());
			}
			total = total
				.checked_add(range.end - range.start)
				.filter(|total| usize::try_from(*total).is_ok_and(|total| total <= isize::MAX as usize))
				.ok_or(SfcxError::ReadTooLarge)?;

			let mut position = range.start;
			while position < range.end {
				let within_page = (position % MU_PAGE_SIZE as u64) as usize;
				let len = (MU_PAGE_SIZE - within_page).min((range.end - position) as usize);
				let piece = match unit.pages[(position >> MU_PAGE_SHIFT) as usize] {
					None => Piece::Erased(len),
					Some(page) => {
						let first_sector = SectorIndex::new(page.get() + within_page / SECTOR_DATA_SIZE);
						Piece::Raw(RawRun::new(first_sector, within_page % SECTOR_DATA_SIZE, len))
					}
				};
				push_piece(&mut pieces, piece);
				position += len as u64;
			}
		}

		Ok(Self {
			pieces,
			next: 0,
			bytes: Vec::with_capacity(total as usize),
			warnings: Vec::new(),
			requested: false,
			finished: false,
		})
	}
}

impl Machine for ReadMemoryUnit {
	type Output = Checked<Vec<u8>>;
	type Error = SfcxError;

	fn step(&mut self) -> Result<Step<Self::Output>, SfcxError> {
		assert!(!self.requested, "ReadMemoryUnit stepped before the requested read was fed");
		assert!(!self.finished, "ReadMemoryUnit stepped after completion");

		while let Some(piece) = self.pieces.get(self.next) {
			match piece {
				Piece::Erased(len) => {
					self.bytes.resize(self.bytes.len() + len, 0xff);
					self.next += 1;
				}
				Piece::Raw(run) => {
					self.requested = true;
					return Ok(Step::Read(run.raw.clone()));
				}
			}
		}

		self.finished = true;
		Ok(Step::Done(Checked::new(std::mem::take(&mut self.bytes), std::mem::take(&mut self.warnings))))
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), SfcxError> {
		assert!(self.requested, "ReadMemoryUnit fed without a requested read");
		self.requested = false;

		let Some(Piece::Raw(run)) = self.pieces.get(self.next) else {
			unreachable!("a raw piece is pending while a read is requested");
		};
		expect_len(&run.raw, bytes)?;
		run.strip(bytes, &mut self.bytes, &mut self.warnings);
		self.next += 1;

		Ok(())
	}
}

/// Scans a raw image for the memory unit as `SfcxMuMountStorage` does. The scan first finds the
/// newest root directory of the system file system, whose block metadata records the blocks the
/// kernel reserves and where the file system starts, then inspects every block that may hold
/// memory unit data.
#[derive(Debug)]
pub struct OpenMemoryUnit<'a> {
	layout: &'a RawLayout,
	shape: BlockShape,
	state: ScanState,
	flash: FlashRegions,
	/// Sequence number of the adopted root directory. The kernel starts from zero.
	root_sequence: u32,
	size: u64,
	targets: Vec<ScanTarget>,
	/// Inspected blocks with a committed page, by physical block.
	blocks: Vec<Option<InspectedBlock>>,
	/// The newest physical block of each logical block (`MuLogicalToPhysicalMap`).
	mapping: Vec<Option<BlockIndex>>,
	warnings: Vec<RawWarning>,
	requested: bool,
}

#[derive(Debug)]
enum ScanState {
	/// Reading the first sector of a logical block of the system flash.
	RootBlock {
		block: usize,
		physical: BlockIndex,
	},
	/// Reading the first sector of a root directory slot.
	RootSlot {
		root: RootBlock,
		slot: usize,
	},
	/// Reading the remaining sectors of a root directory slot whose first sector qualifies.
	RootVerify {
		root: RootBlock,
		slot: usize,
		sequence: u32,
	},
	/// Reading the first sector of a scan target.
	BlockHeader {
		target: usize,
	},
	/// Binary search for the last written page.
	Search {
		target: usize,
		first: PageMetadata,
		low: usize,
		high: usize,
		probe: usize,
	},
	/// Reading every sector of page `low - 1`, looking for the last fully committed page.
	Walk {
		target: usize,
		first: PageMetadata,
		low: usize,
	},
	/// Every block is scanned; the next step returns the memory unit.
	Done,
	Finished,
}

/// A logical system flash block whose first-sector metadata names a valid flash layout.
#[derive(Debug, Clone, Copy)]
struct RootBlock {
	block: usize,
	physical: BlockIndex,
	regions: FlashRegions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FlashRegions {
	kernel_reserved_block_count: usize,
	file_system_first_block: usize,
}

/// Memory unit geometry for a block size (`SfcxInitializeHardware`).
#[derive(Debug, Clone, Copy)]
struct BlockShape {
	geometry: Geometry,
	pages_per_block: usize,
	bits_per_index: u32,
	page_map_size: usize,
}

#[derive(Debug, Clone, Copy)]
struct ScanTarget {
	block: BlockIndex,
	/// Blocks past the configured flash size carry their own bad-block marker; blocks of the
	/// system flash were already translated past bad blocks.
	check_bad_marker: bool,
}

#[derive(Debug, Clone, Copy)]
struct PageMetadata {
	logical_block: MuBlockIndex,
	/// Primary block of a secondary block.
	source_block: Option<BlockIndex>,
	sequence: u64,
	page_type: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InspectedBlock {
	link: BlockLink,
	sequence: u64,
	/// Logical page index of each physical page up to the last committed page.
	page_map: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockLink {
	Primary { logical: MuBlockIndex },
	Secondary { primary: BlockIndex },
}

/// Big-block metadata of the first sector of a system flash block (`BigBlockRootDirectory`).
#[derive(Debug, Clone, Copy)]
struct RootMetadata {
	sequence: u32,
	block_type: u8,
	kernel_reserved_block_count: usize,
	file_system_block_count: usize,
	trailing_reserved_block_count: usize,
}

impl<'a> OpenMemoryUnit<'a> {
	pub fn new(layout: &'a RawLayout) -> Result<Self, SfcxError> {
		let geometry = layout.geometry();
		if !matches!(geometry.kind(), RawKind::BigBlock | RawKind::BigBlock256K) {
			return Err(MemoryUnitError::UnsupportedKind { kind: geometry.kind() }.into());
		}

		let pages_per_block = geometry.block_size() / MU_PAGE_SIZE;
		let bits_per_index = geometry.block_shift() - MU_PAGE_SHIFT;
		let shape = BlockShape {
			geometry,
			pages_per_block,
			bits_per_index,
			page_map_size: pages_per_block * bits_per_index as usize / 8,
		};
		let mut scan = Self {
			layout,
			shape,
			state: ScanState::Done,
			// The kernel starts the mount with no reserved blocks and the file system at block 0
			// (`SfcxMountVolume`); a flash without a root directory keeps them.
			flash: FlashRegions { kernel_reserved_block_count: 0, file_system_first_block: 0 },
			root_sequence: 0,
			size: 0,
			targets: Vec::new(),
			blocks: vec![None; geometry.physical_block_count()],
			mapping: vec![None; BLOCK_NUMBER_MASK + 1],
			warnings: Vec::new(),
			requested: false,
		};
		scan.next_root_block(geometry.logical_block_count())?;

		Ok(scan)
	}

	fn pending_range(&self) -> Option<Range<u64>> {
		let geometry = self.shape.geometry;
		match &self.state {
			ScanState::RootBlock { physical, .. } => Some(geometry.metadata_range(*physical)),
			ScanState::RootSlot { root, slot } => Some(root_slot_sector(geometry, root.physical, *slot).records(1)),
			ScanState::RootVerify { root, slot, .. } => {
				let first = root_slot_sector(geometry, root.physical, *slot);
				Some(SectorIndex::new(first.get() + 1).records(ROOT_SLOT_SECTORS - 1))
			}
			ScanState::BlockHeader { target } => Some(geometry.metadata_range(self.targets[*target].block)),
			ScanState::Search { target, probe, .. } => Some(self.page_sector(*target, *probe).records(1)),
			ScanState::Walk { target, low, .. } => Some(self.page_sector(*target, low - 1).records(SECTORS_PER_PAGE)),
			ScanState::Done | ScanState::Finished => None,
		}
	}

	fn page_sector(&self, target: usize, page: usize) -> SectorIndex {
		let block = self.shape.geometry.first_sector(self.targets[target].block);
		SectorIndex::new(block.get() + page * SECTORS_PER_PAGE)
	}

	/// Moves to the next translatable logical block below `before`, scanning downward as the
	/// kernel mount does, or to the memory unit scan once none remains.
	fn next_root_block(&mut self, before: usize) -> Result<(), SfcxError> {
		for block in (0..before).rev() {
			if let Some(physical) = self.layout.translate(BlockIndex::new(block)) {
				self.state = ScanState::RootBlock { block, physical };
				return Ok(());
			}
		}

		self.start_memory_unit_scan()
	}

	/// Moves past root directory slot `slot` of `root`.
	fn next_root_slot(&mut self, root: RootBlock, slot: usize) -> Result<(), SfcxError> {
		let slots = self.shape.geometry.block_size() / ROOT_SLOT_SIZE;
		if slot + 1 < slots {
			self.state = ScanState::RootSlot { root, slot: slot + 1 };
			return Ok(());
		}

		self.next_root_block(root.block)
	}

	/// Checks the first sector of a root directory slot (`SfcxInspectLargeDataBlock`).
	fn inspect_root_slot(&mut self, root: RootBlock, slot: usize, record: &SectorRecord<'_>) -> Result<(), SfcxError> {
		let metadata = RootMetadata::parse(record.spare);
		if metadata.block_type == RootMetadata::BLOCK_TYPE_ROOT_DIRECTORY && metadata.sequence >= self.root_sequence {
			self.state = ScanState::RootVerify { root, slot, sequence: metadata.sequence };
			return Ok(());
		}

		self.next_root_slot(root, slot)
	}

	/// Sizes the memory unit (`SfcxMuMountStorage`) and lists the blocks to scan: the blocks
	/// past the configured flash size, then the system flash blocks between the kernel
	/// reserved blocks and the file system (`SfcxMuScanMemoryUnitBlocks`).
	fn start_memory_unit_scan(&mut self) -> Result<(), SfcxError> {
		let geometry = self.shape.geometry;
		self.size = memory_unit_size(geometry, self.flash)?;

		let beyond = (geometry.block_count()..geometry.physical_block_count())
			.map(|block| ScanTarget { block: BlockIndex::new(block), check_bad_marker: true });
		self.targets.extend(beyond);

		// A bad block with no replacement gets the next unassigned spare, as
		// `SfcxAllocateSpareBlockReplacement` assigns one during the scan.
		let mut spares = self.layout.unassigned_spares();
		for block in self.flash.kernel_reserved_block_count..self.flash.file_system_first_block {
			let physical = self.layout.translate(BlockIndex::new(block)).or_else(|| spares.next());
			if let Some(block) = physical {
				self.targets.push(ScanTarget { block, check_bad_marker: false });
			}
		}

		self.next_target(0);
		Ok(())
	}

	fn next_target(&mut self, target: usize) {
		self.state = if target < self.targets.len() { ScanState::BlockHeader { target } } else { ScanState::Done };
	}

	/// Continues the binary search for the last written page (`SfcxMuInspectMemoryUnitBlock`).
	fn search(&mut self, target: usize, first: PageMetadata, low: usize, high: usize) {
		if low < high {
			self.state = ScanState::Search { target, first, low, high, probe: (low + high) / 2 };
			return;
		}

		self.walk(target, first, low);
	}

	/// Steps back to the page before `low`, or to the next block when no page is committed.
	fn walk(&mut self, target: usize, first: PageMetadata, low: usize) {
		if low == 0 {
			self.next_target(target + 1);
			return;
		}

		self.state = ScanState::Walk { target, first, low };
	}

	/// Records an inspected block and maps its logical block to it when its sequence number is
	/// the newest (`SfcxMuScanMemoryUnitBlocks`).
	fn record_block(&mut self, target: usize, first: PageMetadata, last_page: PageMetadata, page_map: Vec<u8>) {
		let physical = self.targets[target].block;
		let link = match last_page.source_block {
			Some(primary) => BlockLink::Secondary { primary },
			None => BlockLink::Primary { logical: first.logical_block },
		};

		let logical = first.logical_block;
		match self.mapping[logical.0] {
			Some(mapped) => {
				let mapped_sequence = self.blocks[mapped.get()].as_ref().expect("mapped blocks are inspected").sequence;
				if mapped_sequence < first.sequence {
					self.mapping[logical.0] = Some(physical);
				} else if mapped_sequence == first.sequence {
					self.warnings.push(RawWarning::MuDuplicateSequence { logical, used: mapped, ignored: physical });
				}
			}
			None => self.mapping[logical.0] = Some(physical),
		}

		self.blocks[physical.get()] = Some(InspectedBlock { link, sequence: first.sequence, page_map });
	}

	/// Resolves every logical page to the physical page the kernel reads for it
	/// (`SfcxMuStartRead`).
	fn finish(&mut self) -> MemoryUnit {
		let pages_per_block = self.shape.pages_per_block;
		let logical_blocks = (self.size >> self.shape.geometry.block_shift()) as usize;
		let mut pages = Vec::with_capacity(logical_blocks * pages_per_block);
		for logical in (0..logical_blocks).map(MuBlockIndex) {
			let Some(physical) = self.mapping[logical.0] else {
				pages.extend(iter::repeat_n(None, pages_per_block));
				continue;
			};

			let block = self.blocks[physical.get()].as_ref().expect("mapped blocks are inspected");
			let primary = match block.link {
				BlockLink::Primary { .. } => None,
				BlockLink::Secondary { primary } => {
					let linked = self.blocks.get(primary.get()).and_then(Option::as_ref);
					match linked {
						Some(linked) if linked.link == BlockLink::Primary { logical } => Some((primary, linked)),
						_ => {
							// `SfcxMuMountStorage` drops a secondary block whose primary block does not
							// hold its logical block.
							self.warnings.push(RawWarning::MuOrphanedSecondary {
								logical,
								secondary: physical,
								primary,
							});
							pages.extend(iter::repeat_n(None, pages_per_block));
							continue;
						}
					}
				}
			};

			for page in 0..pages_per_block {
				let location = physical_page(&block.page_map, page).map(|index| (physical, index)).or_else(|| {
					primary.and_then(|(block, linked)| Some((block, physical_page(&linked.page_map, page)?)))
				});
				pages.push(location.map(|(block, index)| {
					SectorIndex::new(self.shape.geometry.first_sector(block).get() + index * SECTORS_PER_PAGE)
				}));
			}
		}

		MemoryUnit { size: self.size, pages }
	}

	fn check_ecc(&mut self, first: SectorIndex, records: &[u8]) {
		for (index, record) in records.as_chunks::<RAW_SECTOR_SIZE>().0.iter().enumerate() {
			if let Some(mismatch) = SectorRecord::new(record).ecc_mismatch() {
				self.warnings.push(mismatch.warning(SectorIndex::new(first.get() + index)));
			}
		}
	}
}

impl Machine for OpenMemoryUnit<'_> {
	type Output = Checked<MemoryUnit>;
	type Error = SfcxError;

	fn step(&mut self) -> Result<Step<Self::Output>, SfcxError> {
		assert!(!self.requested, "OpenMemoryUnit stepped before the requested read was fed");
		if let Some(range) = self.pending_range() {
			self.requested = true;
			return Ok(Step::Read(range));
		}

		assert!(matches!(self.state, ScanState::Done), "OpenMemoryUnit stepped after completion");
		self.state = ScanState::Finished;
		let unit = self.finish();
		let mut warnings = std::mem::take(&mut self.warnings);
		warnings.sort_unstable();
		warnings.dedup();

		Ok(Step::Done(Checked::new(unit, warnings)))
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), SfcxError> {
		assert!(self.requested, "OpenMemoryUnit fed without a requested read");
		self.requested = false;

		let range = self.pending_range().expect("a requested read is pending");
		expect_len(&range, bytes)?;
		let records = bytes.as_chunks::<RAW_SECTOR_SIZE>().0;
		let first = SectorRecord::new(&records[0]);
		// A bad block's metadata is not trusted, so its ECC is not checked either.
		let bad_block = matches!(
			self.state,
			ScanState::BlockHeader { target }
				if self.targets[target].check_bad_marker && SpareLayout::BigBlock.is_bad_block(first.spare)
		);
		if !bad_block {
			let first_sector = SectorIndex::new((range.start / RAW_SECTOR_SIZE as u64) as usize);
			self.check_ecc(first_sector, bytes);
		}

		match std::mem::replace(&mut self.state, ScanState::Done) {
			ScanState::RootBlock { block, physical } => {
				let metadata = RootMetadata::parse(first.spare);
				if !matches!(
					metadata.block_type,
					RootMetadata::BLOCK_TYPE_DATA | RootMetadata::BLOCK_TYPE_ROOT_DIRECTORY
				) {
					return self.next_root_block(block);
				}
				let Some(regions) = metadata.regions(self.shape.geometry, block, self.flash) else {
					return self.next_root_block(block);
				};

				self.inspect_root_slot(RootBlock { block, physical, regions }, 0, &first)
			}
			ScanState::RootSlot { root, slot } => self.inspect_root_slot(root, slot, &first),
			ScanState::RootVerify { root, slot, sequence } => {
				let intact = records.iter().all(|record| {
					let metadata = RootMetadata::parse(SectorRecord::new(record).spare);
					metadata.sequence == sequence && metadata.block_type == RootMetadata::BLOCK_TYPE_ROOT_DIRECTORY
				});
				if intact {
					self.root_sequence = sequence;
					self.flash = root.regions;
				}

				self.next_root_slot(root, slot)
			}
			ScanState::BlockHeader { target } => {
				if self.targets[target].check_bad_marker && SpareLayout::BigBlock.is_bad_block(first.spare) {
					self.next_target(target + 1);
					return Ok(());
				}

				let metadata = PageMetadata::parse(first.spare);
				if !matches!(metadata.page_type, PAGE_TYPE_DATA | PAGE_TYPE_COMMIT) {
					self.next_target(target + 1);
					return Ok(());
				}

				// The kernel probes the last page before bisecting, so a block written to its end
				// is found even with erased pages in between.
				let pages = self.shape.pages_per_block;
				self.state = ScanState::Search { target, first: metadata, low: 0, high: pages, probe: pages - 1 };
				Ok(())
			}
			ScanState::Search { target, first: block_first, low, high, probe } => {
				let (low, high) = match PageMetadata::parse(first.spare).page_type {
					PAGE_TYPE_DATA | PAGE_TYPE_COMMIT => (probe + 1, high),
					_ => (low, probe),
				};
				self.search(target, block_first, low, high);
				Ok(())
			}
			ScanState::Walk { target, first: block_first, low } => {
				let committed = records
					.iter()
					.all(|record| PageMetadata::parse(SectorRecord::new(record).spare).page_type == PAGE_TYPE_COMMIT);
				if !committed {
					self.walk(target, block_first, low - 1);
					return Ok(());
				}

				let page_map = self.shape.decode_page_map(records, low - 1);
				self.record_block(target, block_first, PageMetadata::parse(first.spare), page_map);
				self.next_target(target + 1);
				Ok(())
			}
			ScanState::Done | ScanState::Finished => unreachable!("no read is pending once the scan is done"),
		}
	}
}

impl BlockShape {
	/// The logical page index of physical pages `0..=last_page`, from the page map in the first
	/// 12 spare bytes of sectors 1 to 7 of the last committed page
	/// (`SfcxMuReadAsyncMetadataCompletion`, `SfcxMuReadLogicalToPhysicalIndex`).
	fn decode_page_map(&self, records: &[[u8; RAW_SECTOR_SIZE]], last_page: usize) -> Vec<u8> {
		const PAGE_MAP_BYTES_PER_SECTOR: usize = 12;

		let mut bytes = records[1..]
			.iter()
			.flat_map(|record| SectorRecord::new(record).spare[..PAGE_MAP_BYTES_PER_SECTOR].iter().copied())
			.take(self.page_map_size);
		let mask = (1u32 << self.bits_per_index) - 1;
		let mut buffer = 0u32;
		let mut available = 0u32;
		let mut map = Vec::with_capacity(last_page + 1);
		for _ in 0..=last_page {
			if available < self.bits_per_index {
				let byte = bytes.next().expect("the page map holds an index for every page of a block");
				buffer |= u32::from(byte) << available;
				available += 8;
			}
			map.push((buffer & mask) as u8);
			buffer >>= self.bits_per_index;
			available -= self.bits_per_index;
		}

		map
	}
}

impl PageMetadata {
	/// The spare-area fields of a memory unit page sector (`SFCX_MU_METADATA`). Bit fields are
	/// allocated from the most significant bit.
	fn parse(spare: &[u8; SPARE_SIZE]) -> Self {
		let logical_block = usize::from(spare[2] >> 4) << 8 | usize::from(spare[3]);
		let source_block = (usize::from(spare[5] & 0x0f) << 8 | usize::from(spare[4])) & BLOCK_NUMBER_MASK;
		let mut sequence = [0u8; 8];
		sequence[..5].copy_from_slice(&spare[7..12]);

		Self {
			logical_block: MuBlockIndex(logical_block),
			source_block: (source_block != 0).then_some(BlockIndex::new(source_block)),
			sequence: u64::from_le_bytes(sequence),
			page_type: spare[12] & PAGE_TYPE_MASK,
		}
	}
}

impl RootMetadata {
	const BLOCK_TYPE_DATA: u8 = 42;
	const BLOCK_TYPE_ROOT_DIRECTORY: u8 = 44;

	fn parse(spare: &[u8; SPARE_SIZE]) -> Self {
		Self {
			sequence: u32::from_le_bytes([spare[5], spare[3], spare[4], spare[6]]),
			block_type: spare[12] & PAGE_TYPE_MASK,
			kernel_reserved_block_count: usize::from(spare[7]),
			// Recorded in units of four blocks.
			file_system_block_count: usize::from(spare[8]) * 4,
			trailing_reserved_block_count: usize::from(spare[9]),
		}
	}

	/// The flash layout this metadata names for logical block `block`, when the kernel would
	/// accept it given the layout adopted so far.
	fn regions(&self, geometry: Geometry, block: usize, adopted: FlashRegions) -> Option<FlashRegions> {
		let logical_blocks = geometry.logical_block_count();
		let reserved =
			self.trailing_reserved_block_count + self.file_system_block_count + self.kernel_reserved_block_count;
		if reserved > logical_blocks {
			return None;
		}

		let file_system_first_block =
			logical_blocks - self.trailing_reserved_block_count - self.file_system_block_count;
		let accepted = block >= file_system_first_block
			&& self.kernel_reserved_block_count != 0
			&& self.file_system_block_count != 0
			&& file_system_first_block >= adopted.file_system_first_block;

		accepted.then_some(FlashRegions {
			kernel_reserved_block_count: self.kernel_reserved_block_count,
			file_system_first_block,
		})
	}
}

/// Adds a piece, joining it to the previous one when both are erased or physically contiguous.
fn push_piece(pieces: &mut Vec<Piece>, piece: Piece) {
	match (pieces.last_mut(), piece) {
		(Some(Piece::Erased(previous)), Piece::Erased(len)) => *previous += len,
		(Some(Piece::Raw(previous)), Piece::Raw(run)) => {
			if !previous.try_append(&run) {
				pieces.push(Piece::Raw(run));
			}
		}
		(_, piece) => pieces.push(piece),
	}
}

/// The last physical page that holds `logical_page`; later pages supersede earlier ones.
fn physical_page(page_map: &[u8], logical_page: usize) -> Option<usize> {
	page_map.iter().rposition(|entry| usize::from(*entry) == logical_page)
}

/// Size of the memory unit (`SfcxMuMountStorage`). One block in 16 past the configured flash
/// size is reserved to replace bad blocks, and a few more stay free for writes. The hypervisor's
/// sector limit is not stored in flash and is not applied.
fn memory_unit_size(geometry: Geometry, flash: FlashRegions) -> Result<u64, MemoryUnitError> {
	const LARGE_FLASH_THRESHOLD: u64 = 0x1000_0000;
	const MINIMUM_SIZE: u64 = 0xc0_0000;

	let region_blocks = flash.file_system_first_block - flash.kernel_reserved_block_count;
	let beyond_blocks = geometry.physical_block_count() - geometry.block_count();
	let reserve_blocks = beyond_blocks / 16;
	let large_flash = (geometry.physical_block_count() as u64) << geometry.block_shift() > LARGE_FLASH_THRESHOLD;
	let extra_blocks = if large_flash { 8 } else { 0 } + 4;
	let usable_blocks = region_blocks + beyond_blocks - reserve_blocks;
	let data_blocks = usable_blocks
		.checked_sub(extra_blocks + 1)
		.ok_or(MemoryUnitError::NoDataBlocks { usable_blocks: usable_blocks as u64 })?;

	let size = (data_blocks as u64) << geometry.block_shift();
	if size < MINIMUM_SIZE {
		return Err(MemoryUnitError::TooSmall { size });
	}

	Ok(size)
}

fn root_slot_sector(geometry: Geometry, block: BlockIndex, slot: usize) -> SectorIndex {
	SectorIndex::new(geometry.first_sector(block).get() + slot * ROOT_SLOT_SECTORS)
}

fn collect_warnings(collected: &Mutex<Vec<RawWarning>>, bytes: Checked<Vec<u8>>) -> Vec<u8> {
	let (bytes, warnings) = bytes.into_parts();
	if warnings.is_empty() {
		return bytes;
	}

	let mut collected = collected.lock().unwrap_or_else(PoisonError::into_inner);
	for warning in warnings {
		if let Err(position) = collected.binary_search(&warning) {
			collected.insert(position, warning);
		}
	}

	bytes
}
