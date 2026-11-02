//! The writes of each change, in the order that keeps the volume consistent if they stop part
//! way: clusters are allocated in the FAT before data or records point to them, and a record
//! stops pointing to clusters before the FAT frees them. An interrupted change can leave
//! allocated clusters that no record holds, but never a record that points to free clusters.

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::directory::MAXIMUM_DIRECTORY_SIZE;
use crate::entry::DIRECTORY_ENTRY_SIZE;
use crate::entry::EntryFields;
use crate::entry::EntryRecord;
use crate::entry::FileAttributes;
use crate::entry::Slot;
use crate::entry::SlotState;
use crate::entry::mark_deleted;
use crate::entry::parse_slot;
use crate::entry::slot_state;
use crate::error::ChainError;
use crate::error::FatxError;
use crate::error::UpdateError;
use crate::fat::ClusterNumber;
use crate::fat::ClusterRun;
use crate::fat::FatEntry;
use crate::name::FileName;
use crate::name::MAXIMUM_PATH_LENGTH;
use crate::name::VolumePath;
use crate::time::Timestamp;
use crate::update::plan::Fill;
use crate::update::plan::PendingWrite;
use crate::update::plan::Plan;
use crate::update::request::Change;
use crate::update::request::EntryInfo;
use crate::update::request::Request;
use crate::update::request::RequestKind;
use crate::update::request::WriteMode;
use crate::update::request::split;
use crate::update::resolve::DirectoryImage;
use crate::volume::Volume;

/// The writes of a change, the FAT entries to store once they are done, and the result.
#[derive(Debug)]
pub(crate) struct Planned<'d> {
	pub writes: Vec<PendingWrite<'d>>,
	pub fat: BTreeMap<ClusterNumber, FatEntry>,
	pub change: Change,
}

/// A record's fields with its name copied out of the directory.
#[derive(Debug, Clone)]
struct Fields {
	name: Vec<u8>,
	attributes: FileAttributes,
	first_cluster: Option<ClusterNumber>,
	size: u32,
	created: Timestamp,
	modified: Timestamp,
	accessed: Timestamp,
}

impl Fields {
	fn read(slot: &[u8]) -> Self {
		let fields = EntryFields::read(slot);

		Self {
			name: fields.name.to_vec(),
			attributes: fields.attributes,
			first_cluster: fields.first_cluster,
			size: fields.size,
			created: fields.created,
			modified: fields.modified,
			accessed: fields.accessed,
		}
	}

	fn new(name: &FileName, attributes: FileAttributes, now: Timestamp) -> Self {
		Self {
			name: name.as_bytes().to_vec(),
			attributes,
			first_cluster: None,
			size: 0,
			created: now,
			modified: now,
			accessed: now,
		}
	}

	fn entry(&self) -> EntryFields<'_> {
		EntryFields {
			name: &self.name,
			attributes: self.attributes,
			first_cluster: self.first_cluster,
			size: self.size,
			created: self.created,
			modified: self.modified,
			accessed: self.accessed,
		}
	}
}

struct Context<'v, 'd> {
	plan: Plan<'v, 'd>,
	images: Vec<DirectoryImage>,
}

