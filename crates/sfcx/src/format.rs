use std::fmt;

use crate::raw::Geometry;
use crate::raw::RawKind;

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
	pub(crate) const fn allows_logical(self) -> bool {
		matches!(self, Self::Auto | Self::Logical)
	}

	/// The raw geometries to consider, or none for a logical-only selection.
	pub(crate) fn raw_kinds(self) -> &'static [RawKind] {
		match self {
			Self::Logical => &[],
			Self::Auto | Self::Raw => &RawKind::ALL,
			Self::RawSmallBlock => &[RawKind::SmallBlock],
			Self::RawBigOnSmallBlock => &[RawKind::BigOnSmallBlock],
			Self::RawBigBlock => &[RawKind::BigBlock],
			Self::RawBigBlock256K => &[RawKind::BigBlock256K],
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
