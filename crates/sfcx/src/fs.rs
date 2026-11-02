//! The system flash file system: a 16 KiB root directory block that also holds the file
//! allocation table, and files stored as chains of 16 KiB data blocks (clusters).
//!
//! The root directory of a raw image is located from the block metadata, as the kernel mount
//! locates it ([`LocateRoot`]). A logical image has no block metadata, so only the observed
//! 60 MiB devkit profile, whose root directory location is known, can be read from one.

use std::iter;
use std::mem;
use std::ops::Range;
use std::str;

use crate::boot::BootChain;
use crate::boot::BootRegion;
use crate::checked::Checked;
use crate::error::ParseError;
use crate::error::SfcxError;
use crate::image::FlashImage;
use crate::image::Layout;
use crate::image::ReadLogical;
use crate::machine::Machine;
use crate::machine::Step;
use crate::raw::RawWarning;
use crate::root::DATA_BLOCK_SHIFT;
use crate::root::LocateRoot;
use crate::root::ROOT_DIRECTORY_SIZE;
use crate::root::RootScan;

pub const DIRECTORY_ENTRY_SIZE: usize = 0x20;
pub const DIRECTORY_NAME_SIZE: usize = 22;
/// Directory records before the bad block bitmap, which fills the last directory page
/// (`SFCX_DIRECTORY_ENTRIES_SIZE`).
pub const DIRECTORY_ENTRY_COUNT: usize = (DIRECTORY_SIZE - DIRECTORY_PAGE_SIZE) / DIRECTORY_ENTRY_SIZE;
pub const FAT_END_OF_CHAIN: u16 = 0x1fff;
pub const FAT_FREE: u16 = 0x1ffe;
pub const FAT_LINK_MASK: u16 = 0x1fff;
const DIRECTORY_SIZE: usize = 0x2000;
const DIRECTORY_PAGE_SIZE: usize = 0x200;
const DIRECTORY_PAGE_PAIR_SIZE: usize = 0x400;
const FAT_ENTRIES_PER_PAGE: usize = DIRECTORY_PAGE_SIZE / 2;
/// First byte of a directory record (`SFCX_DIRENT_END_OF_DIRECTORY`, `SFCX_DIRENT_DELETED`).
const DIRENT_END_OF_DIRECTORY: u8 = 0;
const DIRENT_DELETED: u8 = 5;

/// A 16 KiB data block of the file system, numbered from the file system's first block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClusterId(u16);

impl ClusterId {
	pub const fn new(value: u16) -> Self {
		Self(value)
	}

	pub const fn get(self) -> u16 {
		self.0
	}
}

/// Where the file system's data blocks and root directory are in the logical device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemLayout {
	/// Logical offset of data block 0.
	data_offset: u64,
	/// The data blocks a file may occupy.
	file_clusters: Range<ClusterId>,
	/// Logical offset of the root directory block.
	root_offset: u64,
}

impl FilesystemLayout {
	/// The layout of the observed 60 MiB devkit profile, after checking that the image matches
	/// it. A logical image of another flash cannot be read: its root directory can only be
	/// located from the block metadata of a raw image.
	pub fn observed_devkit_profile(boot_region: &BootRegion<'_>, logical_size: u64) -> Result<Self, ParseError> {
		const LOGICAL_SIZE: u64 = 0x03c0_0000;
		const DATA_OFFSET: u64 = 0x00b8_0000;
		const FIRST_FILE_CLUSTER: u16 = 0x0010;
		const ROOT_CLUSTER: u16 = 0x0635;

		let chain = boot_region.chain();
		if !matches!(chain, BootChain::Devkit { .. }) {
			return Err(ParseError::RootDirectoryNeedsRawImage { kind: chain.kind() });
		}
		if logical_size != LOGICAL_SIZE {
			return Err(ParseError::UnsupportedImageSize { actual: logical_size });
		}
		validate_profile(boot_region)?;

		Ok(Self {
			data_offset: DATA_OFFSET,
			file_clusters: ClusterId(FIRST_FILE_CLUSTER)..ClusterId(ROOT_CLUSTER),
			root_offset: DATA_OFFSET + (u64::from(ROOT_CLUSTER) << DATA_BLOCK_SHIFT),
		})
	}

	/// The layout the kernel mount scan found, or `None` when it found no root directory.
	pub fn from_root_scan(scan: &RootScan) -> Option<Self> {
		let root = scan.root()?;
		let regions = scan.regions();

		Some(Self {
			data_offset: regions.data_offset(),
			file_clusters: ClusterId(0)..ClusterId(regions.data_block_count()),
			root_offset: root.offset(),
		})
	}

