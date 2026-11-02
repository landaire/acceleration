//! Partition length inference for volume files.
//!
//! The driver sizes the FAT from the partition length (mountvol.c FatxProcessBootSector), and
//! the file area starts after the FAT. A volume stored in a file that grows, such as a flash
//! `.fdf` file, holds a FAT sized for a partition longer than the file, so opening the file with
//! its own length puts the file area in the wrong place.
//!
//! [`infer_partition_length`] finds the FAT length from the image. It tries each page-aligned
//! FAT length from the smallest and accepts the first for which:
//!
//! - a partition of the given [`PartitionKind`], at least as long as the image, gives a FAT of
//!   that length; an alternate partition always has a 32-bit FAT;
//! - the FAT marks the root directory cluster allocated: the end of a chain, or a link to a data
//!   cluster;
//! - the root directory's first cluster holds at least one live record before its end marker,
//!   and no record the driver would skip;
//! - every live root directory, and every live root file with data, has a first cluster that
//!   the FAT marks allocated. Only an empty file may store cluster 0.
//!
//! A FAT length smaller than the true one places the root directory inside the true FAT, whose
//! big-endian entries and zero pages seldom parse as live records. An empty root directory
//! gives no evidence, so the partition length of a volume with an empty root cannot be inferred.

use std::ops::Range;

use thiserror::Error;

use crate::entry::DIRECTORY_ENTRY_SIZE;
use crate::entry::EntryLocation;
use crate::entry::InvalidEntryReason;
use crate::entry::Slot;
use crate::entry::parse_slot;
use crate::error::FatxError;
use crate::error::GeometryError;
use crate::fat::ClusterNumber;
use crate::fat::FatEntry;
use crate::fat::FatWidth;
use crate::fat::decode_entry;
use crate::io;
use crate::io::AsyncReadAt;
use crate::io::ReadAt;
use crate::machine::Machine;
use crate::machine::Step;
use crate::machine::expect_len;
use crate::volume::Geometry;
use crate::volume::HEADER_SIZE;
use crate::volume::MAXIMUM_CLUSTERS_16;
use crate::volume::METADATA_SIZE;
use crate::volume::PartitionKind;
use crate::volume::Volume;
use crate::volume::VolumeHeader;
use crate::volume::VolumeOptions;

/// The inferred FAT, the partition lengths that give it, and the evidence for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionInference {
	kind: PartitionKind,
	fat_width: FatWidth,
	fat_length: u64,
	partition_lengths: Range<u64>,

	evidence: RootEvidence,
	rejected: Vec<RejectedCandidate>,
}

impl PartitionInference {
	pub const fn kind(&self) -> PartitionKind {
		self.kind
	}

	pub const fn fat_width(&self) -> FatWidth {
		self.fat_width
	}

	/// Bytes reserved for the FAT, a whole number of pages.
	pub const fn fat_length(&self) -> u64 {
		self.fat_length
	}

	/// Every partition length that gives this FAT and holds the image.
	pub fn partition_lengths(&self) -> Range<u64> {
		self.partition_lengths.clone()
	}

	/// The shortest length in [`Self::partition_lengths`]. Every length in the range places the
	/// image's clusters the same way; a longer one adds only clusters past the end of the image.
	pub const fn partition_length(&self) -> u64 {
		self.partition_lengths.start
	}

	/// Options that open the volume with [`Self::partition_length`].
	pub const fn volume_options(&self) -> VolumeOptions {
		VolumeOptions::new().with_kind(self.kind).with_partition_length(self.partition_length())
	}

	pub const fn evidence(&self) -> &RootEvidence {
		&self.evidence
	}

	/// The smaller FAT lengths tried first, and why each failed.
	pub fn rejected(&self) -> &[RejectedCandidate] {
		&self.rejected
	}
}

