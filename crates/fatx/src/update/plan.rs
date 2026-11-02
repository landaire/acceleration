//! The FAT changes and the ordered writes of one change, staged before any write reaches the
//! source.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::error::ChainError;
use crate::error::UpdateError;
use crate::fat::ClusterNumber;
use crate::fat::FatEntry;
use crate::machine::Effect;
use crate::volume::METADATA_SIZE;
use crate::volume::Volume;

/// Zero bytes for writes that clear a range, at most a cluster at a time.
pub(crate) static ZEROES: [u8; 0x10000] = [0; 0x10000];

/// Free directory records for a new directory cluster, as allocsup.c
/// FatxInitializeDirectoryCluster fills it.
pub(crate) static FREE_RECORDS: [u8; 0x10000] = [0xff; 0x10000];

/// One step of a change: a write of `data` at `offset` of the source, or a barrier that makes
/// the writes before it durable before any after it.
#[derive(Debug)]
pub(crate) enum PendingWrite<'d> {
	Write { offset: u64, data: Cow<'d, [u8]> },
	Flush,
}

impl<'d> PendingWrite<'d> {
	pub(crate) fn new(offset: u64, data: Cow<'d, [u8]>) -> Self {
		Self::Write { offset, data }
	}

	pub(crate) fn effect<T>(&self) -> Effect<'_, T> {
		match self {
			Self::Write { offset, data } => Effect::Write { offset: *offset, data },
			Self::Flush => Effect::Flush,
		}
	}
}

/// How the bytes of a range of a new or extended file are written.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Fill<'d> {
	Data(&'d [u8]),
	Zeroes(u64),
}

impl Fill<'_> {
	fn len(&self) -> u64 {
		match self {
			Self::Data(data) => data.len() as u64,
			Self::Zeroes(length) => *length,
		}
	}
}

/// FAT entries changed over the volume's FAT, and the writes so far.
///
/// Each FAT flush and each record write sits between barriers, so the writes a later stage
/// depends on are durable first. A failed write lands a prefix ending on a sector boundary
/// ([`crate::io::WriteAt`]). FAT entries (2 or 4 bytes, aligned, in a FAT that starts at
/// 0x1000) and directory records (64 bytes, aligned, in sector-aligned clusters) never straddle
/// a sector of a volume that starts on a sector boundary, as every partition does, so each
/// lands whole or not at all; the FAT entries of one flush never depend on
/// each other, so any set of them may land.
#[derive(Debug)]
pub(crate) struct Plan<'v, 'd> {
	volume: &'v Volume,
	fat: BTreeMap<ClusterNumber, FatEntry>,
	/// FAT pages changed since the last flush.
	dirty: BTreeSet<u64>,
	writes: Vec<PendingWrite<'d>>,
}

impl<'v, 'd> Plan<'v, 'd> {
	pub(crate) fn new(volume: &'v Volume) -> Self {
		Self { volume, fat: BTreeMap::new(), dirty: BTreeSet::new(), writes: Vec::new() }
	}

