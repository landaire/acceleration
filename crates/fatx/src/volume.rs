//! Volume geometry, as mountvol.c FatxProcessBootSector computes it, and the opened volume.

use std::collections::HashSet;
use std::fmt;
use std::ops::Range;

use crate::error::ChainError;
use crate::error::FatxError;
use crate::error::GeometryError;
use crate::fat::ClusterNumber;
use crate::fat::ClusterRun;
use crate::fat::Fat;
use crate::fat::FatEntry;
use crate::fat::FatWidth;
use crate::io;
use crate::io::AsyncReadAt;
use crate::io::ReadAt;
use crate::machine::Machine;
use crate::machine::Step;
use crate::machine::expect_len;

/// Sector size of every Xbox 360 FATX device.
pub const SECTOR_SIZE: u64 = 0x200;
/// The metadata page: the sector size rounded up to the 4 KiB page. The FAT starts after it.
pub const METADATA_SIZE: u64 = 0x1000;
/// The largest read any machine of this crate asks for. A source never has to return more than
/// this at once, and a large FAT or file arrives in several reads.
pub const MAXIMUM_READ_SIZE: u64 = 0x40_0000;
// Directory, update and scan reads cover whole clusters, which this bound keeps within one read.
const _: () = assert!(128 * SECTOR_SIZE <= MAXIMUM_READ_SIZE);

const SIGNATURE: [u8; 4] = *b"XTAF";
pub(crate) const HEADER_SIZE: u64 = 0x10;
/// Entry count from which the FAT holds 32-bit entries.
pub(crate) const MAXIMUM_CLUSTERS_16: u64 = 0xfff0;
const MAXIMUM_NUMBER_OF_CLUSTERS: u64 = 0x0fff_ffff;
/// fatxp.h FATX_ALTERNATE_PARTITION_CLUSTER_SIZE.
pub const ALTERNATE_PARTITION_CLUSTER_SIZE: u32 = 0x4000;

/// The device a volume is mounted from, which constrains its FAT width and cluster size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PartitionKind {
	/// A disk, memory unit or flash partition, or a volume file of fixed size.
	Standard,
	/// A partition in a file that the piaf driver can grow (FILE_DEVICE_PIAF_ALTERNATE_PARTITION).
	/// mountvol.c requires 0x4000-byte clusters and always uses a 32-bit FAT for it.
	Alternate,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SerialNumber(u32);

impl SerialNumber {
	pub const fn new(value: u32) -> Self {
		Self(value)
	}

	pub const fn get(self) -> u32 {
		self.0
	}
}

impl fmt::Debug for SerialNumber {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "SerialNumber({:#010x})", self.0)
	}
}

impl fmt::Display for SerialNumber {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:08x}", self.0)
	}
}

/// FATX_VOLUME_METADATA, the first 16 bytes of a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeHeader {
	pub serial_number: SerialNumber,
	pub sectors_per_cluster: u32,
	pub root_cluster: u32,
}

impl VolumeHeader {
	pub fn parse(bytes: &[u8; HEADER_SIZE as usize]) -> Result<Self, GeometryError> {
		let field = |offset: usize| {
			u32::from_be_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
		};

		let signature = [bytes[0], bytes[1], bytes[2], bytes[3]];
		if signature != SIGNATURE {
			return Err(GeometryError::BadSignature { signature });
		}

		Ok(Self { serial_number: SerialNumber(field(4)), sectors_per_cluster: field(8), root_cluster: field(12) })
	}

	/// The cluster size, when the driver accepts the header's sectors per cluster for a
	/// partition of `kind` (mountvol.c:54-89).
	pub fn bytes_per_cluster(&self, kind: PartitionKind) -> Result<u32, GeometryError> {
		let sectors_per_cluster = self.sectors_per_cluster;
		if !sectors_per_cluster.is_power_of_two() || sectors_per_cluster > 128 {
			return Err(GeometryError::InvalidSectorsPerCluster { sectors_per_cluster });
		}

		let bytes_per_cluster = sectors_per_cluster * SECTOR_SIZE as u32;
		if kind == PartitionKind::Alternate && bytes_per_cluster != ALTERNATE_PARTITION_CLUSTER_SIZE {
			return Err(GeometryError::AlternateClusterSize {
				bytes_per_cluster,
				required: ALTERNATE_PARTITION_CLUSTER_SIZE,
			});
		}
		if u64::from(bytes_per_cluster) < METADATA_SIZE {
			return Err(GeometryError::ClusterSmallerThanMetadata { bytes_per_cluster, metadata_size: METADATA_SIZE });
		}

		Ok(bytes_per_cluster)
	}
}

