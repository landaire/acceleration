//! Path lookup for a change: the raw records of the directories a change reads or writes, found
//! the way create.c FatxLookupElementNameInDirectory searches them.

use std::ops::Range;

use crate::directory::MAXIMUM_DIRECTORY_SIZE;
use crate::entry::DIRECTORY_ENTRY_SIZE;
use crate::entry::EntryFields;
use crate::entry::EntryLocation;
use crate::entry::Slot;
use crate::entry::SlotState;
use crate::entry::parse_slot;
use crate::entry::slot_state;
use crate::error::DirectoryError;
use crate::error::FatxError;
use crate::error::UpdateError;
use crate::fat::ClusterNumber;
use crate::fat::ClusterRun;
use crate::machine::expect_len;
use crate::name::FileName;
use crate::name::VolumePath;
use crate::volume::Volume;

/// A directory to load for a change.
#[derive(Debug, Clone)]
pub(crate) enum Target {
	/// The directory at the path.
	Directory(VolumePath),
	/// The entry at the path when it is a directory.
	IfDirectory(VolumePath),
}

impl Target {
	fn path(&self) -> &VolumePath {
		match self {
			Self::Directory(path) | Self::IfDirectory(path) => path,
		}
	}
}

/// Where a name is in a directory, and the first slot a new record can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Lookup {
	pub found: Option<usize>,
	/// The first free, deleted or end-marker slot before the name or the end marker. `None`
	/// when every slot of the directory's clusters holds a record.
	pub free: Option<usize>,
}

/// A directory's clusters and the bytes of those up to the one with the end marker.
#[derive(Debug, Clone)]
pub(crate) struct DirectoryImage {
	/// The path with the names as stored.
	path: String,
	clusters: Vec<ClusterNumber>,
	bytes: Vec<u8>,
	ended: bool,
}

impl DirectoryImage {
	pub(crate) fn path(&self) -> &str {
		&self.path
	}

	pub(crate) fn clusters(&self) -> &[ClusterNumber] {
		&self.clusters
	}

	fn first(&self) -> ClusterNumber {
		self.clusters[0]
	}

	/// The path of the entry `name` in this directory.
	pub(crate) fn child_path(&self, name: &[u8]) -> String {
		let name: String = name.iter().map(|byte| char::from(*byte)).collect();
		if self.path == "/" { format!("/{name}") } else { format!("{}/{name}", self.path) }
	}

	pub(crate) fn slot_count(&self) -> usize {
		self.bytes.len() / DIRECTORY_ENTRY_SIZE
	}

	pub(crate) fn slot(&self, index: usize) -> &[u8] {
		let start = index * DIRECTORY_ENTRY_SIZE;

		&self.bytes[start..start + DIRECTORY_ENTRY_SIZE]
	}

	pub(crate) fn set_slot(&mut self, index: usize, slot: &[u8]) {
		let start = index * DIRECTORY_ENTRY_SIZE;
		self.bytes[start..start + DIRECTORY_ENTRY_SIZE].copy_from_slice(slot);
	}

	/// Appends a cluster of free records to the directory.
	pub(crate) fn extend(&mut self, cluster: ClusterNumber, bytes_per_cluster: usize) {
		self.clusters.push(cluster);
		self.bytes.resize(self.bytes.len() + bytes_per_cluster, 0xff);
		self.ended = true;
	}

	pub(crate) fn location(&self, index: usize, bytes_per_cluster: usize) -> EntryLocation {
		let slots_per_cluster = bytes_per_cluster / DIRECTORY_ENTRY_SIZE;

		EntryLocation {
			cluster: self.clusters[index / slots_per_cluster],
			offset: ((index % slots_per_cluster) * DIRECTORY_ENTRY_SIZE) as u32,
		}
	}

	pub(crate) fn lookup(&self, name: &FileName) -> Lookup {
		let mut free = None;
		for index in 0..self.slot_count() {
			match slot_state(self.slot(index)) {
				SlotState::End => {
					return Lookup { found: None, free: free.or(Some(index)) };
				}
				SlotState::Deleted => {
					free = free.or(Some(index));
				}
				SlotState::Named { name: stored } if name.matches(stored) => {
					return Lookup { found: Some(index), free };
				}
				SlotState::Named { .. } | SlotState::Unnamed => {
					// Another record.
				}
			}
		}

		Lookup { found: None, free }
	}

	/// Whether a record the driver would list comes before the end marker (fatsup.c
	/// FatxIsDirectoryEmpty).
	pub(crate) fn has_entries(&self, bytes_per_cluster: usize) -> bool {
		(0..self.slot_count())
			.map(|index| parse_slot(self.slot(index), self.location(index, bytes_per_cluster)))
			.take_while(|slot| *slot != Slot::End)
			.any(|slot| matches!(slot, Slot::Live(_)))
	}
}

