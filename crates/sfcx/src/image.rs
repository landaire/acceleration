//! A flash image in either format, opened through sans-io machines.

use std::iter;
use std::mem;
use std::ops::Range;

use crate::boot::BootRegion;
use crate::boot::IMAGE_HEADER_SIZE;
use crate::boot::ImageHeader;
use crate::checked::Checked;
use crate::error::ParseError;
use crate::error::RawError;
use crate::error::SfcxError;
use crate::format::FormatSelection;
use crate::format::ImageFormat;
use crate::fs::DirectoryEntry;
use crate::fs::Filesystem;
use crate::fs::OpenFilesystem;
use crate::io;
use crate::io::AsyncReadAt;
use crate::io::ReadAt;
use crate::machine::Machine;
use crate::machine::Step;
use crate::machine::expect_len;
use crate::raw::BlockIndex;
use crate::raw::Geometry;
use crate::raw::RAW_SECTOR_SIZE;
use crate::raw::RawLayout;
use crate::raw::RawRun;
use crate::raw::RawWarning;
use crate::raw::numbered_block_count;

/// Where the logical device's bytes are in the image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layout {
	/// The image is the logical device.
	Logical { size: u64 },
	/// The image is raw NAND with spare areas and spare blocks.
	Raw(RawLayout),
}

impl Layout {
	pub fn logical_size(&self) -> u64 {
		match self {
			Self::Logical { size } => *size,
			Self::Raw(layout) => layout.geometry().logical_size(),
		}
	}
}

/// An opened flash image: its format, its layout, and its boot region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashImage {
	format: ImageFormat,
	layout: Layout,
	boot: Vec<u8>,
}

impl FlashImage {
	pub const fn format(&self) -> ImageFormat {
		self.format
	}

	pub const fn layout(&self) -> &Layout {
		&self.layout
	}

	pub fn logical_size(&self) -> u64 {
		self.layout.logical_size()
	}

	pub fn boot_region(&self) -> BootRegion<'_> {
		BootRegion::parse(&self.boot).expect("the boot region parsed when the image was opened")
	}

	/// A machine that reads logical `ranges` and returns their bytes in order.
	pub fn reader<I>(&self, ranges: I) -> Result<ReadLogical, SfcxError>
	where
		I: IntoIterator<Item = Range<u64>>,
	{
		Ok(ReadLogical { plan: Some(LogicalPlan::new(PlanLayout::from(&self.layout), ranges)?), requested: false })
	}

	/// Opens an image of `size` bytes.
	pub fn open<R>(source: &R, size: u64, selection: FormatSelection) -> Result<Checked<Self>, SfcxError>
	where
		R: ReadAt,
	{
		io::drive(OpenImage::new(size, selection), source)
	}

	pub async fn open_async<R>(source: &R, size: u64, selection: FormatSelection) -> Result<Checked<Self>, SfcxError>
	where
		R: AsyncReadAt,
	{
		io::drive_async(OpenImage::new(size, selection), source).await
	}

	pub fn read<R>(&self, source: &R, range: Range<u64>) -> Result<Checked<Vec<u8>>, SfcxError>
	where
		R: ReadAt,
	{
		io::drive(self.reader(iter::once(range))?, source)
	}

	pub async fn read_async<R>(&self, source: &R, range: Range<u64>) -> Result<Checked<Vec<u8>>, SfcxError>
	where
		R: AsyncReadAt,
	{
		io::drive_async(self.reader(iter::once(range))?, source).await
	}

	/// A machine that opens the flash file system. See [`FlashImage::filesystem`].
	pub fn filesystem_opener(&self) -> Result<OpenFilesystem<'_>, SfcxError> {
		OpenFilesystem::new(self)
	}

	/// Reads the root directory of the flash file system. The root directory of a raw image is
	/// located from the block metadata as the kernel mount locates it; a logical image must
	/// match the observed 60 MiB devkit profile.
	pub fn filesystem<R>(&self, source: &R) -> Result<Checked<Filesystem>, SfcxError>
	where
		R: ReadAt,
	{
		io::drive(self.filesystem_opener()?, source)
	}

	pub async fn filesystem_async<R>(&self, source: &R) -> Result<Checked<Filesystem>, SfcxError>
	where
		R: AsyncReadAt,
	{
		io::drive_async(self.filesystem_opener()?, source).await
	}

	pub fn read_file<R>(
		&self,
		source: &R,
		filesystem: &Filesystem,
		entry: &DirectoryEntry,
	) -> Result<Checked<Vec<u8>>, SfcxError>
	where
		R: ReadAt,
	{
		io::drive(self.reader(filesystem.file_ranges(entry)?)?, source)
	}

	pub async fn read_file_async<R>(
		&self,
		source: &R,
		filesystem: &Filesystem,
		entry: &DirectoryEntry,
	) -> Result<Checked<Vec<u8>>, SfcxError>
	where
		R: AsyncReadAt,
	{
		io::drive_async(self.reader(filesystem.file_ranges(entry)?)?, source).await
	}
}