	/// Logical offset of data block 0.
	pub const fn data_offset(&self) -> u64 {
		self.data_offset
	}

	/// The data blocks a file may occupy.
	pub fn file_clusters(&self) -> Range<ClusterId> {
		self.file_clusters.clone()
	}

	/// Logical offset of a data block, as `SfcxSynchronousRead` addresses a data file.
	pub const fn cluster_offset(&self, cluster: ClusterId) -> u64 {
		self.data_offset + ((cluster.0 as u64) << DATA_BLOCK_SHIFT)
	}

	/// Logical range of the root directory block.
	pub const fn root_range(&self) -> Range<u64> {
		self.root_offset..self.root_offset + ROOT_DIRECTORY_SIZE as u64
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
	name: String,
	start_cluster: ClusterId,
	size: u64,
	timestamp: u32,
	/// Logical offset of the 32-byte directory record.
	record_offset: u64,
}

impl DirectoryEntry {
	pub fn name(&self) -> &str {
		&self.name
	}

	pub const fn start_cluster(&self) -> ClusterId {
		self.start_cluster
	}

	pub const fn size(&self) -> u64 {
		self.size
	}

	pub const fn timestamp(&self) -> u32 {
		self.timestamp
	}

	pub const fn record_offset(&self) -> u64 {
		self.record_offset
	}
}

/// The root directory and allocation table of the flash file system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filesystem {
	layout: FilesystemLayout,
	entries: Vec<DirectoryEntry>,
	root: Vec<u8>,
}

impl Filesystem {
	/// Parses the root directory block, the bytes of [`FilesystemLayout::root_range`]. Even
	/// pages hold the file allocation table and odd pages the directory
	/// (`SfcxInterleaveDeinterleaveRootDirectoryBlock`). Directory records are read up to the
	/// first that starts with a zero byte, skipping deleted records.
	pub fn parse(layout: FilesystemLayout, root: Vec<u8>) -> Result<Self, ParseError> {
		if root.len() != ROOT_DIRECTORY_SIZE {
			return Err(ParseError::RootDirectorySize { expected: ROOT_DIRECTORY_SIZE, actual: root.len() });
		}

		let mut entries = Vec::new();
		for index in 0..DIRECTORY_ENTRY_COUNT {
			let offset = directory_record_offset(index);
			let record = &root[offset..offset + DIRECTORY_ENTRY_SIZE];
			match record[0] {
				DIRENT_END_OF_DIRECTORY => break,
				DIRENT_DELETED => continue,
				_ => entries.push(parse_directory_entry(record, layout.root_offset + offset as u64)?),
			}
		}

		Ok(Self { layout, entries, root })
	}

	pub const fn layout(&self) -> &FilesystemLayout {
		&self.layout
	}

	/// Logical range of the root directory block.
	pub const fn root_range(&self) -> Range<u64> {
		self.layout.root_range()
	}

	pub fn entries(&self) -> &[DirectoryEntry] {
		&self.entries
	}

	pub fn find(&self, name: &str) -> Option<&DirectoryEntry> {
		self.entries.iter().find(|entry| entry.name.eq_ignore_ascii_case(name))
	}

	pub fn allocation_entry(&self, cluster: ClusterId) -> Result<u16, ParseError> {
		let index = cluster.0 as usize;
		let offset = index / FAT_ENTRIES_PER_PAGE * DIRECTORY_PAGE_PAIR_SIZE + index % FAT_ENTRIES_PER_PAGE * 2;
		let bytes =
			self.root.get(offset..offset + 2).ok_or(ParseError::AllocationEntryOutOfBounds { cluster: cluster.0 })?;

		Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
	}

	/// The logical ranges that hold the file's data, in order, with adjacent clusters merged.
	pub fn file_ranges(&self, entry: &DirectoryEntry) -> Result<Vec<Range<u64>>, ParseError> {
		const CLUSTER_SIZE: u64 = 1 << DATA_BLOCK_SHIFT;

		let file_clusters = self.layout.file_clusters();
		let cluster_count = usize::from(file_clusters.end.0.saturating_sub(file_clusters.start.0));
		let mut ranges: Vec<Range<u64>> = Vec::new();
		let mut cluster = entry.start_cluster;
		let mut remaining = entry.size;
		let mut steps = 0usize;
		while remaining != 0 {
			if steps >= cluster_count {
				return Err(ParseError::ClusterChainCycle);
			}
			if !file_clusters.contains(&cluster) {
				return Err(ParseError::FileClusterOutOfRange { cluster: cluster.0 });
			}

			let len = remaining.min(CLUSTER_SIZE);
			let start = self.layout.cluster_offset(cluster);
			match ranges.last_mut() {
				Some(last) if last.end == start => last.end += len,
				_ => ranges.push(start..start + len),
			}
			remaining -= len;
			steps += 1;

			let next = self.allocation_entry(cluster)?;
			let link = next & FAT_LINK_MASK;
			if remaining == 0 {
				if link != FAT_END_OF_CHAIN {
					return Err(ParseError::ClusterChainNotTerminated { cluster: cluster.0, next });
				}
				break;
			}
			cluster = match link {
				FAT_END_OF_CHAIN => return Err(ParseError::TruncatedClusterChain),
				FAT_FREE => return Err(ParseError::FreeClusterInChain { cluster: cluster.0 }),
				value => ClusterId(value),
			};
		}

		Ok(ranges)
	}
}