/// Loads the directories of a list of targets, reading each directory once.
#[derive(Debug)]
pub(crate) struct Resolver {
	targets: Vec<Target>,
	/// For each resolved target, the index of its directory, or `None` for an
	/// [`Target::IfDirectory`] path that is a file.
	resolved: Vec<Option<usize>>,
	images: Vec<DirectoryImage>,
	walk: Option<Walk>,
	loading: Option<Loading>,
}

/// The directory a target's lookup has reached.
#[derive(Debug)]
struct Walk {
	depth: usize,
	first: ClusterNumber,
	path: String,
}

#[derive(Debug)]
struct Loading {
	image: DirectoryImage,
	next: usize,
	requested: Option<Range<u64>>,
}

impl Resolver {
	pub(crate) fn new(targets: Vec<Target>) -> Self {
		Self { targets, resolved: Vec::new(), images: Vec::new(), walk: None, loading: None }
	}

	/// The next read, or `None` once every target is resolved.
	pub(crate) fn step(&mut self, volume: &Volume) -> Result<Option<Range<u64>>, FatxError> {
		assert!(
			self.loading.as_ref().is_none_or(|loading| loading.requested.is_none()),
			"Resolver stepped before the requested read was fed"
		);

		loop {
			if let Some(loading) = &mut self.loading {
				if let Some(range) = loading.next_range(volume)? {
					return Ok(Some(range));
				}
				let loaded = self.loading.take().expect("matched above");
				self.images.push(loaded.image);
				continue;
			}

			let Some(target) = self.targets.get(self.resolved.len()) else {
				return Ok(None);
			};
			let walk = self.walk.get_or_insert_with(|| Walk {
				depth: 0,
				first: volume.geometry().root_cluster(),
				path: "/".to_owned(),
			});
			let Some(index) = self.images.iter().position(|image| image.first() == walk.first) else {
				self.loading = Some(Loading::new(volume, walk)?);
				continue;
			};

			let names = target.path().names();
			if walk.depth == names.len() {
				self.resolved.push(Some(index));
				self.walk = None;
				continue;
			}

			let image = &self.images[index];
			let name = &names[walk.depth];
			let Some(slot) = image.lookup(name).found else {
				return Err(UpdateError::NotFound { path: image.child_path(name.as_bytes()) }.into());
			};
			let fields = EntryFields::read(image.slot(slot));
			let path = image.child_path(fields.name);
			if !fields.attributes.is_directory() {
				if matches!(target, Target::IfDirectory(_)) && walk.depth + 1 == names.len() {
					self.resolved.push(None);
					self.walk = None;
					continue;
				}
				return Err(UpdateError::NotADirectory { path }.into());
			}

			let first = fields.first_cluster.ok_or(DirectoryError::MissingFirstCluster)?;
			walk.depth += 1;
			walk.first = first;
			walk.path = path;
		}
	}

	pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<(), FatxError> {
		let loading = self.loading.as_mut().expect("Resolver fed without a requested read");
		let range = loading.requested.take().expect("Resolver fed without a requested read");
		expect_len(&range, bytes)?;
		loading.next += 1;
		loading.image.ended =
			bytes.as_chunks::<DIRECTORY_ENTRY_SIZE>().0.iter().any(|slot| slot_state(slot) == SlotState::End);
		loading.image.bytes.extend_from_slice(bytes);

		Ok(())
	}

	/// The loaded directories, and for each target the index of its directory.
	pub(crate) fn finish(self) -> (Vec<DirectoryImage>, Vec<Option<usize>>) {
		(self.images, self.resolved)
	}
}

impl Loading {
	fn new(volume: &Volume, walk: &Walk) -> Result<Self, FatxError> {
		let runs = volume.chain(walk.first).map_err(DirectoryError::from)?;
		let clusters = runs.iter().flat_map(ClusterRun::clusters).collect();
		let image = DirectoryImage { path: walk.path.clone(), clusters, bytes: Vec::new(), ended: false };

		Ok(Self { image, next: 0, requested: None })
	}

	fn next_range(&mut self, volume: &Volume) -> Result<Option<Range<u64>>, FatxError> {
		if self.image.ended || self.next == self.image.clusters.len() {
			return Ok(None);
		}

		// create.c:157: a lookup past the largest directory fails.
		if self.image.bytes.len() as u64 >= MAXIMUM_DIRECTORY_SIZE {
			return Err(DirectoryError::TooLarge { first: self.image.first(), maximum: MAXIMUM_DIRECTORY_SIZE }.into());
		}

		let range = volume.cluster_range(self.image.clusters[self.next]).map_err(DirectoryError::from)?;
		self.requested = Some(range.clone());

		Ok(Some(range))
	}
}
