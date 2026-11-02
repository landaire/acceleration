//! The directory tree, one entry per line. With `--long` or for deleted entries, the columns
//! are tab-separated: type, size, first cluster, created, modified and accessed times, path,
//! and for a deleted entry the directory that held it.

use std::io::Write;

use fatx::EntryRecord;
use fatx::Volume;

use crate::command::walk;
use crate::describe;
use crate::error::FatxToolError;
use crate::session::Session;
use crate::source::VolumeSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListOptions {
	pub deleted: bool,
	pub long: bool,
}

pub fn run<S, W>(
	volume: &Volume,
	source: &S,
	session: &Session<'_>,
	options: ListOptions,
	out: &mut W,
) -> Result<(), FatxToolError>
where
	S: VolumeSource,
	W: Write,
{
	let listing = walk(volume, source, session)?;

	let mut lines: Vec<(String, String)> = Vec::new();
	for entry in listing.entries() {
		let path = display_path(entry.path(), entry.record());
		let line = if options.long { format!("{}\t{path}", columns(entry.record(), false)) } else { path };
		lines.push((entry.path().to_owned(), line));
	}
	if options.deleted {
		for deleted in listing.deleted() {
			let path = deleted.path();
			let line = format!(
				"{}\t{}\tdeleted from {} ({})",
				columns(deleted.record(), true),
				display_path(&path, deleted.record()),
				deleted.parent(),
				describe::origin(deleted.origin()),
			);
			lines.push((path, line));
		}
	}
	// Tree order: each directory's contents follow it, before its next sibling.
	lines.sort_by(|(left, _), (right, _)| left.split('/').cmp(right.split('/')));

	lines.iter().try_for_each(|(_, line)| writeln!(out, "{line}")).map_err(FatxToolError::WriteListing)
}

/// A directory's path ends with a separator.
fn display_path(path: &str, record: &EntryRecord) -> String {
	if record.is_directory() { format!("{path}/") } else { path.to_owned() }
}

fn columns(record: &EntryRecord, deleted: bool) -> String {
	let kind = match (deleted, record.is_directory()) {
		(false, false) => "file",
		(false, true) => "dir",
		(true, false) => "deleted-file",
		(true, true) => "deleted-dir",
	};

	format!(
		"{kind}\t{}\t{}\t{}\t{}\t{}",
		record.size(),
		describe::cluster(record.first_cluster()),
		record.created(),
		record.modified(),
		record.accessed(),
	)
}