/// Reads logical ranges of an opened image.
#[derive(Debug)]
pub struct ReadLogical {
	plan: Option<LogicalPlan>,
	requested: bool,
}

impl Machine for ReadLogical {
	type Output = Checked<Vec<u8>>;
	type Error = SfcxError;

	fn step(&mut self) -> Result<Step<Self::Output>, SfcxError> {
		assert!(!self.requested, "ReadLogical stepped before the requested read was fed");
		let plan = self.plan.as_ref().expect("ReadLogical stepped after completion");
		if let Some(range) = plan.next_range() {
			self.requested = true;
			return Ok(Step::Read(range));
		}

		let plan = self.plan.take().expect("plan present");
		Ok(Step::Done(Checked::new(plan.bytes, plan.warnings)))
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), SfcxError> {
		assert!(self.requested, "ReadLogical fed without a requested read");
		self.requested = false;
		self.plan.as_mut().expect("plan present while a read is requested").feed(bytes)
	}
}

/// Detects the format of an image and reads its boot region.
#[derive(Debug)]
pub struct OpenImage {
	size: u64,
	selection: FormatSelection,
	/// Why the image is not a logical image, once that is known.
	logical_error: Option<ParseError>,
	state: OpenState,
	requested: bool,
}

#[derive(Debug)]
enum OpenState {
	Start,
	LogicalHeader,
	LogicalBoot { declared: u64 },
	RawMetadata { candidates: Vec<Candidate>, candidate: usize },
	RawHeader { layout: RawLayout, plan: LogicalPlan },
	RawBoot { layout: RawLayout, plan: LogicalPlan },
	Done(Checked<FlashImage>),
	Finished,
}

#[derive(Debug)]
struct Candidate {
	geometry: Geometry,
	metadata: Vec<[u8; RAW_SECTOR_SIZE]>,
}

impl OpenImage {
	/// Opens an image of `size` bytes.
	pub fn new(size: u64, selection: FormatSelection) -> Self {
		Self { size, selection, logical_error: None, state: OpenState::Start, requested: false }
	}

	fn pending_range(&self) -> Option<Range<u64>> {
		match &self.state {
			OpenState::LogicalHeader => Some(0..IMAGE_HEADER_SIZE as u64),
			OpenState::LogicalBoot { declared } => Some(0..*declared),
			OpenState::RawMetadata { candidates, candidate } => {
				let current = &candidates[*candidate];
				Some(current.geometry.metadata_range(BlockIndex::new(current.metadata.len())))
			}
			OpenState::RawHeader { plan, .. } | OpenState::RawBoot { plan, .. } => plan.next_range(),
			OpenState::Start | OpenState::Done(_) | OpenState::Finished => None,
		}
	}

	/// Records why the image is not logical and moves to raw detection, or fails when only a
	/// logical image is acceptable.
	fn logical_failed(&mut self, error: ParseError) -> Result<(), SfcxError> {
		if self.selection.raw_kinds().is_empty() {
			return Err(error.into());
		}

		self.logical_error = Some(error);
		self.start_raw()
	}

	fn start_raw(&mut self) -> Result<(), SfcxError> {
		let kinds = self.selection.raw_kinds();
		let candidates: Vec<Candidate> = kinds
			.iter()
			.filter_map(|kind| Geometry::new(*kind, self.size).ok())
			.map(|geometry| Candidate { geometry, metadata: Vec::with_capacity(geometry.block_count()) })
			.collect();
		if candidates.is_empty() {
			let error = match kinds {
				[kind] => Geometry::new(*kind, self.size).expect_err("no candidate geometry"),
				_ if !self.size.is_multiple_of(RAW_SECTOR_SIZE as u64) => RawError::PartialSector { size: self.size },
				_ => RawError::UnknownGeometry,
			};
			return Err(self.raw_failed(error));
		}

		self.state = OpenState::RawMetadata { candidates, candidate: 0 };
		Ok(())
	}

