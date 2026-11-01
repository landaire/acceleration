//! The flash header and the SB, SC, SD, and SE boot stages that follow it.

use std::str;

use winnow::Parser;
use winnow::binary::be_u16;
use winnow::binary::be_u32;
use winnow::error::ModalResult;
use winnow::token::take;

use crate::error::ParseError;

pub const IMAGE_HEADER_SIZE: usize = 16;
pub const BOOT_STAGE_COUNT: usize = 4;
pub const EXPECTED_STAGE_NAMES: [[u8; 2]; BOOT_STAGE_COUNT] = [*b"SB", *b"SC", *b"SD", *b"SE"];
const FLASH_MAGIC: [u8; 2] = [0xff, 0x4f];

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

impl ImageHeader {
	/// Parses the 16-byte flash header and checks its magic.
	pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
		let mut input = bytes.get(..IMAGE_HEADER_SIZE).ok_or(ParseError::HeaderTruncated)?;
		let header = parse_header_fields(&mut input).map_err(|_| ParseError::HeaderTruncated)?;
		if header.magic != FLASH_MAGIC {
			return Err(ParseError::UnsupportedMagic { magic: header.magic });
		}

		Ok(header)
	}
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

/// The flash header and the SB, SC, SD, and SE stages of a flash image, limited to the declared
/// boot region.
#[derive(Debug, Clone, Copy)]
pub struct BootRegion<'a> {
	bytes: &'a [u8],
	header: ImageHeader,
	boot_stages: [BootStage; BOOT_STAGE_COUNT],
}

impl<'a> BootRegion<'a> {
	/// Parses the boot region of a logical image. Bytes past the declared boot region are
	/// ignored.
	pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
		let header = ImageHeader::parse(bytes)?;
		let declared = header.boot_image_size.get();
		let bytes = bytes
			.get(..declared)
			.ok_or(ParseError::BootRegionTruncated { declared: declared as u64, actual: bytes.len() as u64 })?;

		let mut stages = [BootStage {
			name: [0; 2],
			build: BuildNumber(0),
			flags: 0,
			entry: 0,
			offset: FileOffset(0),
			size: ByteCount(0),
		}; BOOT_STAGE_COUNT];
		let mut offset = header.entry as usize;
		for (index, expected_name) in EXPECTED_STAGE_NAMES.iter().copied().enumerate() {
			let stage = parse_boot_stage(bytes, offset)?;
			if stage.name != expected_name {
				return Err(ParseError::UnexpectedBootStage { offset, expected: expected_name, actual: stage.name });
			}
			offset =
				offset.checked_add(stage.size.get()).ok_or(ParseError::BootStageOutOfBounds { name: stage.name })?;
			stages[index] = stage;
		}

		Ok(Self { bytes, header, boot_stages: stages })
	}

	/// The boot region: the flash header through the declared boot image size.
	pub const fn bytes(&self) -> &'a [u8] {
		self.bytes
	}

	pub const fn header(&self) -> ImageHeader {
		self.header
	}

	/// Offset of SB, the first boot stage, as the flash header declares it.
	pub const fn boot_chain_offset(&self) -> FileOffset {
		FileOffset(self.header.entry as usize)
	}

	pub fn boot_stages(&self) -> &[BootStage; BOOT_STAGE_COUNT] {
		&self.boot_stages
	}

	pub fn boot_stage_data(&self, stage: &BootStage) -> &'a [u8] {
		let start = stage.offset.get();
		&self.bytes[start..start + stage.size.get()]
	}

	pub fn find_stage(&self, name: &str) -> Option<&BootStage> {
		self.boot_stages.iter().find(|stage| stage.name_str().eq_ignore_ascii_case(name))
	}
}

fn parse_header_fields(input: &mut &[u8]) -> ModalResult<ImageHeader> {
	let magic_slice = take(2usize).parse_next(input)?;
	let magic = [magic_slice[0], magic_slice[1]];
	let build = BuildNumber(be_u16.parse_next(input)?);
	let flags = be_u32.parse_next(input)?;
	let entry = be_u32.parse_next(input)?;
	let boot_image_size = ByteCount::new(be_u32.parse_next(input)? as usize);
	Ok(ImageHeader { magic, build, flags, entry, boot_image_size })
}

fn parse_boot_stage(bytes: &[u8], offset: usize) -> Result<BootStage, ParseError> {
	let mut input = offset
		.checked_add(IMAGE_HEADER_SIZE)
		.and_then(|end| bytes.get(offset..end))
		.ok_or(ParseError::BootStageHeaderTruncated { offset })?;
	let parsed = parse_header_fields(&mut input).map_err(|_| ParseError::BootStageHeaderTruncated { offset })?;
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
