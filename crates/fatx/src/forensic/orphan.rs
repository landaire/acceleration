//! Directory records outside any live directory: in free clusters, or after the end marker of a
//! live directory.

use crate::entry::DIRECTORY_ENTRY_SIZE;
use crate::entry::EntryLocation;
use crate::entry::EntryRecord;
use crate::entry::Slot;
use crate::entry::parse_slot;
use crate::fat::ClusterNumber;
use crate::forensic::Confidence;
use crate::volume::Volume;

/// Where an orphaned record was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrphanOrigin {
	/// A cluster the FAT marks free.
	FreeCluster,
	/// After the end marker of the live directory that starts at `directory`.
	DirectorySlack { directory: ClusterNumber },
}

/// A record that passes every check a real directory record would: a printable legal name,
/// supported attributes, valid timestamps, a first cluster inside the volume and a size the
/// partition can hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanEntry {
	pub origin: OrphanOrigin,
	pub record: EntryRecord,
	/// Other records in the same cluster that pass the same checks.
	pub neighbors: u32,
	/// Whether the FAT marks the record's first cluster free; `None` when it has none.
	pub first_cluster_free: Option<bool>,
	pub confidence: Confidence,
}

/// The records of `cluster`'s bytes from `start` that pass the orphan checks.
pub(crate) fn assess_slots(
	volume: &Volume,
	cluster: ClusterNumber,
	bytes: &[u8],
	start: usize,
	origin: OrphanOrigin,
) -> Vec<OrphanEntry> {
	let records: Vec<EntryRecord> = bytes
		.as_chunks::<DIRECTORY_ENTRY_SIZE>()
		.0
		.iter()
		.enumerate()
		.skip(start.div_ceil(DIRECTORY_ENTRY_SIZE))
		.filter_map(|(index, record)| {
			let location = EntryLocation { cluster, offset: (index * DIRECTORY_ENTRY_SIZE) as u32 };
			plausible_record(volume, record, location)
		})
		.collect();
	let neighbors = records.len().saturating_sub(1) as u32;

	records
		.into_iter()
		.map(|record| {
			let first_cluster_free = record.first_cluster().map(|first| volume.fat().is_free(first));
			let confidence = match (neighbors > 0, record.has_uniform_padding()) {
				(true, true) => Confidence::High,
				(true, false) | (false, true) => Confidence::Medium,
				(false, false) => Confidence::Low,
			};

			OrphanEntry { origin, record, neighbors, first_cluster_free, confidence }
		})
		.collect()
}

fn plausible_record(volume: &Volume, record: &[u8], location: EntryLocation) -> Option<EntryRecord> {
	let (Slot::Live(record) | Slot::Deleted(record)) = parse_slot(record, location) else {
		return None;
	};

	let printable = record.name().chars().all(|character| (' '..='~').contains(&character));
	let first_cluster_valid = match record.first_cluster() {
		Some(first) => volume.geometry().is_data_cluster(first),
		None => record.size() == 0,
	};
	let size_valid = if record.is_directory() {
		record.size() == 0
	} else {
		let geometry = volume.geometry();
		u64::from(record.size()) <= u64::from(geometry.cluster_count()) * u64::from(geometry.bytes_per_cluster())
	};

	(printable && record.has_valid_timestamps() && first_cluster_valid && size_valid).then_some(record)
}
