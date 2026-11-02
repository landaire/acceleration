use std::ops::Range;

use fatx::FatxError;
use fatx::PartitionInference;
use fatx::PartitionKind;
use fatx::ReadAt;
use fatx::Volume;
use fatx::VolumeOptions;

/// Where a volume is in its source, and the partition length to mount it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeLocation {
	range: Range<u64>,
	options: VolumeOptions,
	inference: Option<PartitionInference>,
}

impl VolumeLocation {
	pub fn new(range: Range<u64>, options: VolumeOptions) -> Self {
		Self { range, options, inference: None }
	}

	/// The volume at `range` of a standard partition, mounted with the partition length that
	/// its FAT gives. A volume file shorter than its partition, such as a flash `.fdf` file,
	/// opens with the inferred partition length. When the root directory gives no evidence, the
	/// range is the partition, as a partition table gives it.
	pub fn infer<R>(source: &R, range: Range<u64>) -> Result<Self, FatxError>
	where
		R: ReadAt,
	{
		Self::infer_with(source, range, PartitionKind::Standard)
	}

	/// [`Self::infer`] for a partition of `kind`.
	pub fn infer_with<R>(source: &R, range: Range<u64>, kind: PartitionKind) -> Result<Self, FatxError>
	where
		R: ReadAt,
	{
		let image_length = range.end - range.start;
		let unextended = VolumeOptions::new().with_kind(kind);
		match Volume::infer_partition_length(source, range.clone(), kind) {
			Ok(inference) => {
				let options = if inference.partition_lengths().contains(&image_length) {
					unextended
				} else {
					inference.volume_options()
				};

				Ok(Self { range, options, inference: Some(inference) })
			}
			Err(FatxError::PartitionLengthNotInferred { .. }) => Ok(Self::new(range, unextended)),
			Err(error) => Err(error),
		}
	}

	pub fn range(&self) -> Range<u64> {
		self.range.clone()
	}

	pub const fn options(&self) -> VolumeOptions {
		self.options
	}

	pub fn inference(&self) -> Option<&PartitionInference> {
		self.inference.as_ref()
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
