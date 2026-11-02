//! Best-effort recovery of a deleted file's data. Deletion frees the file's chain (fatsup.c
//! FatxDeleteFile), so only the first cluster and the size remain; the strategies assume the
//! file was stored in consecutive clusters.

use std::collections::VecDeque;
use std::ops::Range;

use crate::entry::EntryRecord;
use crate::error::ChainError;
use crate::error::FatxError;
use crate::fat::ClusterNumber;
use crate::fat::ClusterRun;
use crate::forensic::Confidence;
use crate::forensic::content;
use crate::forensic::content::Agreement;
use crate::forensic::content::ContentCheck;
use crate::forensic::content::ContentKind;
use crate::forensic::content::HEAD_SIZE;
use crate::io;
use crate::io::AsyncReadAt;
use crate::io::ReadAt;
use crate::machine::Machine;
use crate::machine::Step;
use crate::machine::expect_len;
use crate::volume::MAXIMUM_READ_SIZE;
use crate::volume::Volume;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryStrategy {
	/// `size` bytes from the clusters that follow the first cluster, whether or not the FAT has
	/// given them to another file since.
	Contiguous,
	/// The same clusters, but only those the FAT marks free. Clusters allocated to live files
	/// are reported, not read.
	ContiguousFree,
}

/// The clusters a strategy would read, and what the FAT says about them now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryPlan {
	pub strategy: RecoveryStrategy,
	pub size: u64,
	/// The clusters the size needs, from the first cluster; `None` for an empty file.
	pub clusters: Option<ClusterRun>,
	/// Needed clusters the FAT has allocated since the deletion.
	pub allocated: Vec<ClusterNumber>,
	/// Needed clusters past the end of the volume or the image.
	pub missing: u32,
}

impl RecoveryPlan {
	/// Whether every needed cluster is in the image and free.
	pub fn is_intact(&self) -> bool {
		self.allocated.is_empty() && self.missing == 0
	}
}

/// Bytes recovered from consecutive clusters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredSegment {
	/// Offset in the deleted file.
	pub file_offset: u64,
	pub bytes: Vec<u8>,
}

/// Recovered data. The bytes are what the clusters hold now, which need not be the deleted
/// file's: see the plan, the content check and the confidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
	pub plan: RecoveryPlan,
	pub segments: Vec<RecoveredSegment>,
	pub content: ContentCheck,
	pub confidence: Confidence,
}

/// What a recovery read, without its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryAssessment {
	pub plan: RecoveryPlan,
	/// The recovered file's first bytes checked against the record.
	pub content: ContentCheck,
	pub confidence: Confidence,
	/// Bytes read; the rest of the plan's size is clusters the strategy does not read.
	pub read: u64,
}

/// One read of a recovery: `range` of the source holds the bytes at `file_offset` of the
/// recovered file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryRead {
	pub file_offset: u64,
	pub range: Range<u64>,
}

impl Volume {
	/// The clusters `strategy` would read for a deleted file's record.
	pub fn recovery_plan(&self, record: &EntryRecord, strategy: RecoveryStrategy) -> Result<RecoveryPlan, FatxError> {
		if record.is_directory() {
			return Err(FatxError::NotAFile { name: record.name().to_owned() });
		}

		Ok(self.contiguous_plan(record.first_cluster(), record.size(), strategy)?)
	}

	/// The clusters `strategy` would read for `size` bytes from `first`.
	pub fn contiguous_plan(
		&self,
		first: Option<ClusterNumber>,
		size: u32,
		strategy: RecoveryStrategy,
	) -> Result<RecoveryPlan, ChainError> {
		let size = u64::from(size);
		// A u32 size over clusters of at least 0x1000 bytes always fits a u32 count.
		let needed = size.div_ceil(u64::from(self.geometry().bytes_per_cluster())) as u32;
		if needed == 0 {
			return Ok(RecoveryPlan { strategy, size, clusters: None, allocated: Vec::new(), missing: 0 });
		}

		let first = first.ok_or(ChainError::MissingFirstCluster)?;
		if !self.geometry().is_data_cluster(first) {
			return Err(ChainError::InvalidCluster { cluster: first });
		}

		let run = ClusterRun::new(first, needed).expect("data clusters end far below the largest cluster number");
		let mut allocated = Vec::new();
		let mut missing = 0;
		for cluster in run.clusters() {
			if !self.is_in_image(cluster) {
				missing += 1;
			} else if !self.fat().is_free(cluster) {
				allocated.push(cluster);
			}
		}

		Ok(RecoveryPlan { strategy, size, clusters: Some(run), allocated, missing })
	}

	/// A machine that recovers a deleted file's data with `strategy`, holding all of it.
	pub fn recoverer(&self, record: &EntryRecord, strategy: RecoveryStrategy) -> Result<RecoverDeleted, FatxError> {
		Ok(RecoverDeleted::new(self.recovery_reader(record, strategy, MAXIMUM_READ_SIZE)?))
	}