/// The root directory as the inferred FAT places it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootEvidence {
	/// The FAT entry of the root directory's first cluster.
	pub fat_entry: FatEntry,
	/// Live records in the root directory's first cluster.
	pub live_records: usize,
	/// Deleted records in the root directory's first cluster.
	pub deleted_records: usize,
}

/// A FAT length the inference tried and rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RejectedCandidate {
	pub fat_width: FatWidth,
	pub fat_length: u64,
	pub reason: CandidateRejection,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum CandidateRejection {
	#[error(transparent)]
	Geometry(#[from] GeometryError),
	#[error("the FAT marks the root directory cluster free")]
	RootClusterFree,
	#[error("the root directory cluster links to {next:#x}, which is not a data cluster")]
	RootLinkInvalid { next: u32 },
	#[error("the root directory holds no live record")]
	EmptyRoot,
	#[error("record at {location} is invalid: {reason}")]
	InvalidRecord { location: EntryLocation, reason: InvalidEntryReason },
	#[error("record at {location} starts at cluster {cluster}, which the FAT does not allocate")]
	UnallocatedFirstCluster { location: EntryLocation, cluster: ClusterNumber },
	#[error("record at {location} is a directory or a file with data, but stores cluster 0")]
	MissingFirstCluster { location: EntryLocation },
}

impl Volume {
	/// A machine that infers the partition length of the volume file stored at `range` of a
	/// source, for a partition of `kind`.
	pub fn partition_length_inferrer(range: Range<u64>, kind: PartitionKind) -> InferPartitionLength {
		InferPartitionLength {
			range,
			kind,
			prefix: Vec::new(),
			state: InferState::Reading { header: None, needed: HEADER_SIZE },
			requested: None,
		}
	}

	/// Infers the partition length of the volume file stored at `range` of a source. See
	/// [`crate::volume_file`] for the evidence it requires.
	pub fn infer_partition_length<R>(
		source: &R,
		range: Range<u64>,
		kind: PartitionKind,
	) -> Result<PartitionInference, FatxError>
	where
		R: ReadAt,
	{
		io::drive(Self::partition_length_inferrer(range, kind), source)
	}

	pub async fn infer_partition_length_async<R>(
		source: &R,
		range: Range<u64>,
		kind: PartitionKind,
	) -> Result<PartitionInference, FatxError>
	where
		R: AsyncReadAt,
	{
		io::drive_async(Self::partition_length_inferrer(range, kind), source).await
	}
}

/// Reads a prefix of a volume file until [`infer_partition_length`] can decide.
#[derive(Debug)]
pub struct InferPartitionLength {
	range: Range<u64>,
	kind: PartitionKind,
	prefix: Vec<u8>,
	state: InferState,
	requested: Option<Range<u64>>,
}

#[derive(Debug)]
enum InferState {
	/// The prefix must reach `needed` bytes.
	Reading {
		header: Option<VolumeHeader>,
		needed: u64,
	},
	Done(PartitionInference),
	Finished,
}

impl InferPartitionLength {
	fn image_length(&self) -> u64 {
		self.range.end.saturating_sub(self.range.start)
	}
}

impl Machine for InferPartitionLength {
	type Output = PartitionInference;
	type Error = FatxError;

	fn step(&mut self) -> Result<Step<Self::Output>, FatxError> {
		// Each read at least doubles the prefix, so a large FAT takes few reads.
		const FIRST_READ: u64 = 0x10000;

		assert!(self.requested.is_none(), "InferPartitionLength stepped before the requested read was fed");

		let needed = match &self.state {
			InferState::Reading { needed, .. } => *needed,
			InferState::Done(_) => {
				let InferState::Done(inference) = std::mem::replace(&mut self.state, InferState::Finished) else {
					unreachable!("matched Done");
				};
				return Ok(Step::Done(inference));
			}
			InferState::Finished => panic!("InferPartitionLength stepped after completion"),
		};

		let available = self.image_length();
		if needed > available {
			return Err(FatxError::ImageTooShort { required: needed, available });
		}

		let start = self.prefix.len() as u64;
		let end = needed.max(start * 2).max(FIRST_READ).min(available);
		let range = self.range.start + start..self.range.start + end;
		self.requested = Some(range.clone());

		Ok(Step::Read(range))
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), FatxError> {
		let range = self.requested.take().expect("InferPartitionLength fed without a requested read");
		expect_len(&range, bytes)?;
		self.prefix.extend_from_slice(bytes);

		let InferState::Reading { header, .. } = &self.state else {
			panic!("InferPartitionLength fed after completion");
		};
		let header = match header {
			Some(header) => *header,
			None => VolumeHeader::parse(self.prefix[..HEADER_SIZE as usize].try_into().expect("the header was read"))?,
		};

		self.state = match infer_from_prefix(&header, self.kind, self.image_length(), &self.prefix)? {
			Progress::Inferred(inference) => InferState::Done(inference),
			Progress::NeedsPrefix(needed) => InferState::Reading { header: Some(header), needed },
		};

		Ok(())
	}
}

/// A FAT length and the partition lengths, at least as long as the image, that give it.
#[derive(Debug, Clone)]
struct Candidate {
	fat_width: FatWidth,
	fat_length: u64,
	partition_lengths: Range<u64>,
}

enum Progress {
	Inferred(PartitionInference),
	/// The candidate under test needs the prefix to reach this many bytes.
	NeedsPrefix(u64),
}

/// Infers the partition length of the volume file `image`, with header `header`, for a
/// partition of `kind`. See [`crate::volume_file`] for the evidence it requires.
pub fn infer_partition_length(
	header: &VolumeHeader,
	kind: PartitionKind,
	image: &[u8],
) -> Result<PartitionInference, FatxError> {
	match infer_from_prefix(header, kind, image.len() as u64, image)? {
		Progress::Inferred(inference) => Ok(inference),
		Progress::NeedsPrefix(_) => unreachable!("a candidate whose root lies past the image ends the search"),
	}
}

/// The inference over the first bytes of an `image_length`-byte image.
fn infer_from_prefix(
	header: &VolumeHeader,
	kind: PartitionKind,
	image_length: u64,
	prefix: &[u8],
) -> Result<Progress, FatxError> {
	let bytes_per_cluster = header.bytes_per_cluster(kind)?;

	let mut rejected = Vec::new();
	for candidate in candidates(bytes_per_cluster, kind, image_length) {
		// The root directory cluster moves out with the FAT, so once it leaves the image no
		// larger FAT places it inside.
		let root_end =
			METADATA_SIZE + candidate.fat_length + u64::from(header.root_cluster) * u64::from(bytes_per_cluster);
		if root_end > image_length {
			return Err(FatxError::PartitionLengthNotInferred { rejected });
		}

		if root_end > prefix.len() as u64 {
			return Ok(Progress::NeedsPrefix(root_end));
		}

		let assessed = Geometry::compute(header, candidate.partition_lengths.start, kind)
			.map_err(CandidateRejection::from)
			.and_then(|geometry| {
				debug_assert_eq!(
					(geometry.fat_width(), geometry.fat_length()),
					(candidate.fat_width, candidate.fat_length)
				);
				assess_root(&geometry, prefix)
			});
		match assessed {
			Ok(evidence) => {
				return Ok(Progress::Inferred(PartitionInference {
					kind,
					fat_width: candidate.fat_width,
					fat_length: candidate.fat_length,
					partition_lengths: candidate.partition_lengths,
					evidence,
					rejected,
				}));
			}
			Err(reason) => rejected.push(RejectedCandidate {
				fat_width: candidate.fat_width,
				fat_length: candidate.fat_length,
				reason,
			}),
		}
	}

	unreachable!("the candidates grow until the root directory cluster leaves the image")
}

/// Every page-aligned FAT length, from the smallest, with the partition lengths that give it.
fn candidates(bytes_per_cluster: u32, kind: PartitionKind, image_length: u64) -> impl Iterator<Item = Candidate> {
	let cluster_shift = bytes_per_cluster.trailing_zeros();

	(1u64..).map(|pages| pages * METADATA_SIZE).flat_map(move |fat_length| {
		[FatWidth::Fat16, FatWidth::Fat32].into_iter().filter_map(move |width| {
			// Entry counts whose table rounds up to `fat_length`.
			let entry_size = width.entry_size();
			let lowest = (fat_length - METADATA_SIZE) / entry_size + 1;
			let highest = fat_length / entry_size;
			// Entry counts for which the driver picks this width (volume::fat_width).
			let (first, last) = match (kind, width) {
				(PartitionKind::Standard, FatWidth::Fat16) => (lowest, highest.min(MAXIMUM_CLUSTERS_16 - 1)),
				(PartitionKind::Standard, FatWidth::Fat32) => (lowest.max(MAXIMUM_CLUSTERS_16), highest),
				(PartitionKind::Alternate, FatWidth::Fat16) => return None,
				(PartitionKind::Alternate, FatWidth::Fat32) => (lowest, highest),
			};
			if first > last {
				return None;
			}

			// Geometry::compute counts `(partition_length >> cluster_shift) + 1` entries.
			let shortest = ((first - 1) << cluster_shift).max(image_length);
			let end = last << cluster_shift;

			(shortest < end).then_some(Candidate { fat_width: width, fat_length, partition_lengths: shortest..end })
		})
	})
}

/// The root directory evidence for `geometry`, from an image prefix that holds the FAT and the
/// root directory's first cluster.
fn assess_root(geometry: &Geometry, image: &[u8]) -> Result<RootEvidence, CandidateRejection> {
	let fat_start = geometry.fat_offset() as usize;
	let fat = &image[fat_start..fat_start + geometry.fat_length() as usize];
	let allocated = |cluster: ClusterNumber| {
		geometry.is_data_cluster(cluster)
			&& decode_entry(geometry.fat_width(), fat, cluster).is_some_and(|entry| entry != FatEntry::Free)
	};

	let root = geometry.root_cluster();
	let fat_entry = decode_entry(geometry.fat_width(), fat, root).expect("the FAT covers every data cluster");
	match fat_entry {
		FatEntry::Free => return Err(CandidateRejection::RootClusterFree),
		FatEntry::Last => {}
		FatEntry::Link(next) => {
			if !ClusterNumber::new(next).is_some_and(|next| geometry.is_data_cluster(next)) {
				return Err(CandidateRejection::RootLinkInvalid { next });
			}
		}
	}

	let start = geometry.cluster_offset(root) as usize;
	let cluster = &image[start..start + geometry.bytes_per_cluster() as usize];
	let mut live_records = 0;
	let mut deleted_records = 0;
	for (index, record) in cluster.as_chunks::<DIRECTORY_ENTRY_SIZE>().0.iter().enumerate() {
		let location = EntryLocation { cluster: root, offset: (index * DIRECTORY_ENTRY_SIZE) as u32 };
		match parse_slot(record, location) {
			Slot::End => break,
			Slot::Live(record) => {
				match record.first_cluster() {
					Some(first) if !allocated(first) => {
						return Err(CandidateRejection::UnallocatedFirstCluster { location, cluster: first });
					}
					Some(_) => {}
					None if record.is_directory() || record.size() != 0 => {
						return Err(CandidateRejection::MissingFirstCluster { location });
					}
					None => {
						// An empty file holds no clusters.
					}
				}
				live_records += 1;
			}
			Slot::Deleted(_) => deleted_records += 1,
			Slot::Invalid(reason) => return Err(CandidateRejection::InvalidRecord { location, reason }),
			Slot::UnreadableDeleted => {
				// The driver skips it too, so it says nothing about the FAT length.
			}
		}
	}

	if live_records == 0 {
		return Err(CandidateRejection::EmptyRoot);
	}

	Ok(RootEvidence { fat_entry, live_records, deleted_records })
}