impl<'v, 'd> Context<'v, 'd> {
	fn volume(&self) -> &'v Volume {
		self.plan.volume()
	}

	fn bytes_per_cluster(&self) -> u64 {
		self.plan.bytes_per_cluster()
	}

	fn slot_offset(&self, image: usize, index: usize) -> Result<u64, ChainError> {
		let location = self.images[image].location(index, self.bytes_per_cluster() as usize);

		Ok(self.volume().cluster_range(location.cluster)?.start + u64::from(location.offset))
	}

	/// Stores `fields` in slot `index`, between barriers: what the record points to is durable
	/// first, and what depends on it comes after.
	///
	/// A write may land any prefix of its bytes, and no order of the multi-byte fields keeps
	/// every prefix of an in-place rewrite consistent. So the record is hidden while its fields
	/// change: a live record's FileNameLength becomes the deleted marker, a one-byte write that
	/// lands whole or not at all; the other 63 bytes follow; the FileNameLength is written last
	/// and alone. A change cut short leaves the old record, or a deleted record whose clusters
	/// stay allocated, never a live record with a torn first cluster or size.
	fn store(&mut self, image: usize, index: usize, fields: &Fields) -> Result<(), ChainError> {
		let current = self.images[image].slot(index);
		let live = !matches!(slot_state(current), SlotState::End | SlotState::Deleted);
		let slot = fields.entry().store(current);
		let offset = self.slot_offset(image, index)?;

		if live {
			self.hide(image, index, offset);
		}
		self.images[image].set_slot(index, &slot);
		self.plan.barrier();
		self.plan.write(offset + 1, Cow::Owned(slot[1..].to_vec()));
		self.plan.barrier();
		self.plan.write(offset, Cow::Owned(slot[..1].to_vec()));
		self.plan.barrier();

		Ok(())
	}

	/// Marks slot `index` deleted with a one-byte write, as fatsup.c FatxDeleteFile changes
	/// only the FileNameLength. The rest of the record stays for recovery.
	fn delete(&mut self, image: usize, index: usize) -> Result<(), ChainError> {
		let offset = self.slot_offset(image, index)?;
		self.hide(image, index, offset);

		Ok(())
	}

	fn hide(&mut self, image: usize, index: usize, offset: u64) {
		let marked = mark_deleted(self.images[image].slot(index));
		self.images[image].set_slot(index, &marked);
		self.plan.barrier();
		self.plan.write(offset, Cow::Owned(marked[..1].to_vec()));
		self.plan.barrier();
	}

	/// The record in slot `index` as the reader parses it.
	fn record(&self, image: usize, index: usize) -> EntryRecord {
		let location = self.images[image].location(index, self.bytes_per_cluster() as usize);
		match parse_slot(self.images[image].slot(index), location) {
			Slot::Live(record) | Slot::Deleted(record) => record,
			other => unreachable!("a stored record parses as live or deleted, not {other:?}"),
		}
	}

	/// The slot for a new record: `free`, or the first slot of a new cluster added to the
	/// directory as fatsup.c FatxExtendDirectoryAllocation does it: allocate the cluster, fill
	/// it with free records, then link it to the directory's chain.
	fn new_slot(&mut self, image: usize, free: Option<usize>) -> Result<usize, FatxError> {
		if let Some(free) = free {
			return Ok(free);
		}

		let bytes_per_cluster = self.bytes_per_cluster();
		let directory = &self.images[image];
		if directory.clusters().len() as u64 * bytes_per_cluster >= MAXIMUM_DIRECTORY_SIZE {
			let maximum = MAXIMUM_DIRECTORY_SIZE / DIRECTORY_ENTRY_SIZE as u64;
			return Err(UpdateError::DirectoryFull { path: directory.path().to_owned(), maximum }.into());
		}
		let last = *directory.clusters().last().expect("a directory has a cluster");
		let index = directory.slot_count();

		let cluster = self.plan.allocate(1, None)?[0];
		self.plan.flush_fat();
		self.plan.initialize_directory_cluster(cluster)?;
		self.plan.link(last, cluster);
		self.plan.flush_fat();
		self.images[image].extend(cluster, bytes_per_cluster as usize);

		Ok(index)
	}

	/// The clusters of an existing file's chain, which must hold its size.
	fn file_clusters(&self, fields: &Fields) -> Result<Vec<ClusterNumber>, ChainError> {
		let Some(first) = fields.first_cluster else {
			if fields.size != 0 {
				return Err(ChainError::MissingFirstCluster);
			}
			return Ok(Vec::new());
		};

		let clusters: Vec<ClusterNumber> = self.volume().chain(first)?.iter().flat_map(ClusterRun::clusters).collect();
		let needed = u64::from(fields.size).div_ceil(self.bytes_per_cluster());
		if (clusters.len() as u64) < needed {
			return Err(ChainError::TooShort { first, needed, available: clusters.len() as u64 });
		}

		Ok(clusters)
	}

	/// Extends `clusters` to hold `end` bytes, linked after the current last cluster.
	fn grow(&mut self, clusters: &mut Vec<ClusterNumber>, end: u64) -> Result<(), FatxError> {
		let needed = end.div_ceil(self.bytes_per_cluster());
		let missing = needed.saturating_sub(clusters.len() as u64);
		if missing == 0 {
			return Ok(());
		}

		let added = self.plan.allocate(missing, clusters.last().copied())?;
		self.plan.flush_fat();
		clusters.extend(added);

		Ok(())
	}

	/// readwrit.c:854-861: a file's allocation, rounded up to clusters, must fit 32 bits.
	fn check_end(&self, end: u64) -> Result<(), UpdateError> {
		let maximum = (u64::from(u32::MAX) + 1) - self.bytes_per_cluster();
		if end > maximum {
			return Err(UpdateError::FileTooLarge { maximum });
		}

		Ok(())
	}
}

