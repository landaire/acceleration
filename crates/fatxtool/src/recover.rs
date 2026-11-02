//! Recovery of a deleted file's data to a host file, read and written in bounded chunks. The
//! holes that a strategy does not read are zero in the output. The data goes to a temporary
//! file beside the output, which replaces the output only after every check passed.

use std::collections::HashMap;
use std::collections::HashSet;
use std::io;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;

use fatx::ClusterNumber;
use fatx::EntryRecord;
use fatx::Listing;
use fatx::Volume;
use fatx::forensic::recover::RecoveryAssessment;
use fatx::forensic::recover::RecoveryReader;
use fatx::forensic::recover::RecoveryStrategy;

use crate::command::normalize_path;
use crate::command::same_path;
use crate::command::walk;
use crate::describe;
use crate::error::DeletedCandidate;
use crate::error::FatxToolError;
use crate::error::OutputError;
use crate::extract::CHUNK_SIZE;
use crate::extract::read_source;
use crate::output::TemporaryOutput;
use crate::session::Session;
use crate::source::VolumeSource;

/// The data to recover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoverTarget<'a> {
	/// The deleted file at this path.
	Entry(&'a str),
	/// `size` bytes from cluster `first`.
	Clusters { first: ClusterNumber, size: u32 },
}

/// Allocated clusters with the same owner, consecutive.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnedRun<'a> {
	first: ClusterNumber,
	last: ClusterNumber,
	owner: Option<&'a str>,
}

pub fn run<S, W>(
	volume: &Volume,
	source: &S,
	session: &Session<'_>,
	target: RecoverTarget<'_>,
	strategy: RecoveryStrategy,
	output: &Path,
	out: &mut W,
) -> Result<(), FatxToolError>
where
	S: VolumeSource,
	W: Write,
{
	let listing = walk(volume, source, session)?;
	let reader = match target {
		RecoverTarget::Entry(path) => volume.recovery_reader(find_deleted(&listing, path)?, strategy, CHUNK_SIZE)?,
		RecoverTarget::Clusters { first, size } => volume.cluster_recovery_reader(first, size, strategy, CHUNK_SIZE)?,
	};

	let assessment = write_recovery(source, session, reader, output)?;

	let owners = owned_runs(volume, &listing, &assessment.plan.allocated);
	print(&assessment, &owners, output, out).map_err(FatxToolError::WriteListing)
}

/// The deleted file at `path`. Records with the same first cluster and size name the same data.
fn find_deleted<'a>(listing: &'a Listing, path: &str) -> Result<&'a EntryRecord, FatxToolError> {
	let path = normalize_path(path);
	let named: Vec<&EntryRecord> = listing
		.deleted()
		.iter()
		.filter(|deleted| same_path(&deleted.path(), &path))
		.map(|deleted| deleted.record())
		.collect();
	if named.is_empty() {
		return Err(FatxToolError::DeletedNotFound { path });
	}

	let mut seen = HashSet::new();
	let files: Vec<&EntryRecord> = named
		.iter()
		.copied()
		.filter(|record| !record.is_directory() && seen.insert((record.first_cluster(), record.size())))
		.collect();

	match files.as_slice() {
		[] => Err(FatxToolError::IsDirectory { path }),
		[record] => Ok(record),
		_ => Err(FatxToolError::AmbiguousDeleted {
			path,
			candidates: files
				.iter()
				.map(|record| DeletedCandidate { first_cluster: record.first_cluster(), size: record.size() })
				.collect(),
		}),
	}
}

/// Writes the recovered data to a temporary file, chunk by chunk, checking the source's
/// integrity after each read, and renames it to `output` once every check passed.
fn write_recovery<S>(
	source: &S,
	session: &Session<'_>,
	mut reader: RecoveryReader,
	output: &Path,
) -> Result<RecoveryAssessment, FatxToolError>
where
	S: VolumeSource,
{
	let mut temporary = TemporaryOutput::create(session.input(), output)?;
	let path = temporary.temporary_path().to_owned();
	let write_error = |source| OutputError::WriteOutput { path: path.clone(), source };

	while let Some(read) = reader.next_read() {
		let bytes = read_source(source, read.range)?;
		reader.accept(bytes.as_ref())?;
		session.check_source(source)?;

		let file = temporary.file();
		file.seek(SeekFrom::Start(read.file_offset)).map_err(write_error)?;
		file.write_all(bytes.as_ref()).map_err(write_error)?;
	}
	let assessment = reader.finish();
	temporary.file().set_len(assessment.plan.size).map_err(write_error)?;
	temporary.persist()?;

	Ok(assessment)
}

/// The live files and directories whose chains hold `allocated`, as runs.
fn owned_runs<'a>(volume: &Volume, listing: &'a Listing, allocated: &[ClusterNumber]) -> Vec<OwnedRun<'a>> {
	let wanted: HashSet<ClusterNumber> = allocated.iter().copied().collect();
	let mut owners: HashMap<ClusterNumber, &str> = HashMap::new();
	for entry in listing.entries() {
		let record = entry.record();
		let Some(first) = record.first_cluster() else {
			continue;
		};
		// A chain holds every cluster the FAT links into it. A file whose chain breaks after the
		// clusters its size needs still holds those. Other broken chains were reported by the
		// walk; their clusters stay unowned.
		let runs = match volume.chain(first) {
			Ok(runs) => runs,
			Err(_) if !record.is_directory() => match volume.file_runs(Some(first), record.size().into()) {
				Ok(runs) => runs,
				Err(_) => continue,
			},
			Err(_) => continue,
		};
		for cluster in runs.iter().flat_map(|run| run.clusters()) {
			if wanted.contains(&cluster) {
				owners.insert(cluster, entry.path());
			}
		}
	}

	let mut runs: Vec<OwnedRun<'a>> = Vec::new();
	for cluster in allocated {
		let owner = owners.get(cluster).copied();
		match runs.last_mut() {
			Some(last) if last.owner == owner && last.last.following() == Some(*cluster) => last.last = *cluster,
			_ => runs.push(OwnedRun { first: *cluster, last: *cluster, owner }),
		}
	}

	runs
}

fn print<W>(recovery: &RecoveryAssessment, owners: &[OwnedRun<'_>], output: &Path, out: &mut W) -> io::Result<()>
where
	W: Write,
{
	let plan = &recovery.plan;
	writeln!(
		out,
		"wrote {} bytes to {} with the {} strategy",
		plan.size,
		output.display(),
		describe::strategy(plan.strategy)
	)?;
	writeln!(out, "{}", describe::plan(plan))?;

	let read = recovery.read;
	if read < plan.size {
		writeln!(out, "{} bytes were not read and are zero", plan.size - read)?;
	}
	writeln!(out, "content: {}", describe::content_check(&recovery.content))?;
	writeln!(out, "confidence: {}", describe::confidence(recovery.confidence))?;

	if !owners.is_empty() {
		writeln!(out, "clusters allocated since the deletion:")?;
	}
	for run in owners {
		let owner = run.owner.unwrap_or("no live file or directory");
		if run.first == run.last {
			writeln!(out, "  {}: {owner}", run.first)?;
		} else {
			writeln!(out, "  {}..={}: {owner}", run.first, run.last)?;
		}
	}

	Ok(())
}
