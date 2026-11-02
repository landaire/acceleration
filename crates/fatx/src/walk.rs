//! Recursive listing of a volume: live entries, deleted entries, and the contents of deleted
//! directories whose first cluster is still free.

use std::collections::HashSet;

use crate::checked::Checked;
use crate::checked::Warning;
use crate::directory::Directory;
use crate::directory::DirectoryMode;
use crate::directory::ReadDirectory;
use crate::entry::EntryRecord;
use crate::entry::InvalidEntryReason;
use crate::entry::MAXIMUM_FILE_NAME_LENGTH;
use crate::error::ChainError;
use crate::error::DirectoryError;
use crate::error::FatxError;
use crate::fat::ClusterNumber;
use crate::fat::ClusterRun;
use crate::forensic::orphan::OrphanEntry;
use crate::io;
use crate::io::AsyncReadAt;
use crate::io::ReadAt;
use crate::machine::Machine;
use crate::machine::Step;
use crate::volume::Volume;

const ROOT_PATH: &str = "/";
/// fatxp.h FATX_MAXIMUM_PATH_NAME_LENGTH.
const MAXIMUM_PATH_LENGTH: usize = 250;

/// A live file or directory and its absolute path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedEntry {
	path: String,
	record: EntryRecord,
}

impl ListedEntry {
	pub fn path(&self) -> &str {
		&self.path
	}

	pub const fn record(&self) -> &EntryRecord {
		&self.record
	}
}

/// Where a deleted entry was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletedOrigin {
	/// A record marked deleted in a live directory.
	LiveDirectory,
	/// Any record in a deleted directory's recovered clusters.
	DeletedDirectory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedEntry {
	parent: String,
	record: EntryRecord,
	origin: DeletedOrigin,
}

impl DeletedEntry {
	/// Path of the directory that holds the record.
	pub fn parent(&self) -> &str {
		&self.parent
	}

	pub fn path(&self) -> String {
		join(&self.parent, self.record.name())
	}

	pub const fn record(&self) -> &EntryRecord {
		&self.record
	}

	pub const fn origin(&self) -> DeletedOrigin {
		self.origin
	}
}

/// A record after the end marker of a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackEntry {
	directory: String,
	orphan: OrphanEntry,
}

impl SlackEntry {
	pub fn directory(&self) -> &str {
		&self.directory
	}

	pub const fn orphan(&self) -> &OrphanEntry {
		&self.orphan
	}
}

/// Everything a walk of the directory tree found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Listing {
	entries: Vec<ListedEntry>,
	deleted: Vec<DeletedEntry>,
	slack: Vec<SlackEntry>,
	/// Free clusters read as deleted directories, sorted.
	recovered_clusters: Vec<ClusterNumber>,
}

impl Listing {
	pub fn entries(&self) -> &[ListedEntry] {
		&self.entries
	}

	pub fn deleted(&self) -> &[DeletedEntry] {
		&self.deleted
	}

	pub fn slack(&self) -> &[SlackEntry] {
		&self.slack
	}

	pub fn recovered_clusters(&self) -> &[ClusterNumber] {
		&self.recovered_clusters
	}

	pub fn find(&self, path: &str) -> Option<&ListedEntry> {
		self.entries.iter().find(|entry| entry.path == path)
	}
}

impl Volume {
	/// A machine that lists the whole directory tree.
	pub fn walker(&self) -> WalkVolume<'_> {
		WalkVolume::new(self)
	}

	pub fn walk<R>(&self, source: &R) -> Result<Checked<Listing>, FatxError>
	where
		R: ReadAt,
	{
		io::drive(self.walker(), source)
	}

	pub async fn walk_async<R>(&self, source: &R) -> Result<Checked<Listing>, FatxError>
	where
		R: AsyncReadAt,
	{
		io::drive_async(self.walker(), source).await
	}
}

#[derive(Debug)]
struct PendingDirectory {
	path: String,
	/// The driver's PathNameLength: 0 for the root, and for a child the parent's plus the name
	/// plus a separator (fcbsup.c:143).
	path_length: usize,
	first: ClusterNumber,
	mode: DirectoryMode,
}

/// Lists the directory tree depth first, one directory cluster at a time. A directory the
/// driver could not read is a warning; the walk continues with the rest of the tree.
#[derive(Debug)]
pub struct WalkVolume<'a> {
	volume: &'a Volume,
	pending: Vec<PendingDirectory>,
	current: Option<(PendingDirectory, ReadDirectory<'a>)>,
	visited: HashSet<ClusterNumber>,
	/// One bit per cluster that a live file or directory holds.
	allocated: Vec<u64>,
	listing: Option<Listing>,
	warnings: Vec<Warning>,
}