/// Opens the file system of an image. The root directory of a raw image is located by
/// [`LocateRoot`]; a logical image must match the observed devkit profile. The root directory
/// block is then read and parsed.
#[derive(Debug)]
pub struct OpenFilesystem<'a> {
	image: &'a FlashImage,
	state: OpenFilesystemState<'a>,
	warnings: Vec<RawWarning>,
}

#[derive(Debug)]
enum OpenFilesystemState<'a> {
	Locate(LocateRoot<'a>),
	ReadRoot { layout: FilesystemLayout, read: ReadLogical },
	Finished,
}

impl<'a> OpenFilesystem<'a> {
	pub fn new(image: &'a FlashImage) -> Result<Self, SfcxError> {
		let state = match image.layout() {
			Layout::Raw(raw) => OpenFilesystemState::Locate(LocateRoot::new(raw)),
			Layout::Logical { size } => {
				let layout = FilesystemLayout::observed_devkit_profile(&image.boot_region(), *size)?;
				read_root(image, layout)?
			}
		};

		Ok(Self { image, state, warnings: Vec::new() })
	}

	fn finish(&mut self, layout: FilesystemLayout, root: Checked<Vec<u8>>) -> Result<Checked<Filesystem>, SfcxError> {
		let (root, warnings) = root.into_parts();
		let filesystem = Filesystem::parse(layout, root)?;
		// The profile only presumes where the root directory is, so an empty one means the image
		// does not match it.
		if matches!(self.image.layout(), Layout::Logical { .. }) && filesystem.entries().is_empty() {
			return Err(ParseError::EmptyRootDirectory.into());
		}

		let mut all = mem::take(&mut self.warnings);
		all.extend(warnings);
		all.sort_unstable();
		all.dedup();

		Ok(Checked::new(filesystem, all))
	}
}

impl Machine for OpenFilesystem<'_> {
	type Output = Checked<Filesystem>;
	type Error = SfcxError;

	fn step(&mut self) -> Result<Step<Self::Output>, SfcxError> {
		loop {
			match &mut self.state {
				OpenFilesystemState::Locate(locate) => {
					let scan = match locate.step()? {
						Step::Read(range) => return Ok(Step::Read(range)),
						Step::Done(scan) => scan,
					};

					let (scan, warnings) = scan.into_parts();
					self.warnings.extend(warnings);
					let Layout::Raw(raw) = self.image.layout() else {
						unreachable!("only a raw image is scanned for its root directory");
					};
					let layout = FilesystemLayout::from_root_scan(&scan)
						.ok_or(ParseError::RootDirectoryNotFound { kind: raw.geometry().kind() })?;
					self.state = read_root(self.image, layout)?;
				}
				OpenFilesystemState::ReadRoot { read, .. } => {
					let root = match read.step()? {
						Step::Read(range) => return Ok(Step::Read(range)),
						Step::Done(root) => root,
					};

					let OpenFilesystemState::ReadRoot { layout, .. } =
						mem::replace(&mut self.state, OpenFilesystemState::Finished)
					else {
						unreachable!("the root directory read just finished");
					};
					return Ok(Step::Done(self.finish(layout, root)?));
				}
				OpenFilesystemState::Finished => panic!("OpenFilesystem stepped after completion"),
			}
		}
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), SfcxError> {
		match &mut self.state {
			OpenFilesystemState::Locate(locate) => locate.feed(bytes),
			OpenFilesystemState::ReadRoot { read, .. } => read.feed(bytes),
			OpenFilesystemState::Finished => panic!("OpenFilesystem fed after completion"),
		}
	}
}

fn read_root<'a>(image: &FlashImage, layout: FilesystemLayout) -> Result<OpenFilesystemState<'a>, SfcxError> {
	let read = image.reader(iter::once(layout.root_range()))?;

	Ok(OpenFilesystemState::ReadRoot { layout, read })
}