/// Stages the writes of `request`. `images` are the directories of [`Request::targets`], and
/// `resolved` gives the index of each target's directory.
pub(crate) fn plan<'d>(
	volume: &Volume,
	request: &Request<'d>,
	images: Vec<DirectoryImage>,
	resolved: &[Option<usize>],
) -> Result<Planned<'d>, FatxError> {
	let mut context = Context { plan: Plan::new(volume), images };
	let parent = resolved[0].expect("a parent target is a directory");

	let change = match request.kind() {
		RequestKind::CreateDirectory { path, now } => create_directory(&mut context, parent, path, *now)?,
		RequestKind::WriteFile { path, data, mode, now } => write_file(&mut context, parent, path, data, *mode, *now)?,
		RequestKind::SetLength { path, length } => set_length(&mut context, parent, path, *length)?,
		RequestKind::Remove { path } => remove(&mut context, parent, path, resolved[1])?,
		RequestKind::Rename { from, to } => {
			let target = resolved[1].expect("a parent target is a directory");
			rename(&mut context, parent, from, target, to)?
		}
		RequestKind::SetInfo { path, info } => set_info(&mut context, parent, path, info)?,
	};

	let (writes, fat) = context.plan.finish();

	Ok(Planned { writes, fat, change })
}

/// create.c:560: the new entry's path must fit the driver's limit.
fn check_path_length(path: &VolumePath) -> Result<(), UpdateError> {
	let length = path.length();
	if length > MAXIMUM_PATH_LENGTH {
		return Err(UpdateError::PathTooLong { path: path.to_string(), length, maximum: MAXIMUM_PATH_LENGTH });
	}

	Ok(())
}

/// The existing record of `name` in `parent`, or an error naming the path.
fn find(context: &Context<'_, '_>, parent: usize, name: &FileName) -> Result<(usize, Fields), UpdateError> {
	let directory = &context.images[parent];
	let Some(index) = directory.lookup(name).found else {
		return Err(UpdateError::NotFound { path: directory.child_path(name.as_bytes()) });
	};

	Ok((index, existing(context, parent, index)?))
}

/// The fields of the record in slot `index`, which the lookup matched by name. A record the
/// reader skips as invalid is not changed.
fn existing(context: &Context<'_, '_>, parent: usize, index: usize) -> Result<Fields, UpdateError> {
	let directory = &context.images[parent];
	let location = directory.location(index, context.bytes_per_cluster() as usize);
	match parse_slot(directory.slot(index), location) {
		Slot::Live(_) => Ok(Fields::read(directory.slot(index))),
		Slot::Invalid(reason) => {
			let path = directory.child_path(EntryFields::read(directory.slot(index)).name);
			Err(UpdateError::InvalidRecord { path, reason })
		}
		other => unreachable!("a record a name matched is live or invalid, not {other:?}"),
	}
}