	/// A machine that recovers `size` bytes from the consecutive clusters from `first`, for data
	/// that no directory record describes. The content check has no file name to compare with.
	pub fn cluster_recoverer(
		&self,
		first: ClusterNumber,
		size: u32,
		strategy: RecoveryStrategy,
	) -> Result<RecoverDeleted, FatxError> {
		Ok(RecoverDeleted::new(self.cluster_recovery_reader(first, size, strategy, MAXIMUM_READ_SIZE)?))
	}

	/// The reads of a deleted file's recovery, each at most `chunk_size` bytes and never more
	/// than [`MAXIMUM_READ_SIZE`], for a caller that writes the data out as it reads it.
	pub fn recovery_reader(
		&self,
		record: &EntryRecord,
		strategy: RecoveryStrategy,
		chunk_size: u64,
	) -> Result<RecoveryReader, FatxError> {
		let plan = self.recovery_plan(record, strategy)?;

		self.plan_reader(plan, content::kind_from_name(record.name()), chunk_size)
	}

	/// The reads of a recovery of `size` bytes from the consecutive clusters from `first`, each at
	/// most `chunk_size` bytes and never more than [`MAXIMUM_READ_SIZE`].
	pub fn cluster_recovery_reader(
		&self,
		first: ClusterNumber,
		size: u32,
		strategy: RecoveryStrategy,
		chunk_size: u64,
	) -> Result<RecoveryReader, FatxError> {
		let plan = self.contiguous_plan(Some(first), size, strategy)?;

		self.plan_reader(plan, None, chunk_size)
	}

	fn plan_reader(
		&self,
		plan: RecoveryPlan,
		expected: Option<ContentKind>,
		chunk_size: u64,
	) -> Result<RecoveryReader, FatxError> {
		assert!(chunk_size > 0, "a recovery reads at least one byte at a time");
		let reads = self.recovery_reads(&plan, chunk_size.min(MAXIMUM_READ_SIZE))?;

		Ok(RecoveryReader { expected, plan, reads, requested: None, head: Vec::new(), read: 0 })
	}

	pub fn recover<R>(
		&self,
		source: &R,
		record: &EntryRecord,
		strategy: RecoveryStrategy,
	) -> Result<Recovery, FatxError>
	where
		R: ReadAt,
	{
		io::drive(self.recoverer(record, strategy)?, source)
	}

	pub async fn recover_async<R>(
		&self,
		source: &R,
		record: &EntryRecord,
		strategy: RecoveryStrategy,
	) -> Result<Recovery, FatxError>
	where
		R: AsyncReadAt,
	{
		io::drive_async(self.recoverer(record, strategy)?, source).await
	}

	pub fn recover_clusters<R>(
		&self,
		source: &R,
		first: ClusterNumber,
		size: u32,
		strategy: RecoveryStrategy,
	) -> Result<Recovery, FatxError>
	where
		R: ReadAt,
	{
		io::drive(self.cluster_recoverer(first, size, strategy)?, source)
	}

	pub async fn recover_clusters_async<R>(
		&self,
		source: &R,
		first: ClusterNumber,
		size: u32,
		strategy: RecoveryStrategy,
	) -> Result<Recovery, FatxError>
	where
		R: AsyncReadAt,
	{
		io::drive_async(self.cluster_recoverer(first, size, strategy)?, source).await
	}

	/// The reads of a plan: runs of readable clusters, split into `chunk_size`-byte reads, with
	/// their offsets in the file.
	fn recovery_reads(&self, plan: &RecoveryPlan, chunk_size: u64) -> Result<VecDeque<RecoveryRead>, FatxError> {
		let mut runs: Vec<RecoveryRead> = Vec::new();
		let Some(run) = plan.clusters else {
			return Ok(VecDeque::new());
		};

		let cluster_size = u64::from(self.geometry().bytes_per_cluster());
		for (index, cluster) in run.clusters().enumerate() {
			let readable = self.is_in_image(cluster)
				&& match plan.strategy {
					RecoveryStrategy::Contiguous => true,
					RecoveryStrategy::ContiguousFree => self.fat().is_free(cluster),
				};
			if !readable {
				continue;
			}

			let file_offset = index as u64 * cluster_size;
			let length = cluster_size.min(plan.size - file_offset);
			let start = self.cluster_range(cluster)?.start;
			let range = start..start + length;
			match runs.last_mut() {
				Some(last) if last.range.end == range.start => last.range.end = range.end,
				_ => runs.push(RecoveryRead { file_offset, range }),
			}
		}

		let mut reads = VecDeque::new();
		for run in runs {
			let mut offset = 0;
			let length = run.range.end - run.range.start;
			while offset < length {
				let chunk = chunk_size.min(length - offset);
				let start = run.range.start + offset;
				reads.push_back(RecoveryRead { file_offset: run.file_offset + offset, range: start..start + chunk });
				offset += chunk;
			}
		}

		Ok(reads)
	}
}

