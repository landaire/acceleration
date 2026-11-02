//! Selects a volume by the name of its partition in the layout of the device the image holds.

use std::ops::Range;

use fatx::PartitionKind;
use xenon_disk::ByteCount;
use xenon_disk::DetectOptions;
use xenon_disk::DeviceKind;
use xenon_disk::VolumeKind;
use xenon_disk::detect;

use crate::error::DiskToolError;
use crate::session::Session;
use crate::source::VolumeSource;

/// A partition found by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedPartition {
	pub name: &'static str,
	pub device: DeviceKind,
	/// The bytes of the partition that the image holds.
	pub range: Range<u64>,
	pub kind: PartitionKind,
	/// The partition length when the image ends before the partition does.
	pub truncated_length: Option<u64>,
}

/// The partition named `name` of the device a `image_length`-byte `source` holds. The failed
/// integrity checks of the device metadata are reported and resolved with the session policy.
pub fn select_partition<S>(
	source: &S,
	image_length: u64,
	name: &str,
	options: DetectOptions,
	session: &Session<'_>,
) -> Result<SelectedPartition, DiskToolError>
where
	S: VolumeSource,
{
	let detection = detect(source, ByteCount::new(image_length), options)?;
	session.check_source(source)?;
	let detection = session.resolve(detection)?;
	let map = detection.map();

	let Some(partition) = map.partition(name) else {
		return Err(DiskToolError::UnknownPartition {
			name: name.to_owned(),
			device: map.device(),
			available: map.partitions().iter().map(|partition| partition.name()).collect(),
		});
	};

	let full = partition.range();
	if full.start >= image_length {
		return Err(DiskToolError::PartitionPastImage { name: partition.name(), start: full.start, image_length });
	}

	let kind = match partition.volume() {
		VolumeKind::Standard => PartitionKind::Standard,
		VolumeKind::Alternate => PartitionKind::Alternate,
	};
	let truncated_length = (full.end > image_length).then_some(full.end - full.start);

	Ok(SelectedPartition {
		name: partition.name(),
		device: map.device(),
		range: full.start..full.end.min(image_length),
		kind,
		truncated_length,
	})
}