/// create.c FatxCreateNewFile for a directory: a free slot, then the directory's first cluster
/// allocated and filled with free records, then the record.
fn create_directory(
	context: &mut Context<'_, '_>,
	parent: usize,
	path: &VolumePath,
	now: Timestamp,
) -> Result<Change, FatxError> {
	let (_, name) = split(path);
	check_path_length(path)?;
	let lookup = context.images[parent].lookup(name);
	if let Some(found) = lookup.found {
		let stored = EntryFields::read(context.images[parent].slot(found)).name;
		return Err(UpdateError::AlreadyExists { path: context.images[parent].child_path(stored) }.into());
	}

	let index = context.new_slot(parent, lookup.free)?;
	let cluster = context.plan.allocate(1, None)?[0];
	context.plan.flush_fat();
	context.plan.initialize_directory_cluster(cluster)?;

	let mut fields = Fields::new(name, FileAttributes::DIRECTORY, now);
	fields.first_cluster = Some(cluster);
	context.store(parent, index, &fields)?;

	Ok(Change::Created {
		path: context.images[parent].child_path(name.as_bytes()),
		record: context.record(parent, index),
	})
}

fn write_file<'d>(
	context: &mut Context<'_, 'd>,
	parent: usize,
	path: &VolumePath,
	data: &'d [u8],
	mode: WriteMode,
	now: Timestamp,
) -> Result<Change, FatxError> {
	let (_, name) = split(path);
	let lookup = context.images[parent].lookup(name);
	let Some(index) = lookup.found else {
		if mode != WriteMode::Replace {
			return Err(UpdateError::NotFound { path: context.images[parent].child_path(name.as_bytes()) }.into());
		}
		return create_file(context, parent, path, lookup.free, data, now);
	};

	let fields = existing(context, parent, index)?;
	if fields.attributes.is_directory() {
		return Err(UpdateError::IsADirectory { path: context.images[parent].child_path(&fields.name) }.into());
	}

	match mode {
		WriteMode::Replace => replace_file(context, parent, index, fields, data, now),
		WriteMode::Append => {
			let offset = u64::from(fields.size);
			write_at(context, parent, index, fields, offset, data, now)
		}
		WriteMode::At(offset) => write_at(context, parent, index, fields, offset, data, now),
	}
}

/// A new file: a free slot, the clusters allocated, the data, then the record. The driver
/// writes the record first with size 0 and again after the data; one record write after the
/// data leaves nothing visible if the data writes stop part way.
fn create_file<'d>(
	context: &mut Context<'_, 'd>,
	parent: usize,
	path: &VolumePath,
	free: Option<usize>,
	data: &'d [u8],
	now: Timestamp,
) -> Result<Change, FatxError> {
	let (_, name) = split(path);
	check_path_length(path)?;
	let length = data.len() as u64;
	context.check_end(length)?;

	let index = context.new_slot(parent, free)?;
	let mut clusters = Vec::new();
	context.grow(&mut clusters, length)?;
	context.plan.write_file_range(&clusters, 0, Fill::Data(data))?;

	let mut fields = Fields::new(name, FileAttributes::from_bits(0), now);
	fields.first_cluster = clusters.first().copied();
	fields.size = length as u32;
	context.store(parent, index, &fields)?;

	Ok(Change::Created {
		path: context.images[parent].child_path(name.as_bytes()),
		record: context.record(parent, index),
	})
}