/// Checks the boot region against the observed devkit image.
fn validate_profile(boot_region: &BootRegion<'_>) -> Result<(), ParseError> {
	const IMAGE_BUILD: u16 = 16547;
	const IMAGE_QFE: u16 = 0x8000;
	const IMAGE_FLAGS: u16 = 0;
	const IMAGE_ENTRY: u32 = 0x0000_8000;
	const BOOT_IMAGE_SIZE: usize = 0x000e_0000;
	const STAGE_BUILDS: [u16; 4] = [14352, 16547, 16547, 16547];
	const STAGE_QFES: [u16; 4] = [0, 0x8000, 0x8000, 0x8000];
	const STAGE_ENTRIES: [u32; 4] = [0x3d0, 0x120, 0x310, 0];

	let header = boot_region.header();
	validate_profile_value("image build", IMAGE_BUILD.into(), header.build.get().into())?;
	validate_profile_value("image QFE", IMAGE_QFE.into(), header.qfe.into())?;
	validate_profile_value("image flags", IMAGE_FLAGS.into(), header.flags.into())?;
	validate_profile_value("image entry", IMAGE_ENTRY.into(), header.entry.into())?;
	validate_profile_value("boot image size", BOOT_IMAGE_SIZE as u64, header.boot_image_size.get() as u64)?;
	for (index, (_, stage)) in boot_region.chain().stages().enumerate() {
		validate_profile_value("boot stage build", STAGE_BUILDS[index].into(), stage.build().get().into())?;
		validate_profile_value("boot stage QFE", STAGE_QFES[index].into(), stage.qfe().into())?;
		validate_profile_value("boot stage flags", 0, stage.flags().into())?;
		validate_profile_value("boot stage entry", STAGE_ENTRIES[index].into(), stage.entry().into())?;
	}

	Ok(())
}

fn validate_profile_value(field: &'static str, expected: u64, actual: u64) -> Result<(), ParseError> {
	if actual != expected {
		return Err(ParseError::UnsupportedProfileValue { field, expected, actual });
	}

	Ok(())
}

/// Offset in the root directory block of directory record `index`: the second page of each
/// page pair holds directory records.
const fn directory_record_offset(index: usize) -> usize {
	let records_per_page = DIRECTORY_PAGE_SIZE / DIRECTORY_ENTRY_SIZE;
	index / records_per_page * DIRECTORY_PAGE_PAIR_SIZE
		+ DIRECTORY_PAGE_SIZE
		+ index % records_per_page * DIRECTORY_ENTRY_SIZE
}

fn parse_directory_entry(record: &[u8], record_offset: u64) -> Result<DirectoryEntry, ParseError> {
	let offset = record_offset as usize;
	let name_field = &record[..DIRECTORY_NAME_SIZE];
	let name_length = name_field.iter().position(|byte| *byte == 0).unwrap_or(DIRECTORY_NAME_SIZE);
	let name_bytes = &name_field[..name_length];
	if name_bytes.is_empty()
		|| !name_field[name_length..].iter().all(|byte| *byte == 0)
		|| !name_bytes.iter().all(|byte| byte.is_ascii_graphic() && *byte != b'/' && *byte != b'\\')
	{
		return Err(ParseError::InvalidDirectoryName { offset });
	}
	let name = str::from_utf8(name_bytes).map_err(|_| ParseError::InvalidDirectoryName { offset })?;
	let start_cluster = ClusterId(u16::from_be_bytes([record[22], record[23]]));
	let size = u32::from_be_bytes([record[24], record[25], record[26], record[27]]).into();
	if size == 0 {
		return Err(ParseError::EmptyFile { offset });
	}
	let timestamp = u32::from_be_bytes([record[28], record[29], record[30], record[31]]);

	Ok(DirectoryEntry { name: name.to_owned(), start_cluster, size, timestamp, record_offset })
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn directory_entry_rejects_non_ascii_name() {
		let mut record = [0u8; DIRECTORY_ENTRY_SIZE];
		record[0] = 0xff;
		assert_eq!(parse_directory_entry(&record, 0x100), Err(ParseError::InvalidDirectoryName { offset: 0x100 }));
	}

	#[test]
	fn directory_entry_rejects_zero_length_file() {
		let mut record = [0u8; DIRECTORY_ENTRY_SIZE];
		record[..8].copy_from_slice(b"file.bin");
		assert_eq!(parse_directory_entry(&record, 0x100), Err(ParseError::EmptyFile { offset: 0x100 }));
	}
}
