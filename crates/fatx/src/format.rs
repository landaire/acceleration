//! Volume creation. The kernel driver only mounts volumes; XAM formats them. The layout and the
//! order of writes here follow the XAM 16547 formatter, the routine that ordinal 0x42d calls at
//! 0x817ff2f0. It was found by searching the XAM disassembly for the `lis rN, 0x5854` that
//! loads the `XT` of `XTAF`, and its output matches the headers and FATs of volumes the console
//! formatted:
//!
//! 1. Zero the metadata page, so an interrupted format leaves no mountable header.
//! 2. Write the FAT: entry 0 is 0xfff8 (16-bit) or 0xfffffff8 (32-bit), entry 1 ends the root
//!    directory's chain, and every other entry is free.
//! 3. Fill the root directory's cluster with 0xff, the free record marker.
//! 4. Write the metadata page: `XTAF`, the serial number, the sectors per cluster, root cluster
//!    1, a 16-bit zero, and 0xff to the end of the page.
//!
//! Each step is made durable with an [`Effect::Flush`] before the next starts.
//!
//! XAM's default cluster size is 0x4000 bytes (the entry at 0x817ff6f0 passes 0x4000), and its
//! serial number is the low 32 bits of KeQuerySystemTime.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::ops::Range;

use crate::error::FatxError;
use crate::error::GeometryError;
use crate::fat::Fat;
use crate::fat::FatEntry;
use crate::io;
use crate::io::AsyncReadAt;
use crate::io::AsyncWriteAt;
use crate::io::ReadAt;
use crate::io::WriteAt;
use crate::machine::Effect;
use crate::machine::WriteMachine;
use crate::update::plan::FREE_RECORDS;
use crate::update::plan::PendingWrite;
use crate::update::plan::ZEROES;
use crate::volume::ALTERNATE_PARTITION_CLUSTER_SIZE;
use crate::volume::Geometry;
use crate::volume::METADATA_SIZE;
use crate::volume::PartitionKind;
use crate::volume::SECTOR_SIZE;
use crate::volume::SerialNumber;
use crate::volume::Volume;
use crate::volume::VolumeHeader;

/// XAM's cluster size when the caller names none.
pub const DEFAULT_CLUSTER_SIZE: u32 = ALTERNATE_PARTITION_CLUSTER_SIZE;

/// How to format a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatOptions {
	serial_number: SerialNumber,
	bytes_per_cluster: u32,
	kind: PartitionKind,
	partition_length: Option<u64>,
}

impl FormatOptions {
	/// A standard partition with XAM's default cluster size.
	pub const fn new(serial_number: SerialNumber) -> Self {
		Self {
			serial_number,
			bytes_per_cluster: DEFAULT_CLUSTER_SIZE,
			kind: PartitionKind::Standard,
			partition_length: None,
		}
	}

	/// The cluster size: a power of two sectors from 0x1000 to 0x10000 bytes. An alternate
	/// partition needs 0x4000.
	pub const fn with_cluster_size(self, bytes_per_cluster: u32) -> Self {
		Self { bytes_per_cluster, ..self }
	}

	pub const fn with_kind(self, kind: PartitionKind) -> Self {
		Self { kind, ..self }
	}

	/// The length of the partition the FAT is sized for, when it is longer than the image
	/// range, as for a partition the image ends inside. The range must still hold the FAT and
	/// the root directory's cluster.
	pub const fn with_partition_length(self, partition_length: u64) -> Self {
		Self { partition_length: Some(partition_length), ..self }
	}

	pub const fn partition_length(&self) -> Option<u64> {
		self.partition_length
	}

	pub const fn serial_number(&self) -> SerialNumber {
		self.serial_number
	}

	pub const fn bytes_per_cluster(&self) -> u32 {
		self.bytes_per_cluster
	}

	pub const fn kind(&self) -> PartitionKind {
		self.kind
	}

	fn header(&self) -> Result<VolumeHeader, GeometryError> {
		let bytes_per_cluster = self.bytes_per_cluster;
		if !bytes_per_cluster.is_multiple_of(SECTOR_SIZE as u32) {
			return Err(GeometryError::ClusterSizeNotSectorMultiple { bytes_per_cluster });
		}

		Ok(VolumeHeader {
			serial_number: self.serial_number,
			sectors_per_cluster: bytes_per_cluster / SECTOR_SIZE as u32,
			root_cluster: 1,
		})
	}
}