	fn raw_failed(&self, raw: RawError) -> SfcxError {
		match self.logical_error {
			Some(logical) => SfcxError::Unrecognized { logical, raw },
			None => raw.into(),
		}
	}

	/// Picks the geometry once every candidate's metadata is read. An explicitly selected
	/// geometry is used as is; otherwise the block-number fields must favor one geometry.
	fn select_geometry(&self, candidates: Vec<Candidate>) -> Result<Candidate, SfcxError> {
		if self.selection.raw_kinds().len() == 1 {
			return Ok(candidates.into_iter().next().expect("one candidate"));
		}

		let mut best: Option<(Candidate, usize)> = None;
		let mut tied = None;
		for candidate in candidates {
			let logical_blocks = candidate.geometry.logical_block_count();
			let score = numbered_block_count(candidate.geometry, &candidate.metadata[..logical_blocks]);
			if score == 0 {
				continue;
			}

			match &best {
				Some((_, best_score)) if score < *best_score => {}
				Some((_, best_score)) if score == *best_score => tied = Some(candidate.geometry.kind()),
				_ => {
					best = Some((candidate, score));
					tied = None;
				}
			}
		}

		let Some((candidate, _)) = best else {
			return Err(self.raw_failed(RawError::UnknownGeometry));
		};
		if let Some(first) = tied {
			return Err(self.raw_failed(RawError::AmbiguousGeometry { first, second: candidate.geometry.kind() }));
		}

		Ok(candidate)
	}

	/// Advances past a state that needs no read.
	fn advance(&mut self, state: OpenState) -> Result<(), SfcxError> {
		match state {
			OpenState::Start if !self.selection.allows_logical() => self.start_raw(),
			OpenState::Start if self.size < IMAGE_HEADER_SIZE as u64 => {
				self.logical_failed(ParseError::HeaderTruncated)
			}
			OpenState::Start => {
				self.state = OpenState::LogicalHeader;
				Ok(())
			}
			OpenState::RawHeader { layout, plan } => {
				let header = ImageHeader::parse(&plan.bytes)?;
				let size =
					header.declared_boot_size().map_or(layout.geometry().logical_size(), |size| size.get() as u64);
				let boot = 0..size;
				let plan = LogicalPlan::new(PlanLayout::Raw(&layout), iter::once(boot))?;
				self.state = OpenState::RawBoot { layout, plan };
				Ok(())
			}
			OpenState::RawBoot { layout, plan } => {
				BootRegion::parse(&plan.bytes)?;
				// The boot region read covers metadata sectors that the layout already reported.
				let mut warnings = layout.warnings().to_vec();
				warnings.extend(plan.warnings.into_iter().filter(|warning| !layout.warnings().contains(warning)));
				warnings.sort_unstable();
				let format = ImageFormat::Raw(layout.geometry());
				let image = FlashImage { format, layout: Layout::Raw(layout), boot: plan.bytes };
				self.state = OpenState::Done(Checked::new(image, warnings));
				Ok(())
			}
			OpenState::LogicalHeader
			| OpenState::LogicalBoot { .. }
			| OpenState::RawMetadata { .. }
			| OpenState::Done(_)
			| OpenState::Finished => unreachable!("only states without a pending read advance"),
		}
	}
}

impl Machine for OpenImage {
	type Output = Checked<FlashImage>;
	type Error = SfcxError;

