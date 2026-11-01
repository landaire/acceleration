//! Flash image formats: the logical device image and the raw NAND image with spare areas.

use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::fmt;

use thiserror::Error;

use crate::BootRegion;
use crate::IMAGE_HEADER_SIZE;
use crate::ParseError;
use crate::parse_image_header;
use crate::raw::Geometry;
use crate::raw::RawError;
use crate::raw::RawImage;
use crate::raw::RawKind;
use crate::raw::RawWarning;

/// How to interpret an input image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum FormatSelection {
	/// Detect the format.
	#[default]
	Auto,
	/// Logical flash device image without spare areas, as XBDM `getfile FLASH:\` returns it.
	Logical,
	/// Raw NAND image with a 16-byte spare area after every 512 bytes; detect the geometry.
	Raw,
	/// Raw NAND image, 16 KiB blocks, small-block spare layout.
	RawSmallBlock,
	/// Raw NAND image, 16 KiB blocks, big-on-small-block spare layout.
	RawBigOnSmallBlock,
	/// Raw NAND image, 128 KiB blocks, big-block spare layout.
	RawBigBlock,
	/// Raw NAND image, 256 KiB blocks, big-block spare layout.
	#[cfg_attr(feature = "clap", value(name = "raw-big-block-256k"))]
	RawBigBlock256K,
}

impl FormatSelection {
	const fn raw_kind(self) -> Option<RawKind> {
		match self {
			Self::Auto | Self::Logical | Self::Raw => None,
			Self::RawSmallBlock => Some(RawKind::SmallBlock),
			Self::RawBigOnSmallBlock => Some(RawKind::BigOnSmallBlock),
			Self::RawBigBlock => Some(RawKind::BigBlock),
			Self::RawBigBlock256K => Some(RawKind::BigBlock256K),
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
	Logical,
	Raw(Geometry),
}

impl fmt::Display for ImageFormat {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Logical => formatter.write_str("logical"),
			Self::Raw(geometry) => write!(
				formatter,
				"raw {}, {} blocks of {:#x} bytes",
				geometry.kind(),
				geometry.physical_block_count(),
				geometry.block_size()
			),
		}
	}
}

/// The part of the logical device to read from a raw image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extent {
	/// The flash header through the declared boot image size.
	BootRegion,
	/// The whole logical device.
	Device,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum FormatError {
	#[error(transparent)]
	Parse(#[from] ParseError),
	#[error(transparent)]
	Raw(#[from] RawError),
	#[error("the image is neither a logical image ({logical}) nor a raw image ({raw})")]
	Unrecognized { logical: ParseError, raw: RawError },
}

/// Logical bytes of an image in either format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalImage<'a> {
	pub bytes: Cow<'a, [u8]>,
	pub format: ImageFormat,
	pub warnings: Vec<RawWarning>,
}

/// Interprets `bytes` as `selection` says and returns the logical bytes of `extent`. A logical
/// image is returned whole and borrowed.
pub fn load(bytes: &[u8], selection: FormatSelection, extent: Extent) -> Result<LogicalImage<'_>, FormatError> {
	match selection {
		FormatSelection::Logical => {
			BootRegion::parse(bytes)?;
			Ok(LogicalImage { bytes: Cow::Borrowed(bytes), format: ImageFormat::Logical, warnings: Vec::new() })
		}
		FormatSelection::Raw => load_raw(RawImage::detect(bytes)?, extent),
		FormatSelection::Auto => {
			let logical = match BootRegion::parse(bytes) {
				Ok(_) => {
					return Ok(LogicalImage {
						bytes: Cow::Borrowed(bytes),
						format: ImageFormat::Logical,
						warnings: Vec::new(),
					});
				}
				Err(error) => error,
			};
			let raw = RawImage::detect(bytes).map_err(|raw| FormatError::Unrecognized { logical, raw })?;
			load_raw(raw, extent)
		}
		FormatSelection::RawSmallBlock
		| FormatSelection::RawBigOnSmallBlock
		| FormatSelection::RawBigBlock
		| FormatSelection::RawBigBlock256K => {
			let kind = selection.raw_kind().expect("raw selection names a kind");
			let geometry = Geometry::new(kind, bytes.len())?;
			load_raw(RawImage::parse(bytes, geometry)?, extent)
		}
	}
}