/// How strongly a plan and a content check support recovered data.
pub fn assess(plan: &RecoveryPlan, content: &ContentCheck) -> Confidence {
	let contradicted = content.kind == Agreement::Disagrees || content.size == Agreement::Disagrees;
	let supported = content.kind == Agreement::Agrees || content.size == Agreement::Agrees;

	match (plan.is_intact(), contradicted, supported) {
		(true, false, true) => Confidence::High,
		(true, false, false) => Confidence::Medium,
		_ => Confidence::Low,
	}
}

/// The reads of a recovery plan. The caller reads each [`RecoveryRead`] from the source, passes
/// its bytes to [`RecoveryReader::accept`] and stores them; the reader keeps only the first
/// [`HEAD_SIZE`] bytes of the file, for the content check.
#[derive(Debug)]
pub struct RecoveryReader {
	/// The type the record's file name implies, if a record names the data.
	expected: Option<ContentKind>,
	plan: RecoveryPlan,
	reads: VecDeque<RecoveryRead>,
	requested: Option<RecoveryRead>,
	head: Vec<u8>,
	read: u64,
}

impl RecoveryReader {
	pub const fn plan(&self) -> &RecoveryPlan {
		&self.plan
	}

	/// The next read, or `None` once every readable cluster was read.
	pub fn next_read(&mut self) -> Option<RecoveryRead> {
		assert!(self.requested.is_none(), "RecoveryReader asked for a read before the last one was accepted");

		let read = self.reads.pop_front()?;
		self.requested = Some(read.clone());

		Some(read)
	}

	/// Takes the bytes of the read [`Self::next_read`] returned.
	pub fn accept(&mut self, bytes: &[u8]) -> Result<(), FatxError> {
		let read = self.requested.take().expect("RecoveryReader accepted bytes without a read");
		expect_len(&read.range, bytes)?;

		// The head is the file's first bytes, so only a read that continues it extends it.
		let head_length = self.head.len() as u64;
		if read.file_offset == head_length && head_length < HEAD_SIZE as u64 {
			let take = bytes.len().min(HEAD_SIZE - self.head.len());
			self.head.extend_from_slice(&bytes[..take]);
		}
		self.read += bytes.len() as u64;

		Ok(())
	}

	/// Bytes that the pending reads add directly after file offset `end`, so a caller can size a
	/// segment once instead of growing it chunk by chunk.
	fn continuing_length(&self, mut end: u64) -> u64 {
		let start = end;
		for read in &self.reads {
			if read.file_offset != end {
				break;
			}
			end += read.range.end - read.range.start;
		}
		end - start
	}

	/// The plan, and the content check of the recovered file's first bytes.
	pub fn finish(self) -> RecoveryAssessment {
		assert!(self.requested.is_none() && self.reads.is_empty(), "RecoveryReader finished before its last read");

		let content = content::check(self.expected, self.plan.size, &self.head);
		let confidence = assess(&self.plan, &content);

		RecoveryAssessment { plan: self.plan, content, confidence, read: self.read }
	}
}

/// Reads the clusters of a recovery plan and holds their bytes.
#[derive(Debug)]
pub struct RecoverDeleted {
	reader: Option<RecoveryReader>,
	segments: Vec<RecoveredSegment>,
	requested: Option<u64>,
}

impl RecoverDeleted {
	fn new(reader: RecoveryReader) -> Self {
		Self { reader: Some(reader), segments: Vec::new(), requested: None }
	}
}

impl Machine for RecoverDeleted {
	type Output = Recovery;
	type Error = FatxError;

	fn step(&mut self) -> Result<Step<Self::Output>, FatxError> {
		assert!(self.requested.is_none(), "RecoverDeleted stepped before the requested read was fed");
		let reader = self.reader.as_mut().expect("RecoverDeleted stepped after completion");

		if let Some(read) = reader.next_read() {
			self.requested = Some(read.file_offset);
			return Ok(Step::Read(read.range));
		}

		let assessment = self.reader.take().expect("checked above").finish();
		let segments = std::mem::take(&mut self.segments);

		Ok(Step::Done(Recovery {
			plan: assessment.plan,
			segments,
			content: assessment.content,
			confidence: assessment.confidence,
		}))
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), FatxError> {
		let file_offset = self.requested.take().expect("RecoverDeleted fed without a requested read");
		let reader = self.reader.as_mut().expect("reader present while a read is requested");
		reader.accept(bytes)?;
		match self.segments.last_mut() {
			Some(last) if last.file_offset + last.bytes.len() as u64 == file_offset => {
				last.bytes.extend_from_slice(bytes)
			}
			_ => {
				let end = file_offset + bytes.len() as u64;
				let capacity = bytes.len() as u64 + reader.continuing_length(end);
				let mut segment = Vec::with_capacity(capacity as usize);
				segment.extend_from_slice(bytes);
				self.segments.push(RecoveredSegment { file_offset, bytes: segment });
			}
		}

		Ok(())
	}
}
