#![no_std]

use core::str;

use thiserror::Error;
use winnow::Parser;
use winnow::binary::be_u16;
use winnow::binary::be_u32;
use winnow::error::ModalResult;
use winnow::token::take;

pub const IMAGE_HEADER_SIZE: usize = 16;
pub const BOOT_CHAIN_OFFSET: usize = 0x8000;
pub const BOOT_STAGE_COUNT: usize = 4;
pub const EXPECTED_STAGE_NAMES: [[u8; 2]; BOOT_STAGE_COUNT] = [*b"SB", *b"SC", *b"SD", *b"SE"];
pub const DEVKIT_60M_IMAGE_SIZE: usize = 0x03c0_0000;
pub const DEVKIT_FLASH_DATA_OFFSET: usize = 0x00b8_0000;
pub const DEVKIT_CLUSTER_SIZE: usize = 0x4000;
pub const DEVKIT_FIRST_FILE_CLUSTER: u16 = 0x0010;
pub const DEVKIT_ROOT_CLUSTER: u16 = 0x0635;
const DIRECTORY_PAGE_SIZE: usize = 0x200;
const DIRECTORY_PAGE_PAIR_SIZE: usize = 0x400;
pub const DIRECTORY_ENTRY_SIZE: usize = 0x20;
pub const DIRECTORY_NAME_SIZE: usize = 22;
const FAT_ENTRIES_PER_PAGE: usize = DIRECTORY_PAGE_SIZE / 2;
pub const FAT_END_OF_CHAIN: u16 = 0x1fff;
pub const FAT_FREE: u16 = 0x1ffe;
pub const FAT_LINK_MASK: u16 = 0x1fff;
pub const DIRECTORY_SLOT_COUNT: usize =
	(DEVKIT_CLUSTER_SIZE / DIRECTORY_PAGE_PAIR_SIZE) * (DIRECTORY_PAGE_SIZE / DIRECTORY_ENTRY_SIZE);