/// Where the FAT and the clusters are, in bytes from the start of the volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
	serial_number: SerialNumber,
	bytes_per_cluster: u32,
	root_cluster: ClusterNumber,

	kind: PartitionKind,
	partition_length: u64,
	fat_width: FatWidth,
	fat_length: u64,
	file_area_offset: u64,
	cluster_count: u32,
}

impl Geometry {
	/// The layout the driver derives from the header and the length of a partition of `kind`.
	pub fn compute(header: &VolumeHeader, partition_length: u64, kind: PartitionKind) -> Result<Self, GeometryError> {
		let fat_offset = METADATA_SIZE;
		if fat_offset >= partition_length {
			return Err(GeometryError::PartitionTooSmall { partition_length });
		}

		let bytes_per_cluster = header.bytes_per_cluster(kind)?;

		// One FAT entry per cluster of the whole partition, plus the reserved entry.
		let cluster_shift = bytes_per_cluster.trailing_zeros();
		let entries = (partition_length >> cluster_shift) + 1;
		let fat_width = fat_width(entries, kind);
		let fat_length = (entries * fat_width.entry_size()).next_multiple_of(METADATA_SIZE);

		let available = partition_length - fat_offset;
		if fat_length >= available {
			return Err(GeometryError::FatTooLarge { fat_length, available });
		}

		let clusters = (available - fat_length) >> cluster_shift;
		if clusters > MAXIMUM_NUMBER_OF_CLUSTERS {
			return Err(GeometryError::TooManyClusters { clusters });
		}
		let cluster_count = clusters as u32;

		let root_cluster = ClusterNumber::new(header.root_cluster)
			.filter(|cluster| cluster.get() <= cluster_count)
			.ok_or(GeometryError::InvalidRootCluster { cluster: header.root_cluster })?;

		Ok(Self {
			serial_number: header.serial_number,
			bytes_per_cluster,
			root_cluster,
			kind,
			partition_length,
			fat_width,
			fat_length,
			file_area_offset: fat_offset + fat_length,
			cluster_count,
		})
	}

	pub const fn kind(&self) -> PartitionKind {
		self.kind
	}

	pub const fn serial_number(&self) -> SerialNumber {
		self.serial_number
	}

	pub const fn bytes_per_cluster(&self) -> u32 {
		self.bytes_per_cluster
	}

	pub const fn root_cluster(&self) -> ClusterNumber {
		self.root_cluster
	}

	pub const fn partition_length(&self) -> u64 {
		self.partition_length
	}

	pub const fn fat_width(&self) -> FatWidth {
		self.fat_width
	}

	pub const fn fat_offset(&self) -> u64 {
		METADATA_SIZE
	}

	/// Bytes reserved for the FAT, rounded up to whole pages.
	pub const fn fat_length(&self) -> u64 {
		self.fat_length
	}

	pub const fn file_area_offset(&self) -> u64 {
		self.file_area_offset
	}

	/// Number of data clusters, numbered from 1.
	pub const fn cluster_count(&self) -> u32 {
		self.cluster_count
	}

	pub const fn is_data_cluster(&self, cluster: ClusterNumber) -> bool {
		cluster.get() <= self.cluster_count
	}

	/// Offset of a cluster from the start of the volume.
	pub const fn cluster_offset(&self, cluster: ClusterNumber) -> u64 {
		self.file_area_offset + (cluster.get() as u64 - 1) * self.bytes_per_cluster as u64
	}
}

/// The FAT width the driver picks for `entries` FAT entries (mountvol.c:95-101).
pub(crate) fn fat_width(entries: u64, kind: PartitionKind) -> FatWidth {
	match kind {
		PartitionKind::Standard if entries < MAXIMUM_CLUSTERS_16 => FatWidth::Fat16,
		PartitionKind::Standard | PartitionKind::Alternate => FatWidth::Fat32,
	}
}

/// How to open a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeOptions {
	partition_length: Option<u64>,
	kind: PartitionKind,
}

impl VolumeOptions {
	pub const fn new() -> Self {
		Self { partition_length: None, kind: PartitionKind::Standard }
	}

	/// The partition length the console mounts the volume with, when it differs from the length
	/// of the image range: the length its device reports to FatxProcessBootSector, which sizes
	/// the FAT from it. A flash `.fdf` file, for one, is mounted through a device longer than the
	/// file.
	pub const fn with_partition_length(self, partition_length: u64) -> Self {
		Self { partition_length: Some(partition_length), ..self }
	}

	pub const fn with_kind(self, kind: PartitionKind) -> Self {
		Self { kind, ..self }
	}