	pub(crate) fn volume(&self) -> &'v Volume {
		self.volume
	}

	pub(crate) fn bytes_per_cluster(&self) -> u64 {
		u64::from(self.volume.geometry().bytes_per_cluster())
	}

	pub(crate) fn entry(&self, cluster: ClusterNumber) -> FatEntry {
		match self.fat.get(&cluster) {
			Some(entry) => *entry,
			None => self.volume.fat().entry(cluster).expect("the FAT covers every data cluster"),
		}
	}

	fn set(&mut self, cluster: ClusterNumber, entry: FatEntry) {
		self.fat.insert(cluster, entry);
		self.dirty.insert(self.page(cluster));
	}

	fn page(&self, cluster: ClusterNumber) -> u64 {
		u64::from(cluster.get()) * self.volume.fat().width().entry_size() / METADATA_SIZE
	}

	/// Free data clusters that the image holds. A volume file can have clusters past the end of
	/// the image; the console grows the file to reach them, this crate does not.
	pub(crate) fn free_clusters(&self) -> u64 {
		(1..=self.volume.image_cluster_count())
			.filter_map(ClusterNumber::new)
			.filter(|cluster| self.entry(*cluster) == FatEntry::Free)
			.count() as u64
	}

	/// Allocates `count` free clusters as a chain and links it after `after`, the last cluster
	/// of an existing chain. Without `after` the search starts at cluster 1; with it, at the
	/// cluster after `after`, and wraps around to cluster 1 (allocsup.c FatxAllocateClusters).
	/// The driver starts a new chain at its allocation hint, which is never above the lowest
	/// free cluster, so the clusters chosen are the same. The new chain is flushed before the
	/// link to it is staged, as FatxLinkClusterChains writes the link after the new pages.
	pub(crate) fn allocate(
		&mut self,
		count: u64,
		after: Option<ClusterNumber>,
	) -> Result<Vec<ClusterNumber>, UpdateError> {
		if count == 0 {
			return Ok(Vec::new());
		}

		let available = self.free_clusters();
		if count > available {
			return Err(UpdateError::DiskFull { needed: count, available });
		}

		let last = self.volume.image_cluster_count();
		let start = after.map_or(1, |cluster| cluster.get().saturating_add(1));
		let clusters: Vec<ClusterNumber> = (start..=last)
			.chain(1..start.min(last + 1))
			.filter_map(ClusterNumber::new)
			.filter(|cluster| self.entry(*cluster) == FatEntry::Free)
			.take(count as usize)
			.collect();

		for pair in clusters.windows(2) {
			self.set(pair[0], FatEntry::Link(pair[1].get()));
		}
		self.set(*clusters.last().expect("count is not zero"), FatEntry::Last);
		if let Some(after) = after {
			self.flush_fat();
			self.link(after, clusters[0]);
		}

		Ok(clusters)
	}

	/// Links `next` after `last`, the end of a chain (allocsup.c FatxLinkClusterChains).
	/// `next`'s chain must be flushed already.
	pub(crate) fn link(&mut self, last: ClusterNumber, next: ClusterNumber) {
		self.set(last, FatEntry::Link(next.get()));
	}

	/// Frees the chain from `first`, as allocsup.c FatxFreeClusters does: up to the first
	/// entry that is not a data cluster. With `keep_first`, `first` becomes the end of the
	/// chain, and that end is flushed before the rest is freed. A cycle stops the walk where
	/// the driver's would not end.
	pub(crate) fn free_chain(&mut self, first: ClusterNumber, keep_first: bool) {
		let geometry = *self.volume.geometry();
		if !geometry.is_data_cluster(first) {
			return;
		}

		let mut seen = BTreeSet::from([first]);
		let mut cluster = match self.entry(first) {
			FatEntry::Link(next) => ClusterNumber::new(next),
			FatEntry::Free | FatEntry::Last => None,
		};
		if keep_first {
			self.set(first, FatEntry::Last);
			self.flush_fat();
		} else {
			self.set(first, FatEntry::Free);
		}

		while let Some(current) = cluster.filter(|cluster| geometry.is_data_cluster(*cluster) && seen.insert(*cluster))
		{
			cluster = match self.entry(current) {
				FatEntry::Link(next) => ClusterNumber::new(next),
				FatEntry::Free | FatEntry::Last => None,
			};
			self.set(current, FatEntry::Free);
		}
	}

	/// Queues writes of the FAT pages changed since the last flush, between barriers.
	pub(crate) fn flush_fat(&mut self) {
		const PAGE: u64 = METADATA_SIZE;

		if self.dirty.is_empty() {
			return;
		}

		self.barrier();
		let table = self.volume.fat().bytes();
		let entry_size = self.volume.fat().width().entry_size();
		let fat_start = self.volume.range().start + self.volume.geometry().fat_offset();
		for page in std::mem::take(&mut self.dirty) {
			let start = page * PAGE;
			let end = (start + PAGE).min(table.len() as u64);
			let mut bytes = table[start as usize..end as usize].to_vec();
			let first =
				ClusterNumber::new((start / entry_size) as u32).or(ClusterNumber::new(1)).expect("1 is a cluster");
			let last = ClusterNumber::new(((end - 1) / entry_size) as u32).expect("a page past entry 0 holds clusters");
			for (cluster, entry) in self.fat.range(first..=last) {
				let offset = (u64::from(cluster.get()) * entry_size - start) as usize;
				self.volume.fat().width().encode(*entry, &mut bytes[offset..offset + entry_size as usize]);
			}
			self.writes.push(PendingWrite::new(fat_start + start, Cow::Owned(bytes)));
		}
		self.barrier();
	}

	/// Makes the writes so far durable before any later one.
	pub(crate) fn barrier(&mut self) {
		if matches!(self.writes.last(), None | Some(PendingWrite::Flush)) {
			return;
		}

		self.writes.push(PendingWrite::Flush);
	}

	pub(crate) fn write(&mut self, offset: u64, data: Cow<'d, [u8]>) {
		self.writes.push(PendingWrite::new(offset, data));
	}

	/// Fills a new directory cluster with free records.
	pub(crate) fn initialize_directory_cluster(&mut self, cluster: ClusterNumber) -> Result<(), ChainError> {
		let range = self.volume.cluster_range(cluster)?;
		let length = (range.end - range.start) as usize;
		self.write(range.start, Cow::Borrowed(&FREE_RECORDS[..length]));

		Ok(())
	}

	/// Writes `fill` at `file_offset` of the file whose clusters are `clusters`, which cover
	/// the range.
	pub(crate) fn write_file_range(
		&mut self,
		clusters: &[ClusterNumber],
		file_offset: u64,
		fill: Fill<'d>,
	) -> Result<(), ChainError> {
		let bytes_per_cluster = self.bytes_per_cluster();
		let mut written = 0;
		while written < fill.len() {
			let offset = file_offset + written;
			let cluster = clusters[(offset / bytes_per_cluster) as usize];
			let within = offset % bytes_per_cluster;
			let range = self.volume.cluster_range(cluster)?;
			let length = (bytes_per_cluster - within).min(fill.len() - written);
			let data = match fill {
				Fill::Data(data) => Cow::Borrowed(&data[written as usize..(written + length) as usize]),
				Fill::Zeroes(_) => Cow::Borrowed(&ZEROES[..length as usize]),
			};
			self.write(range.start + within, data);
			written += length;
		}

		Ok(())
	}

	/// The writes in order, durable at the end, and the FAT entries to store in the volume
	/// once they are done.
	pub(crate) fn finish(mut self) -> (Vec<PendingWrite<'d>>, BTreeMap<ClusterNumber, FatEntry>) {
		assert!(self.dirty.is_empty(), "every FAT change is flushed");
		self.barrier();

		(self.writes, self.fat)
	}
}