fn load_raw(raw: RawImage<'_>, extent: Extent) -> Result<LogicalImage<'static>, FormatError> {
	let len = match extent {
		Extent::Device => raw.geometry().logical_size(),
		Extent::BootRegion => {
			let header = raw.read_logical(0, IMAGE_HEADER_SIZE)?;
			parse_image_header(&header.bytes)?.boot_image_size.get()
		}
	};

	let read = raw.read_logical(0, len)?;
	BootRegion::parse(&read.bytes)?;

	let mut warnings = raw.warnings().to_vec();
	warnings.extend(read.warnings);
	Ok(LogicalImage { bytes: Cow::Owned(read.bytes), format: ImageFormat::Raw(raw.geometry()), warnings })
}

#[cfg(test)]
mod tests {
	use alloc::vec;

	use super::*;
	use crate::raw::BlockIndex;
	use crate::raw::RAW_SECTOR_SIZE;
	use crate::raw::SECTOR_DATA_SIZE;
	use crate::raw::SPARE_SIZE;
	use crate::raw::SectorIndex;
	use crate::raw::sector_ecc;

	const BLOCK_SIZE: usize = 0x20000;
	const PHYSICAL_BLOCKS: usize = 512;
	const LOGICAL_BLOCKS: usize = 480;
	const SECTORS_PER_BLOCK: usize = BLOCK_SIZE / SECTOR_DATA_SIZE;
	const BAD_BLOCK: usize = 2;
	const REPLACEMENT_BLOCK: usize = PHYSICAL_BLOCKS - 1;

	/// A raw 64 MiB big-block image of `logical`, with logical block 2 marked bad and held by the
	/// last spare block, as the console's flash driver remaps it.
	#[test]
	fn raw_big_block_image_detects_and_remaps_bad_block() {
		let logical = logical_image();
		let raw = raw_big_block_image(&logical);

		let image = load(&raw, FormatSelection::Auto, Extent::Device).unwrap();
		let ImageFormat::Raw(geometry) = image.format else {
			panic!("raw image detected as {}", image.format);
		};
		assert_eq!(geometry.kind(), RawKind::BigBlock);
		assert_eq!(image.warnings, []);
		assert!(image.bytes.as_ref() == logical.as_slice());

		let boot_region = load(&raw, FormatSelection::RawBigBlock, Extent::BootRegion).unwrap();
		assert!(boot_region.bytes.as_ref() == &logical[..0x8040]);
		assert!(matches!(load(&raw, FormatSelection::Logical, Extent::Device), Err(FormatError::Parse(_))));

		let image = load(&logical, FormatSelection::Auto, Extent::Device).unwrap();
		assert_eq!(image.format, ImageFormat::Logical);
	}

	#[test]
	fn raw_sector_with_wrong_ecc_warns() {
		let logical = logical_image();
		let mut raw = raw_big_block_image(&logical);
		raw[RAW_SECTOR_SIZE + 3] ^= 0x10;

		let image = load(&raw, FormatSelection::Auto, Extent::BootRegion).unwrap();
		assert_eq!(image.warnings, [RawWarning::EccMismatch { sector: SectorIndex::new(1) }]);
	}

	#[test]
	fn bad_block_without_spare_is_an_error() {
		let logical = logical_image();
		let mut raw = raw_big_block_image(&logical);
		let replacement_spare = (REPLACEMENT_BLOCK * SECTORS_PER_BLOCK) * RAW_SECTOR_SIZE + SECTOR_DATA_SIZE;
		raw[replacement_spare] = 0;

		assert_eq!(
			load(&raw, FormatSelection::RawBigBlock, Extent::Device),
			Err(FormatError::Raw(RawError::UntranslatedBlock { block: BlockIndex::new(BAD_BLOCK) }))
		);
	}

	/// Runs against a private raw NAND dump named by `XFLASH_RAW_IMAGE`. Every sector of the
	/// logical device passes the ECC check, which the console computed, and the logical image
	/// parses as the observed devkit profile.
	#[test]
	#[ignore = "requires XFLASH_RAW_IMAGE"]
	fn raw_dump_loads_without_warnings() {
		extern crate std;

		let path = std::env::var_os("XFLASH_RAW_IMAGE").expect("XFLASH_RAW_IMAGE is set");
		let raw = std::fs::read(path).unwrap();

		let image = load(&raw, FormatSelection::Auto, Extent::Device).unwrap();
		assert!(matches!(image.format, ImageFormat::Raw(geometry) if geometry.kind() == RawKind::BigBlock));
		assert_eq!(image.warnings, []);
		crate::Image::parse(&image.bytes).unwrap();
	}

	/// A logical 60 MiB image: a boot region of four header-only stages and a marker in every
	/// block.
	fn logical_image() -> Vec<u8> {
		let mut logical = vec![0xffu8; LOGICAL_BLOCKS * BLOCK_SIZE];
		logical[..16].copy_from_slice(&header(*b"\xff\x4f", 0x8000, 0x8040));
		for (index, name) in [*b"SB", *b"SC", *b"SD", *b"SE"].into_iter().enumerate() {
			let offset = 0x8000 + index * 0x10;
			logical[offset..offset + 16].copy_from_slice(&header(name, 0, 0x10));
		}
		for block in 1..LOGICAL_BLOCKS {
			logical[block * BLOCK_SIZE..block * BLOCK_SIZE + 4].copy_from_slice(&(block as u32).to_be_bytes());
		}
		logical
	}

	fn header(magic: [u8; 2], entry: u32, size: u32) -> [u8; 16] {
		let mut header = [0u8; 16];
		header[..2].copy_from_slice(&magic);
		header[8..12].copy_from_slice(&entry.to_be_bytes());
		header[12..].copy_from_slice(&size.to_be_bytes());
		header
	}

	fn raw_big_block_image(logical: &[u8]) -> Vec<u8> {
		let mut raw = vec![0xffu8; PHYSICAL_BLOCKS * SECTORS_PER_BLOCK * RAW_SECTOR_SIZE];
		for physical in 0..PHYSICAL_BLOCKS {
			let source = match physical {
				BAD_BLOCK => None,
				REPLACEMENT_BLOCK => Some(BAD_BLOCK),
				block if block < LOGICAL_BLOCKS => Some(block),
				_ => None,
			};
			for index in 0..SECTORS_PER_BLOCK {
				let sector = physical * SECTORS_PER_BLOCK + index;
				let mut data = [0xffu8; SECTOR_DATA_SIZE];
				if let Some(block) = source {
					let offset = block * BLOCK_SIZE + index * SECTOR_DATA_SIZE;
					data.copy_from_slice(&logical[offset..offset + SECTOR_DATA_SIZE]);
				}
				let mut spare = [0xffu8; SPARE_SIZE];
				if index == 0 {
					if physical == BAD_BLOCK {
						spare[0] = 0;
					}
					let number = source.unwrap_or(physical) as u16;
					spare[1..3].copy_from_slice(&number.to_le_bytes());
				}
				if data.iter().chain(&spare).all(|byte| *byte == 0xff) {
					continue;
				}

				let ecc = sector_ecc(&data, &spare);
				let word = u32::from_le_bytes(spare[12..].try_into().unwrap()) & 0x3f | ecc << 6;
				spare[12..].copy_from_slice(&word.to_le_bytes());
				let start = sector * RAW_SECTOR_SIZE;
				raw[start..start + SECTOR_DATA_SIZE].copy_from_slice(&data);
				raw[start + SECTOR_DATA_SIZE..start + RAW_SECTOR_SIZE].copy_from_slice(&spare);
			}
		}
		raw
	}
}