	pub const fn partition_length(&self) -> Option<u64> {
		self.partition_length
	}

	pub const fn kind(&self) -> PartitionKind {
		self.kind
	}
}

impl Default for VolumeOptions {
	fn default() -> Self {
		Self::new()
	}
}

/// An opened volume: its byte range in the source, its geometry and its FAT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
	range: Range<u64>,
	geometry: Geometry,
	fat: Fat,
}

impl Volume {
	pub(crate) fn from_parts(range: Range<u64>, geometry: Geometry, fat: Fat) -> Self {
		Self { range, geometry, fat }
	}

	/// A machine that opens the volume stored at `range` of a source.
	pub fn opener(range: Range<u64>, options: VolumeOptions) -> OpenVolume {
		OpenVolume { range, options, state: OpenState::Header, requested: None }
	}

	pub fn open<R>(source: &R, range: Range<u64>) -> Result<Self, FatxError>
	where
		R: ReadAt,
	{
		io::drive(Self::opener(range, VolumeOptions::new()), source)
	}

	pub async fn open_async<R>(source: &R, range: Range<u64>) -> Result<Self, FatxError>
	where
		R: AsyncReadAt,
	{
		io::drive_async(Self::opener(range, VolumeOptions::new()), source).await
	}

	pub fn open_with<R>(source: &R, range: Range<u64>, options: VolumeOptions) -> Result<Self, FatxError>
	where
		R: ReadAt,
	{
		io::drive(Self::opener(range, options), source)
	}

	pub async fn open_with_async<R>(source: &R, range: Range<u64>, options: VolumeOptions) -> Result<Self, FatxError>
	where
		R: AsyncReadAt,
	{
		io::drive_async(Self::opener(range, options), source).await
	}

	/// The volume's bytes in the source.
	pub fn range(&self) -> Range<u64> {
		self.range.clone()
	}

	pub const fn geometry(&self) -> &Geometry {
		&self.geometry
	}

	pub const fn fat(&self) -> &Fat {
		&self.fat
	}

	pub(crate) fn fat_mut(&mut self) -> &mut Fat {
		&mut self.fat
	}

	fn image_length(&self) -> u64 {
		self.range.end - self.range.start
	}

	/// Whether the cluster is a data cluster whose bytes the image range holds.
	pub fn is_in_image(&self, cluster: ClusterNumber) -> bool {
		self.geometry.is_data_cluster(cluster)
			&& self.geometry.cluster_offset(cluster) + u64::from(self.geometry.bytes_per_cluster) <= self.image_length()
	}

	/// Number of data clusters, from cluster 1, that the image range holds.
	pub fn image_cluster_count(&self) -> u32 {
		let file_area = self.image_length().saturating_sub(self.geometry.file_area_offset);
		let clusters = file_area / u64::from(self.geometry.bytes_per_cluster);

		clusters.min(u64::from(self.geometry.cluster_count)) as u32
	}

	/// The source range of a cluster.
	pub fn cluster_range(&self, cluster: ClusterNumber) -> Result<Range<u64>, ChainError> {
		if !self.geometry.is_data_cluster(cluster) {
			return Err(ChainError::InvalidCluster { cluster });
		}

		if !self.is_in_image(cluster) {
			return Err(ChainError::OutsideImage { cluster });
		}

		let start = self.range.start + self.geometry.cluster_offset(cluster);

		Ok(start..start + u64::from(self.geometry.bytes_per_cluster))
	}

	/// The cluster after `cluster` in its chain, or `None` at the end of the chain.
	pub fn next_cluster(&self, cluster: ClusterNumber) -> Result<Option<ClusterNumber>, ChainError> {
		if !self.geometry.is_data_cluster(cluster) {
			return Err(ChainError::InvalidCluster { cluster });
		}

		let entry = self.fat.entry(cluster).expect("the FAT covers every data cluster");
		let next = match entry {
			FatEntry::Last => return Ok(None),
			FatEntry::Free => 0,
			FatEntry::Link(next) => next,
		};

		match ClusterNumber::new(next).filter(|next| self.geometry.is_data_cluster(*next)) {
			Some(next) => Ok(Some(next)),
			None => Err(ChainError::InvalidLink { cluster, next }),
		}
	}

	/// The whole chain from `first`, as runs of consecutive clusters.
	pub fn chain(&self, first: ClusterNumber) -> Result<Vec<ClusterRun>, ChainError> {
		let walk = self.walk_chain(first, u64::MAX);

		walk.error.map_or(Ok(walk.runs), Err)
	}