/// create.c FatxOverwriteExistingFile: the record loses its clusters and takes new timestamps,
/// the FAT frees them (fatsup.c FatxDeleteFileAllocation), then the data is written as for a
/// new file.
fn replace_file<'d>(
	context: &mut Context<'_, 'd>,
	parent: usize,
	index: usize,
	mut fields: Fields,
	data: &'d [u8],
	now: Timestamp,
) -> Result<Change, FatxError> {
	let length = data.len() as u64;
	context.check_end(length)?;

	fields.created = now;
	fields.modified = now;
	fields.accessed = now;
	let old_first = fields.first_cluster.take();
	if old_first.is_some() || fields.size != 0 {
		fields.size = 0;
		context.store(parent, index, &fields)?;
	}
	if let Some(first) = old_first {
		context.plan.free_chain(first, false);
		context.plan.flush_fat();
	}

	let mut clusters = Vec::new();
	context.grow(&mut clusters, length)?;
	context.plan.write_file_range(&clusters, 0, Fill::Data(data))?;
	fields.first_cluster = clusters.first().copied();
	fields.size = length as u32;
	context.store(parent, index, &fields)?;

	Ok(Change::Modified {
		path: context.images[parent].child_path(&fields.name),
		record: context.record(parent, index),
	})
}

/// readwrit.c FatxFsdReadWrite for a write: the chain extended to hold the data, the data,
/// then the record with the new size and last write time. Like the driver (readwrit.c:747),
/// a write of no bytes changes nothing.
#[allow(clippy::too_many_arguments)]
fn write_at<'d>(
	context: &mut Context<'_, 'd>,
	parent: usize,
	index: usize,
	mut fields: Fields,
	offset: u64,
	data: &'d [u8],
	now: Timestamp,
) -> Result<Change, FatxError> {
	if data.is_empty() {
		let path = context.images[parent].child_path(&fields.name);
		return Ok(Change::Modified { path, record: context.record(parent, index) });
	}

	let end = offset
		.checked_add(data.len() as u64)
		.ok_or(UpdateError::FileTooLarge { maximum: (u64::from(u32::MAX) + 1) - context.bytes_per_cluster() })?;
	context.check_end(end)?;

	let size = u64::from(fields.size);
	let mut clusters = context.file_clusters(&fields)?;
	context.grow(&mut clusters, end)?;
	if offset > size {
		context.plan.write_file_range(&clusters, size, Fill::Zeroes(offset - size))?;
	}
	context.plan.write_file_range(&clusters, offset, Fill::Data(data))?;

	fields.first_cluster = clusters.first().copied();
	fields.size = size.max(end) as u32;
	fields.modified = now;
	context.store(parent, index, &fields)?;

	Ok(Change::Modified {
		path: context.images[parent].child_path(&fields.name),
		record: context.record(parent, index),
	})
}

/// fileinfo.c FatxSetEndOfFileInformation then FatxSetAllocationSize. A shorter file's record
/// is written before the FAT frees its tail (fatsup.c FatxTruncateFileAllocation); a longer
/// file's clusters are allocated and zeroed before the record grows. The driver leaves the
/// old bytes of new clusters in place; zeroing them keeps deleted data out of a live file.
fn set_length(
	context: &mut Context<'_, '_>,
	parent: usize,
	path: &VolumePath,
	length: u32,
) -> Result<Change, FatxError> {
	let (index, mut fields) = find(context, parent, split(path).1)?;
	let path = context.images[parent].child_path(&fields.name);
	if fields.attributes.is_directory() {
		return Err(UpdateError::IsADirectory { path }.into());
	}

	let size = u64::from(fields.size);
	let length = u64::from(length);
	context.check_end(length)?;
	let mut clusters = context.file_clusters(&fields)?;
	if length > size {
		context.grow(&mut clusters, length)?;
		context.plan.write_file_range(&clusters, size, Fill::Zeroes(length - size))?;
		fields.first_cluster = clusters.first().copied();
		fields.size = length as u32;
		context.store(parent, index, &fields)?;
	} else if length < size || clusters.len() as u64 > length.div_ceil(context.bytes_per_cluster()) {
		let keep = length.div_ceil(context.bytes_per_cluster()) as usize;
		fields.size = length as u32;
		if keep == 0 {
			fields.first_cluster = None;
		}
		context.store(parent, index, &fields)?;
		match keep {
			0 => context.plan.free_chain(clusters[0], false),
			keep => context.plan.free_chain(clusters[keep - 1], true),
		}
		context.plan.flush_fat();
	}

	Ok(Change::Modified { path, record: context.record(parent, index) })
}

