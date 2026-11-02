//! Directory enumeration, as dirctrl.c FatxFindNextDirectoryEntry walks a directory: 64-byte
//! records through the directory's cluster chain up to the first 0x00 or 0xff name length.

use std::ops::Range;

use crate::checked::Checked;
use crate::checked::Warning;
use crate::entry::DIRECTORY_ENTRY_SIZE;
use crate::entry::EntryLocation;
use crate::entry::EntryRecord;
use crate::entry::InvalidEntryReason;
use crate::entry::Slot;
use crate::entry::parse_slot;
use crate::error::DirectoryError;
use crate::error::FatxError;
use crate::fat::ClusterNumber;
use crate::forensic::orphan::OrphanEntry;
use crate::forensic::orphan::OrphanOrigin;
use crate::forensic::orphan::assess_slots;
use crate::io;
use crate::io::AsyncReadAt;
use crate::io::ReadAt;
use crate::machine::Machine;
use crate::machine::Step;
use crate::machine::expect_len;
use crate::volume::Volume;

/// fatxp.h FATX_MAXIMUM_DIRECTORY_SIZE: the driver fails a directory with more entry bytes.
pub const MAXIMUM_DIRECTORY_SIZE: u64 = 0x40000;

/// How a directory's clusters follow each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryMode {
	/// Through the FAT, as the driver reads a live directory.
	Chain,
	/// Through physically following free clusters. A deleted directory's chain is freed, so this
	/// is a best-effort reading of its first cluster and any free clusters after it.
	Recovered,
}

/// A record the driver skips as invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidRecord {
	pub location: EntryLocation,
	pub reason: InvalidEntryReason,
}

/// The records of one directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directory {
	first_cluster: ClusterNumber,
	mode: DirectoryMode,
	clusters: Vec<ClusterNumber>,

	entries: Vec<EntryRecord>,
	deleted: Vec<EntryRecord>,
	invalid: Vec<InvalidRecord>,
	/// Records that pass the orphan checks after the end marker.
	slack: Vec<OrphanEntry>,
}

impl Directory {
	fn new(first_cluster: ClusterNumber, mode: DirectoryMode) -> Self {
		Self {
			first_cluster,
			mode,
			clusters: Vec::new(),
			entries: Vec::new(),
			deleted: Vec::new(),
			invalid: Vec::new(),
			slack: Vec::new(),
		}
	}

	pub const fn first_cluster(&self) -> ClusterNumber {
		self.first_cluster
	}

	pub const fn mode(&self) -> DirectoryMode {
		self.mode
	}

	/// The clusters read, in order.
	pub fn clusters(&self) -> &[ClusterNumber] {
		&self.clusters
	}

	/// Records with a valid name length, in directory order.
	pub fn entries(&self) -> &[EntryRecord] {
		&self.entries
	}

	/// Records marked deleted, in directory order.
	pub fn deleted(&self) -> &[EntryRecord] {
		&self.deleted
	}

	pub fn invalid(&self) -> &[InvalidRecord] {
		&self.invalid
	}

	pub fn slack(&self) -> &[OrphanEntry] {
		&self.slack
	}

	pub(crate) fn into_checked(self) -> Checked<Self> {
		let warnings = self
			.invalid
			.iter()
			.map(|record| Warning::InvalidEntry { location: record.location, reason: record.reason })
			.collect();

		Checked::new(self, warnings)
	}
}

impl Volume {
	/// A machine that reads the live directory whose chain starts at `first`.
	pub fn directory_reader(&self, first: ClusterNumber) -> ReadDirectory<'_> {
		ReadDirectory::new(self, first, DirectoryMode::Chain)
	}

	/// A machine that reads a deleted directory from its first cluster.
	pub fn recovered_directory_reader(&self, first: ClusterNumber) -> ReadDirectory<'_> {
		ReadDirectory::new(self, first, DirectoryMode::Recovered)
	}

	/// Reads a live directory. Records the driver skips are warnings.
	pub fn read_directory<R>(&self, source: &R, first: ClusterNumber) -> Result<Checked<Directory>, FatxError>
	where
		R: ReadAt,
	{
		io::drive(self.directory_reader(first), source).map(Directory::into_checked)
	}

	pub async fn read_directory_async<R>(
		&self,
		source: &R,
		first: ClusterNumber,
	) -> Result<Checked<Directory>, FatxError>
	where
		R: AsyncReadAt,
	{
		io::drive_async(self.directory_reader(first), source).await.map(Directory::into_checked)
	}
}

