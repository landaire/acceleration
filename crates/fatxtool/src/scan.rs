//! The forensic scan: deleted files, orphaned directory records, carved signatures, and the
//! matches between them. Every location is a cluster and a byte offset in the source.

use std::io;
use std::io::Write;

use fatx::EntryRecord;
use fatx::Listing;
use fatx::Volume;
use fatx::forensic::matching::EntryReference;
use fatx::forensic::orphan::OrphanEntry;
use fatx::forensic::orphan::OrphanOrigin;
use fatx::forensic::scan::ForensicReport;
use fatx::forensic::scan::ScanOptions;

use crate::command::walk;
use crate::describe;
use crate::error::FatxToolError;
use crate::session::Session;
use crate::source::VolumeSource;

pub fn run<S, W>(volume: &Volume, source: &S, session: &Session<'_>, out: &mut W) -> Result<(), FatxToolError>
where
	S: VolumeSource,
	W: Write,
{
	let listing = walk(volume, source, session)?;
	let report = volume.scan(source, &listing, ScanOptions::new())?;
	session.check_source(source)?;

	print(volume, &listing, &report, out).map_err(FatxToolError::WriteListing)
}

fn print<W>(volume: &Volume, listing: &Listing, report: &ForensicReport, out: &mut W) -> io::Result<()>
where
	W: Write,
{
	writeln!(out, "scanned {} free clusters", report.scanned_clusters)?;

	writeln!(out, "\ndeleted files: {}", report.deleted.len())?;
	for assessment in &report.deleted {
		let deleted = &listing.deleted()[assessment.entry];
		let record = deleted.record();
		let plan = match &assessment.plan {
			Ok(plan) => describe::plan(plan),
			Err(error) => format!("no plan: {error}"),
		};
		let content = match &assessment.content {
			Some(check) => describe::content_check(check),
			None => "first cluster not free in the image".to_owned(),
		};
		writeln!(
			out,
			"  {} at {}: {} bytes from cluster {}; {plan}; {content}; confidence {}",
			deleted.path(),
			record_location(volume, record),
			record.size(),
			describe::cluster(record.first_cluster()),
			describe::confidence(assessment.confidence),
		)?;
	}

	let orphan_count = report.orphans.len() + listing.slack().len();
	writeln!(out, "\norphaned records: {orphan_count}")?;
	for orphan in &report.orphans {
		print_orphan(volume, orphan, out)?;
	}
	for slack in listing.slack() {
		print_orphan(volume, slack.orphan(), out)?;
	}

	writeln!(out, "\ncarved signatures: {}", report.carved.len())?;
	for (index, candidate) in report.carved.iter().enumerate() {
		let explained = if candidate.explained { "matched" } else { "unmatched" };
		writeln!(
			out,
			"  #{index} cluster {} at {:#x}: {}; {explained}",
			candidate.cluster,
			candidate.offset,
			describe::content(&candidate.content),
		)?;
	}

	writeln!(out, "\nmatches: {}", report.matches.len())?;
	for found in &report.matches {
		let candidate = &report.carved[found.candidate];
		let entry = match found.entry {
			EntryReference::Deleted(index) => format!("deleted {}", listing.deleted()[index].path()),
			EntryReference::FreeOrphan(index) => orphan_name(&report.orphans[index]),
			EntryReference::SlackOrphan(index) => orphan_name(listing.slack()[index].orphan()),
		};
		let reasons: Vec<&str> = found.reasons.iter().map(|reason| describe::reason(*reason)).collect();
		writeln!(
			out,
			"  #{} cluster {} with {entry}: {}; confidence {}",
			found.candidate,
			candidate.cluster,
			reasons.join(", "),
			describe::confidence(found.confidence),
		)?;
	}

	Ok(())
}

fn print_orphan<W>(volume: &Volume, orphan: &OrphanEntry, out: &mut W) -> io::Result<()>
where
	W: Write,
{
	let record = &orphan.record;
	let origin = match orphan.origin {
		OrphanOrigin::FreeCluster => "free cluster".to_owned(),
		OrphanOrigin::DirectorySlack { directory } => format!("after the end of directory {directory}"),
	};
	let kind = if record.is_directory() { "directory" } else { "file" };
	let first_cluster = match orphan.first_cluster_free {
		Some(true) => "free",
		Some(false) => "allocated",
		None => "none",
	};
	let deleted = if record.is_marked_deleted() { "deleted " } else { "" };

	writeln!(
		out,
		"  {deleted}{kind} {:?} at {} ({origin}): {} bytes from cluster {} ({first_cluster}); modified {}; {} \
		 neighbors; confidence {}",
		record.name(),
		record_location(volume, record),
		record.size(),
		describe::cluster(record.first_cluster()),
		record.modified(),
		orphan.neighbors,
		describe::confidence(orphan.confidence),
	)
}

fn orphan_name(orphan: &OrphanEntry) -> String {
	format!("orphaned {:?} at {}", orphan.record.name(), orphan.record.location())
}

/// A record's cluster and offset, and its offset in the source.
fn record_location(volume: &Volume, record: &EntryRecord) -> String {
	let location = record.location();
	match volume.cluster_range(location.cluster) {
		Ok(range) => format!("{location} ({:#x})", range.start + u64::from(location.offset)),
		Err(_) => location.to_string(),
	}
}