const EXPECTED_IMAGE_BUILD: u16 = 16547;
const EXPECTED_IMAGE_FLAGS: u32 = 0x8000_0000;
const EXPECTED_IMAGE_ENTRY: u32 = 0x0000_8000;
const EXPECTED_BOOT_IMAGE_SIZE: usize = 0x000e_0000;
const EXPECTED_STAGE_BUILDS: [u16; BOOT_STAGE_COUNT] = [14352, 16547, 16547, 16547];
const EXPECTED_STAGE_FLAGS: [u32; BOOT_STAGE_COUNT] = [0, 0x8000_0000, 0x8000_0000, 0x8000_0000];
const EXPECTED_STAGE_ENTRIES: [u32; BOOT_STAGE_COUNT] = [0x3d0, 0x120, 0x310, 0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileOffset(usize);

impl FileOffset {
	pub const fn new(value: usize) -> Self {
		Self(value)
	}

	pub const fn get(self) -> usize {
		self.0
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteCount(usize);

impl ByteCount {
	pub const fn new(value: usize) -> Self {
		Self(value)
	}

	pub const fn get(self) -> usize {
		self.0
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClusterId(u16);

impl ClusterId {
	pub const fn new(value: u16) -> Self {
		Self(value)
	}

	pub const fn get(self) -> u16 {
		self.0
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildNumber(u16);

impl BuildNumber {
	pub const fn get(self) -> u16 {
		self.0
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageHeader {
	pub magic: [u8; 2],
	pub build: BuildNumber,
	pub flags: u32,
	pub entry: u32,
	pub boot_image_size: ByteCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootStage {
	name: [u8; 2],
	build: BuildNumber,
	flags: u32,
	entry: u32,
	offset: FileOffset,
	size: ByteCount,
}

impl BootStage {
	pub fn name_str(&self) -> &str {
		str::from_utf8(&self.name).expect("boot stage names are ASCII")
	}

	pub const fn build(&self) -> BuildNumber {
		self.build
	}

	pub const fn flags(&self) -> u32 {
		self.flags
	}

	pub const fn entry(&self) -> u32 {
		self.entry
	}

	pub const fn offset(&self) -> FileOffset {
		self.offset
	}

	pub const fn size(&self) -> ByteCount {
		self.size
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectoryEntry<'a> {
	name: &'a str,
	start_cluster: ClusterId,
	size: ByteCount,
	timestamp: u32,
	record_offset: FileOffset,
}

impl DirectoryEntry<'_> {
	pub const fn name(&self) -> &str {
		self.name
	}

	pub const fn start_cluster(&self) -> ClusterId {
		self.start_cluster
	}

	pub const fn size(&self) -> ByteCount {
		self.size
	}

	pub const fn timestamp(&self) -> u32 {
		self.timestamp
	}

	pub const fn record_offset(&self) -> FileOffset {
		self.record_offset
	}
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
	#[error("image is shorter than the fixed image header")]
	HeaderTruncated,
	#[error("unsupported image magic {magic:02x?}")]
	UnsupportedMagic { magic: [u8; 2] },
	#[error("the observed devkit profile requires a 0x3c00000-byte image, got {actual:#x}")]
	UnsupportedImageSize { actual: usize },
	#[error("unsupported observed-profile value for {field}: expected {expected:#x}, got {actual:#x}")]
	UnsupportedProfileValue { field: &'static str, expected: u64, actual: u64 },
	#[error("boot stage header at {offset:#x} is truncated")]
	BootStageHeaderTruncated { offset: usize },
	#[error("expected boot stage {expected:02x?} at {offset:#x}, got {actual:02x?}")]
	UnexpectedBootStage { offset: usize, expected: [u8; 2], actual: [u8; 2] },
	#[error("boot stage {name:02x?} has an invalid size {size:#x}")]
	InvalidBootStageSize { name: [u8; 2], size: usize },
	#[error("boot stage {name:02x?} extends past the image")]
	BootStageOutOfBounds { name: [u8; 2] },
	#[error("boot chain extends past the image header's boot region")]
	BootChainOutOfBounds,
	#[error("root directory cluster is outside the image")]
	RootDirectoryOutOfBounds,
	#[error("the observed profile requires a non-empty root directory")]
	EmptyRootDirectory,
	#[error("directory entry at {offset:#x} has an invalid name")]
	InvalidDirectoryName { offset: usize },
	#[error("directory entry at {offset:#x} has zero length")]
	EmptyFile { offset: usize },
	#[error("cluster {cluster:#x} is outside the image")]
	ClusterOutOfBounds { cluster: u16 },
	#[error("file cluster {cluster:#x} is outside the observed data region")]
	FileClusterOutOfRange { cluster: u16 },
	#[error("cluster {cluster:#x} has no allocation entry")]
	AllocationEntryOutOfBounds { cluster: u16 },
	#[error("cluster chain entered free cluster {cluster:#x}")]
	FreeClusterInChain { cluster: u16 },
	#[error("cluster chain ended before the declared file size")]
	TruncatedClusterChain,
	#[error("cluster chain contains a cycle")]
	ClusterChainCycle,
	#[error("cluster chain continues from final cluster {cluster:#x} to {next:#x}")]
	ClusterChainNotTerminated { cluster: u16, next: u16 },
}

#[derive(Debug, Error)]
pub enum VisitError<E> {
	#[error(transparent)]
	Parse(#[from] ParseError),
	#[error("file consumer failed")]
	Consumer(E),
}

pub struct Image<'a> {
	bytes: &'a [u8],
	header: ImageHeader,
	boot_stages: [BootStage; BOOT_STAGE_COUNT],
}

impl<'a> Image<'a> {
	pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
		if bytes.len() != DEVKIT_60M_IMAGE_SIZE {
			return Err(ParseError::UnsupportedImageSize { actual: bytes.len() });
		}
		let header = parse_image_header(bytes)?;
		if header.magic != [0xff, 0x4f] {
			return Err(ParseError::UnsupportedMagic { magic: header.magic });
		}
		validate_profile_value("image build", EXPECTED_IMAGE_BUILD as u64, header.build.get() as u64)?;
		validate_profile_value("image flags", EXPECTED_IMAGE_FLAGS as u64, header.flags as u64)?;
		validate_profile_value("image entry", EXPECTED_IMAGE_ENTRY as u64, header.entry as u64)?;
		validate_profile_value(
			"boot image size",
			EXPECTED_BOOT_IMAGE_SIZE as u64,
			header.boot_image_size.get() as u64,
		)?;

		let mut stages = [BootStage {
			name: [0; 2],
			build: BuildNumber(0),
			flags: 0,
			entry: 0,
			offset: FileOffset(0),
			size: ByteCount(0),
		}; BOOT_STAGE_COUNT];
		let mut offset = BOOT_CHAIN_OFFSET;
		for (index, expected_name) in EXPECTED_STAGE_NAMES.iter().copied().enumerate() {
			let stage = parse_boot_stage(bytes, offset)?;
			if stage.name != expected_name {
				return Err(ParseError::UnexpectedBootStage { offset, expected: expected_name, actual: stage.name });
			}
			offset =
				offset.checked_add(stage.size.get()).ok_or(ParseError::BootStageOutOfBounds { name: stage.name })?;
			stages[index] = stage;
		}
		for (index, stage) in stages.iter().enumerate() {
			validate_profile_value("boot stage build", EXPECTED_STAGE_BUILDS[index] as u64, stage.build.get() as u64)?;
			validate_profile_value("boot stage flags", EXPECTED_STAGE_FLAGS[index] as u64, stage.flags as u64)?;
			validate_profile_value("boot stage entry", EXPECTED_STAGE_ENTRIES[index] as u64, stage.entry as u64)?;
		}

		if offset > header.boot_image_size.get() {
			return Err(ParseError::BootChainOutOfBounds);
		}
		let root_offset = cluster_offset(ClusterId::new(DEVKIT_ROOT_CLUSTER))?.get();
		if root_offset.checked_add(DEVKIT_CLUSTER_SIZE).filter(|end| *end <= bytes.len()).is_none() {
			return Err(ParseError::RootDirectoryOutOfBounds);
		}

		let image = Self { bytes, header, boot_stages: stages };
		match image.root_entries().next() {
			Some(Ok(_)) => Ok(image),
			Some(Err(error)) => Err(error),
			None => Err(ParseError::EmptyRootDirectory),
		}
	}

	pub const fn bytes(&self) -> &'a [u8] {
		self.bytes
	}

	pub const fn header(&self) -> ImageHeader {
		self.header
	}

	pub fn boot_stages(&self) -> &[BootStage; BOOT_STAGE_COUNT] {
		&self.boot_stages
	}

	pub fn boot_stage_data(&self, stage: &BootStage) -> &'a [u8] {
		let start = stage.offset.get();
		&self.bytes[start..start + stage.size.get()]
	}

	pub fn root_entries(&self) -> RootEntries<'a> {
		RootEntries {
			bytes: self.bytes,
			root_offset: cluster_offset(ClusterId::new(DEVKIT_ROOT_CLUSTER))
				.expect("validated root directory cluster")
				.get(),
			slot: 0,
		}
	}

	pub fn find_root_entry(&self, name: &str) -> Result<Option<DirectoryEntry<'a>>, ParseError> {
		for entry in self.root_entries() {
			let entry = entry?;
			if entry.name.eq_ignore_ascii_case(name) {
				return Ok(Some(entry));
			}
		}
		Ok(None)
	}

	pub fn visit_file<E>(
		&self,
		entry: DirectoryEntry<'_>,
		mut visitor: impl FnMut(&[u8]) -> Result<(), E>,
	) -> Result<(), VisitError<E>> {
		let mut cluster = entry.start_cluster;
		let mut remaining = entry.size.get();
		let maximum_steps = self.bytes.len() / DEVKIT_CLUSTER_SIZE;
		let mut steps = 0usize;

		while remaining != 0 {
			if steps >= maximum_steps {
				return Err(ParseError::ClusterChainCycle.into());
			}
			validate_file_cluster(cluster)?;
			let offset = cluster_offset(cluster)?.get();
			let available = core::cmp::min(remaining, DEVKIT_CLUSTER_SIZE);
			let end = offset
				.checked_add(available)
				.filter(|end| *end <= self.bytes.len())
				.ok_or(ParseError::ClusterOutOfBounds { cluster: cluster.get() })?;
			visitor(&self.bytes[offset..end]).map_err(VisitError::Consumer)?;
			remaining -= available;
			steps += 1;

			let next = self.allocation_entry(cluster)?;
			let link = next & FAT_LINK_MASK;
			if remaining == 0 {
				if link != FAT_END_OF_CHAIN {
					return Err(ParseError::ClusterChainNotTerminated { cluster: cluster.get(), next }.into());
				}
				break;
			}
			match link {
				FAT_END_OF_CHAIN => return Err(ParseError::TruncatedClusterChain.into()),
				FAT_FREE => {
					return Err(ParseError::FreeClusterInChain { cluster: cluster.get() }.into());
				}
				value => cluster = ClusterId::new(value),
			}
		}
		Ok(())
	}

	pub fn visit_file_clusters<E>(
		&self,
		entry: DirectoryEntry<'_>,
		mut visitor: impl FnMut(ClusterId) -> Result<(), E>,
	) -> Result<(), VisitError<E>> {
		let mut cluster = entry.start_cluster;
		let mut remaining = entry.size.get();
		let maximum_steps = self.bytes.len() / DEVKIT_CLUSTER_SIZE;
		let mut steps = 0usize;

		while remaining != 0 {
			if steps >= maximum_steps {
				return Err(ParseError::ClusterChainCycle.into());
			}
			validate_file_cluster(cluster)?;
			visitor(cluster).map_err(VisitError::Consumer)?;
			remaining = remaining.saturating_sub(DEVKIT_CLUSTER_SIZE);
			steps += 1;

			let next = self.allocation_entry(cluster)?;
			let link = next & FAT_LINK_MASK;
			if remaining == 0 {
				if link != FAT_END_OF_CHAIN {
					return Err(ParseError::ClusterChainNotTerminated { cluster: cluster.get(), next }.into());
				}
				break;
			}
			match link {
				FAT_END_OF_CHAIN => return Err(ParseError::TruncatedClusterChain.into()),
				FAT_FREE => return Err(ParseError::FreeClusterInChain { cluster: cluster.get() }.into()),
				value => cluster = ClusterId::new(value),
			}
		}
		Ok(())
	}

	pub fn allocation_entry(&self, cluster: ClusterId) -> Result<u16, ParseError> {
		let offset = allocation_entry_offset(cluster)?;
		let bytes = self
			.bytes
			.get(offset.get()..offset.get() + 2)
			.ok_or(ParseError::AllocationEntryOutOfBounds { cluster: cluster.get() })?;
		Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
	}
}

pub struct RootEntries<'a> {
	bytes: &'a [u8],
	root_offset: usize,
	slot: usize,
}

impl<'a> Iterator for RootEntries<'a> {
	type Item = Result<DirectoryEntry<'a>, ParseError>;

	fn next(&mut self) -> Option<Self::Item> {
		let slots_per_page = DIRECTORY_PAGE_SIZE / DIRECTORY_ENTRY_SIZE;
		let slot_count = DIRECTORY_SLOT_COUNT;
		while self.slot < slot_count {
			let slot = self.slot;
			self.slot += 1;
			let page_pair = slot / slots_per_page;
			let slot_in_page = slot % slots_per_page;
			let offset = self.root_offset
				+ page_pair * DIRECTORY_PAGE_PAIR_SIZE
				+ DIRECTORY_PAGE_SIZE
				+ slot_in_page * DIRECTORY_ENTRY_SIZE;
			let record = &self.bytes[offset..offset + DIRECTORY_ENTRY_SIZE];
			if record.iter().all(|byte| *byte == 0) {
				continue;
			}
			return Some(parse_directory_entry(record, offset));
		}
		None
	}
}

fn validate_profile_value(field: &'static str, expected: u64, actual: u64) -> Result<(), ParseError> {
	if actual != expected {
		return Err(ParseError::UnsupportedProfileValue { field, expected, actual });
	}
	Ok(())
}

fn parse_image_header(bytes: &[u8]) -> Result<ImageHeader, ParseError> {
	let input = bytes.get(..IMAGE_HEADER_SIZE).ok_or(ParseError::HeaderTruncated)?;
	let mut input = input;
	parse_image_header_fields(&mut input).map_err(|_| ParseError::HeaderTruncated)
}

fn parse_image_header_fields(input: &mut &[u8]) -> ModalResult<ImageHeader> {
	let magic_slice = take(2usize).parse_next(input)?;
	let magic = [magic_slice[0], magic_slice[1]];
	let build = BuildNumber(be_u16.parse_next(input)?);
	let flags = be_u32.parse_next(input)?;
	let entry = be_u32.parse_next(input)?;
	let boot_image_size = ByteCount::new(be_u32.parse_next(input)? as usize);
	Ok(ImageHeader { magic, build, flags, entry, boot_image_size })
}

fn parse_boot_stage(bytes: &[u8], offset: usize) -> Result<BootStage, ParseError> {
	let header =
		bytes.get(offset..offset + IMAGE_HEADER_SIZE).ok_or(ParseError::BootStageHeaderTruncated { offset })?;
	let mut input = header;
	let parsed = parse_image_header_fields(&mut input).map_err(|_| ParseError::BootStageHeaderTruncated { offset })?;
	let size = parsed.boot_image_size.get();
	if size < IMAGE_HEADER_SIZE {
		return Err(ParseError::InvalidBootStageSize { name: parsed.magic, size });
	}
	offset
		.checked_add(size)
		.filter(|end| *end <= bytes.len())
		.ok_or(ParseError::BootStageOutOfBounds { name: parsed.magic })?;
	Ok(BootStage {
		name: parsed.magic,
		build: parsed.build,
		flags: parsed.flags,
		entry: parsed.entry,
		offset: FileOffset::new(offset),
		size: parsed.boot_image_size,
	})
}

fn parse_directory_entry(record: &[u8], offset: usize) -> Result<DirectoryEntry<'_>, ParseError> {
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
	let start_cluster = ClusterId::new(u16::from_be_bytes([record[22], record[23]]));
	let size = ByteCount::new(u32::from_be_bytes([record[24], record[25], record[26], record[27]]) as usize);
	if size.get() == 0 {
		return Err(ParseError::EmptyFile { offset });
	}
	let timestamp = u32::from_be_bytes([record[28], record[29], record[30], record[31]]);
	Ok(DirectoryEntry { name, start_cluster, size, timestamp, record_offset: FileOffset::new(offset) })
}

fn validate_file_cluster(cluster: ClusterId) -> Result<(), ParseError> {
	if cluster.get() < DEVKIT_FIRST_FILE_CLUSTER || cluster.get() >= DEVKIT_ROOT_CLUSTER {
		return Err(ParseError::FileClusterOutOfRange { cluster: cluster.get() });
	}
	Ok(())
}

pub fn cluster_offset(cluster: ClusterId) -> Result<FileOffset, ParseError> {
	DEVKIT_FLASH_DATA_OFFSET
		.checked_add(cluster.get() as usize * DEVKIT_CLUSTER_SIZE)
		.map(FileOffset::new)
		.ok_or(ParseError::ClusterOutOfBounds { cluster: cluster.get() })
}

pub fn allocation_entry_offset(cluster: ClusterId) -> Result<FileOffset, ParseError> {
	let cluster_index = cluster.get() as usize;
	let page = cluster_index / FAT_ENTRIES_PER_PAGE;
	let index = cluster_index % FAT_ENTRIES_PER_PAGE;
	let allocation_base = cluster_offset(ClusterId::new(DEVKIT_ROOT_CLUSTER))?.get();
	allocation_base
		.checked_add(page * DIRECTORY_PAGE_PAIR_SIZE)
		.and_then(|offset| offset.checked_add(index * 2))
		.map(FileOffset::new)
		.ok_or(ParseError::AllocationEntryOutOfBounds { cluster: cluster.get() })
}

pub fn directory_slot_offset(slot: usize) -> Option<FileOffset> {
	if slot >= DIRECTORY_SLOT_COUNT {
		return None;
	}
	let slots_per_page = DIRECTORY_PAGE_SIZE / DIRECTORY_ENTRY_SIZE;
	let page_pair = slot / slots_per_page;
	let slot_in_page = slot % slots_per_page;
	let root_offset = cluster_offset(ClusterId::new(DEVKIT_ROOT_CLUSTER)).ok()?.get();
	root_offset
		.checked_add(page_pair * DIRECTORY_PAGE_PAIR_SIZE)?
		.checked_add(DIRECTORY_PAGE_SIZE)?
		.checked_add(slot_in_page * DIRECTORY_ENTRY_SIZE)
		.map(FileOffset::new)
}

#[cfg(test)]
mod tests {
	extern crate std;

	use super::*;
	use std::vec;

	#[test]
	fn rejects_images_outside_observed_profile() {
		let bytes = vec![0; 4096];
		assert_eq!(Image::parse(&bytes).err(), Some(ParseError::UnsupportedImageSize { actual: 4096 }));
	}

	#[test]
	fn directory_entry_rejects_non_ascii_name() {
		let mut record = [0u8; DIRECTORY_ENTRY_SIZE];
		record[0] = 0xff;
		assert_eq!(
			parse_directory_entry(&record, 0x100).err(),
			Some(ParseError::InvalidDirectoryName { offset: 0x100 })
		);
	}

	#[test]
	fn directory_entry_rejects_zero_length_file() {
		let mut record = [0u8; DIRECTORY_ENTRY_SIZE];
		record[..8].copy_from_slice(b"file.bin");
		assert_eq!(parse_directory_entry(&record, 0x100).err(), Some(ParseError::EmptyFile { offset: 0x100 }));
	}
}