impl<'a> WalkVolume<'a> {
	fn new(volume: &'a Volume) -> Self {
		let root = PendingDirectory {
			path: ROOT_PATH.to_owned(),
			path_length: 0,
			first: volume.geometry().root_cluster(),
			mode: DirectoryMode::Chain,
		};
		let bits = (volume.geometry().cluster_count() as usize + 1).div_ceil(64);

		Self {
			volume,
			pending: vec![root],
			current: None,
			visited: HashSet::new(),
			allocated: vec![0; bits],
			listing: Some(Listing::default()),
			warnings: Vec::new(),
		}
	}

	fn listing(&mut self) -> &mut Listing {
		self.listing.as_mut().expect("listing present until the walk completes")
	}

	/// Marks `cluster` as held; false when a live file or directory already held it.
	fn mark(&mut self, cluster: ClusterNumber) -> bool {
		let index = cluster.get() as usize;
		let word = &mut self.allocated[index / 64];
		let bit = 1u64 << (index % 64);
		let first_owner = *word & bit == 0;
		*word |= bit;

		first_owner
	}

	fn mark_all<I>(&mut self, path: &str, clusters: I)
	where
		I: IntoIterator<Item = ClusterNumber>,
	{
		let mut reported = false;
		for cluster in clusters {
			if !self.mark(cluster) && !reported {
				self.warnings.push(Warning::CrossLinkedCluster { path: path.to_owned(), cluster });
				reported = true;
			}
		}
	}

	fn unreadable(&mut self, pending: PendingDirectory, error: DirectoryError) {
		match pending.mode {
			DirectoryMode::Chain => self.warnings.push(Warning::UnreadableDirectory { path: pending.path, error }),
			DirectoryMode::Recovered => {
				// Best effort: a deleted directory's clusters may hold anything.
			}
		}
	}

	/// A deleted directory to read, when its first cluster is free and in the image.
	fn recoverable(&self, parent: &PendingDirectory, record: &EntryRecord) -> Option<PendingDirectory> {
		let first = record.first_cluster()?;
		let free = self.volume.is_in_image(first) && self.volume.fat().is_free(first);

		(record.is_directory() && free).then(|| PendingDirectory {
			path: join(&parent.path, record.name()),
			path_length: child_path_length(parent, record),
			first,
			mode: DirectoryMode::Recovered,
		})
	}

	fn absorb(&mut self, pending: PendingDirectory, directory: Directory) {
		// A recovered directory runs on through free clusters, which may be another recovered
		// directory's; records of a cluster already read are not repeated.
		let mut reread = HashSet::new();
		if pending.mode == DirectoryMode::Recovered {
			for cluster in &directory.clusters()[1..] {
				if !self.visited.insert(*cluster) {
					reread.insert(*cluster);
				}
			}
		}
		let fresh = |record: &EntryRecord| !reread.contains(&record.location().cluster);

		let mut subdirectories = Vec::new();
		let deleted_origin = match pending.mode {
			DirectoryMode::Chain => {
				// The whole chain, past the end marker the reader stopped at, belongs to the
				// directory.
				match self.volume.chain(directory.first_cluster()) {
					Ok(runs) => self.mark_all(&pending.path, runs.iter().flat_map(ClusterRun::clusters)),
					Err(error) => {
						self.mark_all(&pending.path, directory.clusters().iter().copied());
						self.warnings.push(Warning::BrokenChain { path: pending.path.clone(), error });
					}
				}
				for invalid in directory.invalid() {
					self.warnings.push(Warning::InvalidEntry { location: invalid.location, reason: invalid.reason });
				}
				// dirctrl.c:58-65: the driver skips a name that would make the path too long.
				let maximum = MAXIMUM_PATH_LENGTH.saturating_sub(1 + pending.path_length).min(MAXIMUM_FILE_NAME_LENGTH);
				for record in directory.entries() {
					let length = record.name().chars().count();
					if length > maximum {
						let reason = InvalidEntryReason::PathTooLong { length: length as u8, maximum: maximum as u8 };
						self.warnings.push(Warning::InvalidEntry { location: record.location(), reason });
						continue;
					}

					let path = join(&pending.path, record.name());
					self.check_live(&pending, &path, record, &mut subdirectories);
					self.listing().entries.push(ListedEntry { path, record: record.clone() });
				}

				DeletedOrigin::LiveDirectory
			}
			DirectoryMode::Recovered => {
				let clusters = directory.clusters().iter().copied();
				self.listing().recovered_clusters.extend(clusters);
				for record in directory.entries().iter().filter(|record| fresh(record)) {
					subdirectories.extend(self.recoverable(&pending, record));
					let entry = DeletedEntry {
						parent: pending.path.clone(),
						record: record.clone(),
						origin: DeletedOrigin::DeletedDirectory,
					};
					self.listing().deleted.push(entry);
				}

				DeletedOrigin::DeletedDirectory
			}
		};

		for record in directory.deleted().iter().filter(|record| fresh(record)) {
			subdirectories.extend(self.recoverable(&pending, record));
			let entry = DeletedEntry { parent: pending.path.clone(), record: record.clone(), origin: deleted_origin };
			self.listing().deleted.push(entry);
		}

		for orphan in directory.slack().iter().filter(|orphan| fresh(&orphan.record)) {
			let entry = SlackEntry { directory: pending.path.clone(), orphan: orphan.clone() };
			self.listing().slack.push(entry);
		}

		// Reversed so the first subdirectory is listed first.
		self.pending.extend(subdirectories.into_iter().rev());
	}

