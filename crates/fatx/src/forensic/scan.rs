//! A scan of a volume's free clusters for orphaned directory records and known file signatures,
//! and an assessment of every deleted file in a listing. Free clusters are read in bounded
//! chunks; only findings are kept.

use std::collections::HashMap;
use std::ops::Range;

use crate::entry::EntryRecord;
use crate::error::ChainError;
use crate::error::FatxError;
use crate::fat::ClusterNumber;
use crate::forensic::Confidence;
use crate::forensic::content;
use crate::forensic::content::ContentCheck;
use crate::forensic::content::ContentMatch;
use crate::forensic::content::HEAD_SIZE;
use crate::forensic::matching::EntryMatch;
use crate::forensic::matching::EntryReference;
use crate::forensic::matching::match_candidate;
use crate::forensic::orphan::OrphanEntry;
use crate::forensic::orphan::OrphanOrigin;
use crate::forensic::orphan::assess_slots;
use crate::forensic::recover::RecoveryPlan;
use crate::forensic::recover::RecoveryStrategy;
use crate::forensic::recover::assess;
use crate::io;
use crate::io::AsyncReadAt;
use crate::io::ReadAt;
use crate::machine::Machine;
use crate::machine::Step;
use crate::machine::expect_len;
use crate::volume::Volume;
use crate::walk::Listing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanOptions {
	read_size: u64,
}

impl ScanOptions {
	pub const fn new() -> Self {
		Self { read_size: 0x10_0000 }
	}

	/// Largest read, in bytes. A read always covers at least one cluster.
	pub const fn with_read_size(self, read_size: u64) -> Self {
		Self { read_size }
	}

	pub const fn read_size(&self) -> u64 {
		self.read_size
	}
}

impl Default for ScanOptions {
	fn default() -> Self {
		Self::new()
	}
}

/// A free cluster that starts with a known file signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarvedCandidate {
	pub cluster: ClusterNumber,
	/// Offset of the cluster in the source.
	pub offset: u64,
	pub content: ContentMatch,
	/// Whether a deleted or orphaned record starts at this cluster.
	pub explained: bool,
}

/// What the volume says about recovering a deleted file with [`RecoveryStrategy::Contiguous`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedAssessment {
	/// Index into [`Listing::deleted`].
	pub entry: usize,
	pub plan: Result<RecoveryPlan, ChainError>,
	/// The first cluster's bytes checked against the record; `None` when the first cluster is
	/// not a free cluster in the image.
	pub content: Option<ContentCheck>,
	pub confidence: Confidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ForensicReport {
	pub scanned_clusters: u64,
	pub deleted: Vec<DeletedAssessment>,
	pub orphans: Vec<OrphanEntry>,
	pub carved: Vec<CarvedCandidate>,
	pub matches: Vec<EntryMatch>,
}

impl Volume {
	/// A machine that scans the free clusters and assesses the deleted files of `listing`.
	pub fn scanner<'a>(&'a self, listing: &'a Listing, options: ScanOptions) -> ScanVolume<'a> {
		ScanVolume::new(self, listing, options)
	}

	pub fn scan<R>(&self, source: &R, listing: &Listing, options: ScanOptions) -> Result<ForensicReport, FatxError>
	where
		R: ReadAt,
	{
		io::drive(self.scanner(listing, options), source)
	}

	pub async fn scan_async<R>(
		&self,
		source: &R,
		listing: &Listing,
		options: ScanOptions,
	) -> Result<ForensicReport, FatxError>
	where
		R: AsyncReadAt,
	{
		io::drive_async(self.scanner(listing, options), source).await
	}
}

#[derive(Debug)]
struct ChunkRead {
	first: ClusterNumber,
	count: u32,
	range: Range<u64>,
}

/// Reads runs of free clusters in order, skipping clusters already read as deleted directories.
#[derive(Debug)]
pub struct ScanVolume<'a> {
	volume: &'a Volume,
	listing: &'a Listing,
	clusters_per_read: u32,
	/// The next cluster to consider.
	cursor: u32,
	/// Deleted files by first cluster, for those whose first cluster is a free cluster.
	interest: HashMap<ClusterNumber, Vec<usize>>,
	contents: HashMap<usize, ContentCheck>,
	requested: Option<ChunkRead>,
	report: Option<ForensicReport>,
}

impl<'a> ScanVolume<'a> {
	fn new(volume: &'a Volume, listing: &'a Listing, options: ScanOptions) -> Self {
		let cluster_size = u64::from(volume.geometry().bytes_per_cluster());
		let clusters_per_read = (options.read_size / cluster_size).clamp(1, u64::from(u32::MAX)) as u32;

		let mut interest: HashMap<ClusterNumber, Vec<usize>> = HashMap::new();
		for (index, deleted) in listing.deleted().iter().enumerate() {
			let record = deleted.record();
			if record.is_directory() || record.size() == 0 {
				continue;
			}
			if let Some(first) = record.first_cluster() {
				interest.entry(first).or_default().push(index);
			}
		}

		Self {
			volume,
			listing,
			clusters_per_read,
			cursor: 1,
			interest,
			contents: HashMap::new(),
			requested: None,
			report: Some(ForensicReport::default()),
		}
	}

