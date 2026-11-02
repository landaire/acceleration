use std::ops::Range;

use fatx::FatxError;
use fatx::ReadAt;
use fatx::Volume;
use fatx::VolumeOptions;

/// Where a volume is in its source, and the partition length to mount it with. The console
/// takes the length from the device it mounts (mountvol.c FatxProcessBootSector), never from the
/// volume: a partition table's, or the file's length for a FATX file mounted as a partition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeLocation {
	range: Range<u64>,
	options: VolumeOptions,
}

impl VolumeLocation {
	/// The volume at `range`, mounted as `options` say: the partition length they give, or the
	/// range's length.
	pub fn new(range: Range<u64>, options: VolumeOptions) -> Self {
		Self { range, options }
	}

	pub fn range(&self) -> Range<u64> {
		self.range.clone()
	}

	pub const fn options(&self) -> VolumeOptions {
		self.options
	}

	pub fn image_length(&self) -> u64 {
		self.range.end - self.range.start
	}

	/// The partition length, when it is longer than the image range.
	pub fn extended_partition_length(&self) -> Option<u64> {
		self.options.partition_length().filter(|length| *length > self.image_length())
	}

	pub fn open<R>(&self, source: &R) -> Result<Volume, FatxError>
	where
		R: ReadAt,
	{
		Volume::open_with(source, self.range(), self.options)
	}
}