	/// The clusters the chain from `first` reaches before it ends or breaks, and how it broke.
	pub fn reached_chain(&self, first: ClusterNumber) -> (Vec<ClusterRun>, Option<ChainError>) {
		let walk = self.walk_chain(first, u64::MAX);

		(walk.runs, walk.error)
	}

	/// The clusters that hold `size` bytes of a file that starts at `first`. The chain may hold
	/// more clusters than the size needs.
	pub fn file_runs(&self, first: Option<ClusterNumber>, size: u64) -> Result<Vec<ClusterRun>, ChainError> {
		let needed = size.div_ceil(u64::from(self.geometry.bytes_per_cluster));
		if needed == 0 {
			return Ok(Vec::new());
		}

		let Some(first) = first else {
			return Err(ChainError::MissingFirstCluster);
		};
		let walk = self.walk_chain(first, needed);
		if let Some(error) = walk.error {
			return Err(error);
		}
		if walk.taken < needed {
			return Err(ChainError::TooShort { first, needed, available: walk.taken });
		}

		Ok(walk.runs)
	}

	/// Up to `limit` clusters of the chain from `first`, stopping at the first error.
	fn walk_chain(&self, first: ClusterNumber, limit: u64) -> ChainWalk {
		let mut walk = ChainWalk { runs: Vec::new(), taken: 0, error: None };
		if !self.geometry.is_data_cluster(first) {
			walk.error = Some(ChainError::InvalidCluster { cluster: first });
			return walk;
		}

		let mut seen = HashSet::new();
		let mut cluster = Some(first);
		while let Some(current) = cluster {
			if !seen.insert(current) {
				walk.error = Some(ChainError::Cycle { first });
				break;
			}

			push_cluster(&mut walk.runs, current);
			walk.taken += 1;
			if walk.taken == limit {
				break;
			}
			cluster = match self.next_cluster(current) {
				Ok(next) => next,
				Err(error) => {
					walk.error = Some(error);
					break;
				}
			};
		}

		walk
	}

	/// The source ranges that hold the first `size` bytes of `runs`, adjacent clusters merged.
	pub fn run_ranges(&self, runs: &[ClusterRun], size: u64) -> Result<Vec<Range<u64>>, ChainError> {
		let mut ranges: Vec<Range<u64>> = Vec::new();
		let mut remaining = size;
		for cluster in runs.iter().flat_map(ClusterRun::clusters) {
			if remaining == 0 {
				break;
			}

			let range = self.cluster_range(cluster)?;
			let length = remaining.min(range.end - range.start);
			let range = range.start..range.start + length;
			remaining -= length;
			match ranges.last_mut() {
				Some(last) if last.end == range.start => last.end = range.end,
				_ => ranges.push(range),
			}
		}

		Ok(ranges)
	}
}

/// The clusters a chain walk reached, and the error that stopped it.
#[derive(Debug)]
struct ChainWalk {
	runs: Vec<ClusterRun>,
	taken: u64,
	error: Option<ChainError>,
}

pub(crate) fn push_cluster(runs: &mut Vec<ClusterRun>, cluster: ClusterNumber) {
	match runs.last_mut() {
		Some(last) if u64::from(last.first().get()) + u64::from(last.count()) == u64::from(cluster.get()) => {
			last.extend()
		}
		_ => runs.push(ClusterRun::new(cluster, 1).expect("one cluster always fits")),
	}
}

/// Reads a volume's header and FAT. The FAT arrives in reads of at most
/// [`MAXIMUM_READ_SIZE`] bytes, appended to one buffer that becomes the volume's table.
#[derive(Debug)]
pub struct OpenVolume {
	range: Range<u64>,
	options: VolumeOptions,
	state: OpenState,
	requested: Option<Range<u64>>,
}

#[derive(Debug)]
enum OpenState {
	Header,
	Fat { geometry: Geometry, table: Vec<u8> },
	Finished,
}

impl OpenVolume {
	fn image_length(&self) -> u64 {
		self.range.end.saturating_sub(self.range.start)
	}

	fn fat_range(&self, geometry: &Geometry) -> Range<u64> {
		let length = (u64::from(geometry.cluster_count) + 1) * geometry.fat_width.entry_size();
		let start = self.range.start + geometry.fat_offset();

		start..start + length
	}

	fn require(&self, end: u64) -> Result<(), FatxError> {
		let required = end - self.range.start;
		if required > self.image_length() {
			return Err(FatxError::ImageTooShort { required, available: self.image_length() });
		}

		Ok(())
	}
}

impl Machine for OpenVolume {
	type Output = Volume;
	type Error = FatxError;