/// Reads one directory, a cluster at a time.
#[derive(Debug)]
pub struct ReadDirectory<'a> {
	volume: &'a Volume,
	next: Option<ClusterNumber>,
	requested: Option<Range<u64>>,
	directory: Option<Directory>,
}

impl<'a> ReadDirectory<'a> {
	pub(crate) fn new(volume: &'a Volume, first: ClusterNumber, mode: DirectoryMode) -> Self {
		Self { volume, next: Some(first), requested: None, directory: Some(Directory::new(first, mode)) }
	}

	/// The cluster after `cluster`, or `None` when the directory ends there.
	fn following(
		&self,
		directory: &Directory,
		cluster: ClusterNumber,
	) -> Result<Option<ClusterNumber>, DirectoryError> {
		let read = directory.clusters.len() as u64 * u64::from(self.volume.geometry().bytes_per_cluster());

		match directory.mode {
			DirectoryMode::Chain => {
				let next = self.volume.next_cluster(cluster)?;
				if next.is_some() && read >= MAXIMUM_DIRECTORY_SIZE {
					return Err(DirectoryError::TooLarge {
						first: directory.first_cluster,
						maximum: MAXIMUM_DIRECTORY_SIZE,
					});
				}

				Ok(next)
			}
			DirectoryMode::Recovered => Ok(cluster.following().filter(|next| {
				read < MAXIMUM_DIRECTORY_SIZE && self.volume.is_in_image(*next) && self.volume.fat().is_free(*next)
			})),
		}
	}
}

impl Machine for ReadDirectory<'_> {
	type Output = Directory;
	type Error = FatxError;

	fn step(&mut self) -> Result<Step<Self::Output>, FatxError> {
		assert!(self.requested.is_none(), "ReadDirectory stepped before the requested read was fed");
		assert!(self.directory.is_some(), "ReadDirectory stepped after completion");

		let Some(cluster) = self.next else {
			return Ok(Step::Done(self.directory.take().expect("checked above")));
		};
		let range = self.volume.cluster_range(cluster).map_err(DirectoryError::from)?;
		self.requested = Some(range.clone());

		Ok(Step::Read(range))
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), FatxError> {
		let range = self.requested.take().expect("ReadDirectory fed without a requested read");
		expect_len(&range, bytes)?;
		let cluster = self.next.expect("a cluster is requested");
		let mut directory = self.directory.take().expect("directory present while a read is requested");
		directory.clusters.push(cluster);

		let mut ended = false;
		for (index, record) in bytes.as_chunks::<DIRECTORY_ENTRY_SIZE>().0.iter().enumerate() {
			let offset = index * DIRECTORY_ENTRY_SIZE;
			let location = EntryLocation { cluster, offset: offset as u32 };
			match parse_slot(record, location) {
				Slot::End => {
					let origin = OrphanOrigin::DirectorySlack { directory: directory.first_cluster };
					directory.slack.extend(assess_slots(
						self.volume,
						cluster,
						bytes,
						offset + DIRECTORY_ENTRY_SIZE,
						origin,
					));
					ended = true;
					break;
				}
				Slot::Live(record) => directory.entries.push(record),
				Slot::Deleted(record) => directory.deleted.push(record),
				Slot::Invalid(reason) => directory.invalid.push(InvalidRecord { location, reason }),
				Slot::UnreadableDeleted => {
					// Nothing of the record is recoverable; the driver skips it too.
				}
			}
		}

		self.next = if ended { None } else { self.following(&directory, cluster)? };
		self.directory = Some(directory);

		Ok(())
	}
}