	fn step(&mut self) -> Result<Step<Self::Output>, SfcxError> {
		assert!(!self.requested, "OpenImage stepped before the requested read was fed");
		loop {
			if let Some(range) = self.pending_range() {
				self.requested = true;
				return Ok(Step::Read(range));
			}

			match mem::replace(&mut self.state, OpenState::Finished) {
				OpenState::Done(image) => return Ok(Step::Done(image)),
				OpenState::Finished => panic!("OpenImage stepped after completion"),
				state => self.advance(state)?,
			}
		}
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), SfcxError> {
		assert!(self.requested, "OpenImage fed without a requested read");
		self.requested = false;
		let range = self.pending_range().expect("a requested read is pending");
		match mem::replace(&mut self.state, OpenState::Finished) {
			OpenState::LogicalHeader => {
				expect_len(&range, bytes)?;
				match ImageHeader::parse(bytes) {
					Ok(header) => {
						let declared = header.declared_boot_size().map_or(self.size, |size| size.get() as u64);
						if declared > self.size {
							return self
								.logical_failed(ParseError::BootRegionTruncated { declared, actual: self.size });
						}
						self.state = OpenState::LogicalBoot { declared };
						Ok(())
					}
					Err(error) => self.logical_failed(error),
				}
			}
			OpenState::LogicalBoot { .. } => {
				expect_len(&range, bytes)?;
				if let Err(error) = BootRegion::parse(bytes) {
					return self.logical_failed(error);
				}

				let image = FlashImage {
					format: ImageFormat::Logical,
					layout: Layout::Logical { size: self.size },
					boot: bytes.to_vec(),
				};
				self.state = OpenState::Done(Checked::new(image, Vec::new()));
				Ok(())
			}
			OpenState::RawMetadata { mut candidates, candidate } => {
				expect_len(&range, bytes)?;
				let current = &mut candidates[candidate];
				current.metadata.push(bytes.try_into().expect("one record"));
				if current.metadata.len() < current.geometry.block_count() {
					self.state = OpenState::RawMetadata { candidates, candidate };
					return Ok(());
				}
				if candidate + 1 < candidates.len() {
					self.state = OpenState::RawMetadata { candidates, candidate: candidate + 1 };
					return Ok(());
				}

				let chosen = self.select_geometry(candidates)?;
				let layout = RawLayout::new(chosen.geometry, &chosen.metadata);
				let plan = LogicalPlan::new(PlanLayout::Raw(&layout), iter::once(0..IMAGE_HEADER_SIZE as u64))?;
				self.state = OpenState::RawHeader { layout, plan };
				Ok(())
			}
			OpenState::RawHeader { layout, mut plan } => {
				plan.feed(bytes)?;
				self.state = OpenState::RawHeader { layout, plan };
				Ok(())
			}
			OpenState::RawBoot { layout, mut plan } => {
				plan.feed(bytes)?;
				self.state = OpenState::RawBoot { layout, plan };
				Ok(())
			}
			OpenState::Start | OpenState::Done(_) | OpenState::Finished => unreachable!("no pending read"),
		}
	}
}

/// The reads that cover logical ranges and the bytes collected from them.
#[derive(Debug)]
struct LogicalPlan {
	runs: Vec<Run>,
	next: usize,
	bytes: Vec<u8>,
	warnings: Vec<RawWarning>,
}

#[derive(Debug)]
enum Run {
	Logical(Range<u64>),
	Raw(RawRun),
}

/// The layout a plan translates through, borrowed from either an opened image or an opener.
#[derive(Debug, Clone, Copy)]
enum PlanLayout<'a> {
	Logical { size: u64 },
	Raw(&'a RawLayout),
}

impl<'a> From<&'a Layout> for PlanLayout<'a> {
	fn from(layout: &'a Layout) -> Self {
		match layout {
			Layout::Logical { size } => Self::Logical { size: *size },
			Layout::Raw(raw) => Self::Raw(raw),
		}
	}
}

impl LogicalPlan {
	fn new<I>(layout: PlanLayout<'_>, ranges: I) -> Result<Self, SfcxError>
	where
		I: IntoIterator<Item = Range<u64>>,
	{
		let size = match layout {
			PlanLayout::Logical { size } => size,
			PlanLayout::Raw(raw) => raw.geometry().logical_size(),
		};
		let mut runs = Vec::new();
		let mut total = 0u64;
		for range in ranges {
			if range.start > range.end || range.end > size {
				return Err(SfcxError::LogicalRangeOutOfBounds { range, size });
			}
			total = total
				.checked_add(range.end - range.start)
				.filter(|total| usize::try_from(*total).is_ok_and(|total| total <= isize::MAX as usize))
				.ok_or(SfcxError::ReadTooLarge)?;
			match layout {
				PlanLayout::Logical { .. } => runs.push(Run::Logical(range)),
				PlanLayout::Raw(raw) => runs.extend(raw.runs(range)?.into_iter().map(Run::Raw)),
			}
		}

		Ok(Self { runs, next: 0, bytes: Vec::with_capacity(total as usize), warnings: Vec::new() })
	}

	fn next_range(&self) -> Option<Range<u64>> {
		self.runs.get(self.next).map(|run| match run {
			Run::Logical(range) => range.clone(),
			Run::Raw(run) => run.raw.clone(),
		})
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), SfcxError> {
		let range = self.next_range().expect("LogicalPlan fed without a pending read");
		expect_len(&range, bytes)?;
		match &self.runs[self.next] {
			Run::Logical(_) => self.bytes.extend_from_slice(bytes),
			Run::Raw(run) => run.strip(bytes, &mut self.bytes, &mut self.warnings),
		}
		self.next += 1;

		Ok(())
	}
}