impl Volume {
	/// A machine that formats the partition at `range` of a source. The range is the partition
	/// unless the options give a partition length.
	pub fn formatter(range: Range<u64>, options: FormatOptions) -> Result<FormatVolume, FatxError> {
		FormatVolume::new(range, options)
	}

	/// Formats the partition at `range` and returns the empty volume.
	pub fn format<S>(source: &S, range: Range<u64>, options: FormatOptions) -> Result<Self, FatxError>
	where
		S: ReadAt + WriteAt,
	{
		io::drive_write(Self::formatter(range, options)?, source)
	}

	pub async fn format_async<S>(source: &S, range: Range<u64>, options: FormatOptions) -> Result<Self, FatxError>
	where
		S: AsyncReadAt + AsyncWriteAt,
	{
		io::drive_write_async(Self::formatter(range, options)?, source).await
	}
}

/// Writes a new volume in XAM's order; see the module documentation.
#[derive(Debug)]
pub struct FormatVolume {
	writes: VecDeque<PendingWrite<'static>>,
	current: Option<PendingWrite<'static>>,
	volume: Option<Volume>,
}

impl FormatVolume {
	fn new(range: Range<u64>, options: FormatOptions) -> Result<Self, FatxError> {
		let header = options.header()?;
		let image_length = range.end.saturating_sub(range.start);
		let partition_length = options.partition_length.unwrap_or(image_length);
		let geometry = Geometry::compute(&header, partition_length, options.kind)?;
		let root = geometry.root_cluster();
		let bytes_per_cluster = u64::from(geometry.bytes_per_cluster());
		let root_offset = geometry.cluster_offset(root);
		// The FAT comes before the root cluster, so this also places the whole FAT in the image.
		if root_offset + bytes_per_cluster > image_length {
			return Err(FatxError::ImageTooShort {
				required: root_offset + bytes_per_cluster,
				available: image_length,
			});
		}

		// XAM writes 0xfff8 or 0xfffffff8 to reserved entry 0, which the driver never reads.
		const MEDIA: u32 = 0xffff_fff8;

		let entries = (u64::from(geometry.cluster_count()) + 1) * geometry.fat_width().entry_size();
		let mut fat = Fat::new(geometry.fat_width(), vec![0; entries as usize]);
		fat.set_reserved(FatEntry::Link(MEDIA));
		fat.set(root, FatEntry::Last);

		let mut writes = VecDeque::new();
		let start = range.start;
		writes.push_back(PendingWrite::new(start, Cow::Borrowed(&ZEROES[..METADATA_SIZE as usize])));
		writes.push_back(PendingWrite::Flush);

		let fat_start = start + geometry.fat_offset();
		let first_page = fat.bytes().len().min(METADATA_SIZE as usize);
		let mut page = vec![0; METADATA_SIZE as usize];
		page[..first_page].copy_from_slice(&fat.bytes()[..first_page]);
		writes.push_back(PendingWrite::new(fat_start, Cow::Owned(page)));
		let mut offset = METADATA_SIZE;
		while offset < geometry.fat_length() {
			let length = (geometry.fat_length() - offset).min(ZEROES.len() as u64);
			writes.push_back(PendingWrite::new(fat_start + offset, Cow::Borrowed(&ZEROES[..length as usize])));
			offset += length;
		}
		writes.push_back(PendingWrite::Flush);

		let root_records = &FREE_RECORDS[..bytes_per_cluster as usize];
		writes.push_back(PendingWrite::new(start + root_offset, Cow::Borrowed(root_records)));
		writes.push_back(PendingWrite::Flush);
		writes.push_back(PendingWrite::new(start, Cow::Owned(header_page(&header))));
		writes.push_back(PendingWrite::Flush);

		Ok(Self { writes, current: None, volume: Some(Volume::from_parts(range, geometry, fat)) })
	}
}

impl WriteMachine for FormatVolume {
	type Output = Volume;
	type Error = FatxError;