/// fatsup.c FatxDeleteFile: the record marked deleted, keeping its name, first cluster and
/// size, then its chain freed.
fn remove(
	context: &mut Context<'_, '_>,
	parent: usize,
	path: &VolumePath,
	directory: Option<usize>,
) -> Result<Change, FatxError> {
	let (index, fields) = find(context, parent, split(path).1)?;
	let path = context.images[parent].child_path(&fields.name);
	if let Some(directory) = directory
		&& context.images[directory].has_entries(context.bytes_per_cluster() as usize)
	{
		return Err(UpdateError::DirectoryNotEmpty { path }.into());
	}

	context.delete(parent, index)?;
	if let Some(first) = fields.first_cluster {
		context.plan.free_chain(first, false);
		context.plan.flush_fat();
	}

	Ok(Change::Removed { path, record: context.record(parent, index) })
}

/// fileinfo.c FatxSetRenameInformation without replacement. A rename in one directory rewrites
/// the record's name in place. A move gets a slot in the target directory, marks the old
/// record deleted, then writes the new one.
fn rename(
	context: &mut Context<'_, '_>,
	from_parent: usize,
	from: &VolumePath,
	to_parent: usize,
	to: &VolumePath,
) -> Result<Change, FatxError> {
	let (index, fields) = find(context, from_parent, split(from).1)?;
	let from_path = context.images[from_parent].child_path(&fields.name);
	let (target_directory, name) = split(to);
	check_path_length(to)?;
	if fields.attributes.is_directory() && target_directory.starts_with(from) {
		return Err(UpdateError::MoveIntoItself { from: from_path, to: to.to_string() }.into());
	}

	let lookup = context.images[to_parent].lookup(name);
	let to_path = context.images[to_parent].child_path(name.as_bytes());
	if lookup.found.is_some_and(|found| to_parent != from_parent || found != index) {
		return Err(UpdateError::AlreadyExists { path: to_path }.into());
	}

	let renamed = Fields { name: name.as_bytes().to_vec(), ..fields };
	let index = if to_parent == from_parent {
		context.store(from_parent, index, &renamed)?;
		index
	} else {
		let target = context.new_slot(to_parent, lookup.free)?;
		context.delete(from_parent, index)?;
		context.store(to_parent, target, &renamed)?;
		target
	};

	Ok(Change::Renamed { from: from_path, to: to_path, record: context.record(to_parent, index) })
}

/// fileinfo.c FatxSetBasicInformation: the record is written only when a field changes.
fn set_info(
	context: &mut Context<'_, '_>,
	parent: usize,
	path: &VolumePath,
	info: &EntryInfo,
) -> Result<Change, FatxError> {
	let (index, fields) = find(context, parent, split(path).1)?;
	let path = context.images[parent].child_path(&fields.name);

	let mut updated = fields.clone();
	if let Some(attributes) = info.attributes {
		let supported = FileAttributes::HIDDEN.bits() | FileAttributes::DIRECTORY.bits();
		if attributes.bits() & !supported != 0 {
			return Err(UpdateError::UnsupportedAttributes { attributes: attributes.bits() }.into());
		}
		let directory = fields.attributes.is_directory();
		if attributes.is_directory() && !directory {
			return Err(UpdateError::DirectoryAttributeChange { path }.into());
		}
		updated.attributes = match directory {
			true => FileAttributes::from_bits(attributes.bits() | FileAttributes::DIRECTORY.bits()),
			false => attributes,
		};
	}
	updated.created = info.created.unwrap_or(fields.created);
	updated.modified = info.modified.unwrap_or(fields.modified);
	updated.accessed = info.accessed.unwrap_or(fields.accessed);

	let unchanged = updated.attributes == fields.attributes
		&& updated.created == fields.created
		&& updated.modified == fields.modified
		&& updated.accessed == fields.accessed;
	if !unchanged {
		context.store(parent, index, &updated)?;
	}

	Ok(Change::Modified { path, record: context.record(parent, index) })
}