	/// Queues a live directory, or checks a live file's chain.
	fn check_live(
		&mut self,
		parent: &PendingDirectory,
		path: &str,
		record: &EntryRecord,
		subdirectories: &mut Vec<PendingDirectory>,
	) {
		if record.is_directory() {
			let first = match record.first_cluster() {
				Some(first) if self.volume.geometry().is_data_cluster(first) => first,
				Some(first) => {
					let error = DirectoryError::Chain(ChainError::InvalidCluster { cluster: first });
					self.warnings.push(Warning::UnreadableDirectory { path: path.to_owned(), error });
					return;
				}
				None => {
					let error = DirectoryError::MissingFirstCluster;
					self.warnings.push(Warning::UnreadableDirectory { path: path.to_owned(), error });
					return;
				}
			};
			subdirectories.push(PendingDirectory {
				path: path.to_owned(),
				path_length: child_path_length(parent, record),
				first,
				mode: DirectoryMode::Chain,
			});
			return;
		}

		let size = u64::from(record.size());
		let Some(first) = record.first_cluster() else {
			if size != 0 {
				self.warnings
					.push(Warning::BrokenChain { path: path.to_owned(), error: ChainError::MissingFirstCluster });
			}
			return;
		};

		// The whole chain is the file's allocation, even past the clusters its size needs.
		let runs = match self.volume.chain(first) {
			Ok(runs) => runs,
			Err(error) => {
				self.warnings.push(Warning::BrokenChain { path: path.to_owned(), error });
				return;
			}
		};
		self.mark_all(path, runs.iter().flat_map(ClusterRun::clusters));

		let needed = size.div_ceil(u64::from(self.volume.geometry().bytes_per_cluster()));
		let available = runs.iter().map(|run| u64::from(run.count())).sum::<u64>();
		if available < needed {
			let error = ChainError::TooShort { first, needed, available };
			self.warnings.push(Warning::BrokenChain { path: path.to_owned(), error });
		}
	}

	fn finish(&mut self) -> Checked<Listing> {
		let mut listing = self.listing.take().expect("WalkVolume stepped after completion");
		listing.recovered_clusters.sort_unstable();
		listing.recovered_clusters.dedup();

		Checked::new(listing, std::mem::take(&mut self.warnings))
	}
}

impl Machine for WalkVolume<'_> {
	type Output = Checked<Listing>;
	type Error = FatxError;

	fn step(&mut self) -> Result<Step<Self::Output>, FatxError> {
		assert!(self.listing.is_some(), "WalkVolume stepped after completion");

		loop {
			if self.current.is_none() {
				let Some(pending) = self.pending.pop() else {
					return Ok(Step::Done(self.finish()));
				};
				if !self.visited.insert(pending.first) {
					if pending.mode == DirectoryMode::Chain {
						self.warnings.push(Warning::DirectoryLoop { path: pending.path, cluster: pending.first });
					}
					continue;
				}
				let reader = ReadDirectory::new(self.volume, pending.first, pending.mode);
				self.current = Some((pending, reader));
			}

			let (_, reader) = self.current.as_mut().expect("set above");
			match reader.step() {
				Ok(Step::Read(range)) => return Ok(Step::Read(range)),
				Ok(Step::Done(directory)) => {
					let (pending, _) = self.current.take().expect("set above");
					self.absorb(pending, directory);
				}
				Err(FatxError::Directory(error)) => {
					let (pending, _) = self.current.take().expect("set above");
					self.unreadable(pending, error);
				}
				Err(error) => return Err(error),
			}
		}
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), FatxError> {
		let (_, reader) = self.current.as_mut().expect("WalkVolume fed without a requested read");
		match reader.feed(bytes) {
			Ok(()) => Ok(()),
			Err(FatxError::Directory(error)) => {
				let (pending, _) = self.current.take().expect("checked above");
				self.unreadable(pending, error);
				Ok(())
			}
			Err(error) => Err(error),
		}
	}
}

fn join(parent: &str, name: &str) -> String {
	if parent == ROOT_PATH { format!("/{name}") } else { format!("{parent}/{name}") }
}

fn child_path_length(parent: &PendingDirectory, record: &EntryRecord) -> usize {
	parent.path_length + record.name().chars().count() + 1
}