	fn scannable(&self, cluster: u32) -> Option<ClusterNumber> {
		let cluster = ClusterNumber::new(cluster)?;
		let recovered = self.listing.recovered_clusters().binary_search(&cluster).is_ok();

		(self.volume.fat().is_free(cluster) && !recovered).then_some(cluster)
	}

	/// The next run of scannable clusters, at most one read long.
	fn next_chunk(&mut self) -> Option<ChunkRead> {
		let limit = self.volume.image_cluster_count();
		let first = loop {
			if self.cursor > limit {
				return None;
			}
			let cluster = self.cursor;
			self.cursor += 1;
			if let Some(first) = self.scannable(cluster) {
				break first;
			}
		};

		let mut count = 1;
		while count < self.clusters_per_read && self.cursor <= limit && self.scannable(self.cursor).is_some() {
			count += 1;
			self.cursor += 1;
		}

		let start = self.volume.cluster_range(first).expect("scanned clusters are in the image").start;
		let length = u64::from(count) * u64::from(self.volume.geometry().bytes_per_cluster());

		Some(ChunkRead { first, count, range: start..start + length })
	}

	fn scan_cluster(&mut self, cluster: ClusterNumber, offset: u64, bytes: &[u8]) {
		let head = &bytes[..bytes.len().min(HEAD_SIZE)];
		let report = self.report.as_mut().expect("report present until the scan completes");

		if let Some(content) = content::identify(head) {
			report.carved.push(CarvedCandidate { cluster, offset, content, explained: false });
		}

		for index in self.interest.get(&cluster).into_iter().flatten() {
			let record = self.listing.deleted()[*index].record();
			let size = record.size() as usize;
			let check = content::check(
				content::kind_from_name(record.name()),
				u64::from(record.size()),
				&head[..head.len().min(size)],
			);
			self.contents.insert(*index, check);
		}

		report.orphans.extend(assess_slots(self.volume, cluster, bytes, 0, OrphanOrigin::FreeCluster));
	}

	fn finish(&mut self) -> ForensicReport {
		let mut report = self.report.take().expect("ScanVolume stepped after completion");

		for (index, deleted) in self.listing.deleted().iter().enumerate() {
			let record = deleted.record();
			if record.is_directory() || record.size() == 0 {
				continue;
			}

			let plan = self.volume.contiguous_plan(record.first_cluster(), record.size(), RecoveryStrategy::Contiguous);
			let content = self.contents.remove(&index);
			let confidence = match (&plan, &content) {
				(Ok(plan), Some(content)) => assess(plan, content),
				_ => Confidence::Low,
			};
			report.deleted.push(DeletedAssessment { entry: index, plan, content, confidence });
		}

		let mut by_cluster: HashMap<ClusterNumber, Vec<(EntryReference, &EntryRecord)>> = HashMap::new();
		for (cluster, indices) in &self.interest {
			for index in indices {
				by_cluster
					.entry(*cluster)
					.or_default()
					.push((EntryReference::Deleted(*index), self.listing.deleted()[*index].record()));
			}
		}
		for (index, orphan) in report.orphans.iter().enumerate() {
			if let Some(first) = orphan.record.first_cluster() {
				by_cluster.entry(first).or_default().push((EntryReference::FreeOrphan(index), &orphan.record));
			}
		}
		for (index, slack) in self.listing.slack().iter().enumerate() {
			if let Some(first) = slack.orphan().record.first_cluster() {
				by_cluster.entry(first).or_default().push((EntryReference::SlackOrphan(index), &slack.orphan().record));
			}
		}

		let mut matches = Vec::new();
		for (index, candidate) in report.carved.iter().enumerate() {
			for (entry, record) in by_cluster.get(&candidate.cluster).into_iter().flatten() {
				if !record.is_directory() {
					matches.push(match_candidate(index, candidate, *entry, record));
				}
			}
		}
		matches.sort_by_key(|found| found.candidate);
		for found in &matches {
			report.carved[found.candidate].explained = true;
		}
		report.matches = matches;

		report
	}
}

impl Machine for ScanVolume<'_> {
	type Output = ForensicReport;
	type Error = FatxError;

	fn step(&mut self) -> Result<Step<Self::Output>, FatxError> {
		assert!(self.requested.is_none(), "ScanVolume stepped before the requested read was fed");
		assert!(self.report.is_some(), "ScanVolume stepped after completion");

		let Some(chunk) = self.next_chunk() else {
			return Ok(Step::Done(self.finish()));
		};
		let range = chunk.range.clone();
		self.requested = Some(chunk);

		Ok(Step::Read(range))
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), FatxError> {
		let chunk = self.requested.take().expect("ScanVolume fed without a requested read");
		expect_len(&chunk.range, bytes)?;

		let cluster_size = self.volume.geometry().bytes_per_cluster() as usize;
		for (index, cluster_bytes) in bytes.chunks_exact(cluster_size).enumerate() {
			let cluster = ClusterNumber::new(chunk.first.get() + index as u32).expect("cluster numbers start at 1");
			let offset = chunk.range.start + (index * cluster_size) as u64;
			self.scan_cluster(cluster, offset, cluster_bytes);
		}
		self.report.as_mut().expect("report present until the scan completes").scanned_clusters +=
			u64::from(chunk.count);

		Ok(())
	}
}