	fn step(&mut self) -> Result<Effect<'_, Volume>, FatxError> {
		self.current = self.writes.pop_front();
		match &self.current {
			Some(write) => Ok(write.effect()),
			None => Ok(Effect::Done(self.volume.take().expect("FormatVolume stepped after completion"))),
		}
	}

	fn feed(&mut self, _bytes: &[u8]) -> Result<(), FatxError> {
		panic!("FormatVolume requests no reads");
	}
}

/// The metadata page XAM writes: the header, a 16-bit zero, then 0xff.
fn header_page(header: &VolumeHeader) -> Vec<u8> {
	let mut page = vec![0xff; METADATA_SIZE as usize];
	page[0..4].copy_from_slice(b"XTAF");
	page[4..8].copy_from_slice(&header.serial_number.get().to_be_bytes());
	page[8..12].copy_from_slice(&header.sectors_per_cluster.to_be_bytes());
	page[12..16].copy_from_slice(&header.root_cluster.to_be_bytes());
	page[16..18].fill(0);

	page
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::fat::ClusterNumber;
	use crate::io::MemorySource;

	#[test]
	fn rejects_cluster_sizes_xam_would_not_produce() {
		let options = FormatOptions::new(SerialNumber::new(1));

		assert!(matches!(
			Volume::formatter(0..0x0100_0000, options.with_cluster_size(0x1100)),
			Err(FatxError::Geometry(GeometryError::ClusterSizeNotSectorMultiple { bytes_per_cluster: 0x1100 }))
		));
		assert!(matches!(
			Volume::formatter(0..0x0100_0000, options.with_cluster_size(0x800)),
			Err(FatxError::Geometry(GeometryError::ClusterSmallerThanMetadata { .. }))
		));
		assert!(matches!(
			Volume::formatter(0..0x0100_0000, options.with_cluster_size(0x1000).with_kind(PartitionKind::Alternate)),
			Err(FatxError::Geometry(GeometryError::AlternateClusterSize { .. }))
		));
	}

	#[test]
	fn formatted_volume_reopens_empty() {
		let source = MemorySource::new(vec![0xaa; 0x0100_0000]);

		let formatted = Volume::format(&source, 0..0x0100_0000, FormatOptions::new(SerialNumber::new(0x1234))).unwrap();
		let reopened = Volume::open(&source, 0..0x0100_0000).unwrap();
		let image = source.into_inner();

		assert_eq!(formatted, reopened);
		assert_eq!(&image[..0x12], b"XTAF\0\0\x12\x34\0\0\0\x20\0\0\0\x01\0\0");
		assert!(image[0x12..0x1000].iter().all(|byte| *byte == 0xff));
		assert_eq!(&image[0x1000..0x1006], [0xff, 0xf8, 0xff, 0xff, 0, 0]);
		assert!(image[0x1006..0x2000].iter().all(|byte| *byte == 0));
		let root = reopened.geometry().cluster_offset(ClusterNumber::new(1).unwrap()) as usize;
		assert!(image[root..root + 0x4000].iter().all(|byte| *byte == 0xff));
		// Clusters past the root directory keep their old bytes.
		assert_eq!(image[root + 0x4000], 0xaa);
	}

	#[test]
	fn partition_longer_than_the_image() {
		// A 16 MiB partition with 4 KiB clusters has 0x1001 16-bit entries, a 0x3000-byte FAT,
		// and a root cluster that ends at 0x5000.
		let options =
			FormatOptions::new(SerialNumber::new(1)).with_cluster_size(0x1000).with_partition_length(0x0100_0000);
		let source = MemorySource::zeroed(0x5000);

		let formatted = Volume::format(&source, 0..0x5000, options).unwrap();
		let reopened = Volume::open_with(
			&source,
			0..0x5000,
			crate::volume::VolumeOptions::new().with_partition_length(0x0100_0000),
		)
		.unwrap();

		assert_eq!(formatted, reopened);
		assert_eq!(reopened.geometry().fat_length(), 0x3000);
		assert!(matches!(
			Volume::formatter(0..0x4fff, options),
			Err(FatxError::ImageTooShort { required: 0x5000, available: 0x4fff })
		));
	}
}