	fn step(&mut self) -> Result<Step<Self::Output>, FatxError> {
		assert!(self.requested.is_none(), "OpenVolume stepped before the requested read was fed");

		let range = match &self.state {
			OpenState::Header => self.range.start..self.range.start + HEADER_SIZE,
			OpenState::Fat { geometry, table } => {
				let whole = self.fat_range(geometry);
				let start = whole.start + table.len() as u64;
				if start == whole.end {
					let OpenState::Fat { geometry, table } = std::mem::replace(&mut self.state, OpenState::Finished)
					else {
						unreachable!("matched Fat");
					};
					let fat = Fat::new(geometry.fat_width, table);
					return Ok(Step::Done(Volume { range: self.range.clone(), geometry, fat }));
				}

				start..whole.end.min(start + MAXIMUM_READ_SIZE)
			}
			OpenState::Finished => panic!("OpenVolume stepped after completion"),
		};
		self.require(range.end)?;
		self.requested = Some(range.clone());

		Ok(Step::Read(range))
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), FatxError> {
		let range = self.requested.take().expect("OpenVolume fed without a requested read");
		expect_len(&range, bytes)?;

		match &mut self.state {
			OpenState::Header => {
				let header = VolumeHeader::parse(bytes.try_into().expect("length checked"))?;
				// Without an override the image range is the partition, as a partition table gives it.
				let partition_length = self.options.partition_length.unwrap_or(self.image_length());
				let geometry = Geometry::compute(&header, partition_length, self.options.kind)?;
				let whole = self.fat_range(&geometry);
				self.require(whole.end)?;
				let capacity = usize::try_from(whole.end - whole.start).map_err(|_| FatxError::ReadTooLarge)?;
				self.state = OpenState::Fat { geometry, table: Vec::with_capacity(capacity) };
			}
			OpenState::Fat { table, .. } => table.extend_from_slice(bytes),
			OpenState::Finished => panic!("OpenVolume fed after completion"),
		}

		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn header(sectors_per_cluster: u32) -> VolumeHeader {
		VolumeHeader { serial_number: SerialNumber(0x1234_5678), sectors_per_cluster, root_cluster: 1 }
	}

	fn standard(sectors_per_cluster: u32, partition_length: u64) -> Result<Geometry, GeometryError> {
		Geometry::compute(&header(sectors_per_cluster), partition_length, PartitionKind::Standard)
	}

	#[test]
	fn fat16_below_0xfff0_entries() {
		// 0x20 sectors per cluster, as on a memory unit's system partition.
		let geometry = standard(0x20, 0x0100_0000).unwrap();

		assert_eq!(geometry.fat_width(), FatWidth::Fat16);
		// 0x401 two-byte entries round up to one page past the metadata page.
		assert_eq!(geometry.fat_length(), 0x1000);
		assert_eq!(geometry.file_area_offset(), 0x2000);
		assert_eq!(geometry.cluster_count(), (0x0100_0000 - 0x2000) / 0x4000);
	}

	#[test]
	fn fat32_from_0xfff0_entries() {
		let geometry = standard(8, 0xfff0 * 0x1000).unwrap();

		assert_eq!(geometry.fat_width(), FatWidth::Fat32);
		assert_eq!(geometry.fat_length(), (0xfff1u64 * 4).next_multiple_of(0x1000));
	}

	#[test]
	fn alternate_partition_is_fat32_with_16_kib_clusters() {
		let geometry = Geometry::compute(&header(0x20), 0x0100_0000, PartitionKind::Alternate).unwrap();

		// 0x401 entries would be FAT16 on a standard partition.
		assert_eq!(geometry.fat_width(), FatWidth::Fat32);
		assert_eq!(geometry.fat_length(), 0x2000);
		assert_eq!(geometry.file_area_offset(), 0x3000);
		assert_eq!(geometry.kind(), PartitionKind::Alternate);
		assert_eq!(
			Geometry::compute(&header(8), 0x0100_0000, PartitionKind::Alternate),
			Err(GeometryError::AlternateClusterSize { bytes_per_cluster: 0x1000, required: 0x4000 })
		);
	}

	#[test]
	fn rejects_unmountable_headers() {
		assert_eq!(standard(3, 0x0100_0000), Err(GeometryError::InvalidSectorsPerCluster { sectors_per_cluster: 3 }));
		assert_eq!(
			standard(4, 0x0100_0000),
			Err(GeometryError::ClusterSmallerThanMetadata { bytes_per_cluster: 0x800, metadata_size: 0x1000 })
		);
		assert_eq!(standard(8, 0x1000), Err(GeometryError::PartitionTooSmall { partition_length: 0x1000 }));
		assert!(matches!(standard(8, 0x2000), Err(GeometryError::FatTooLarge { .. })));
	}
}
