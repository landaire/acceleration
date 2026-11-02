//! What a change asks for and what it did.

use crate::entry::EntryRecord;
use crate::entry::FileAttributes;
use crate::error::UpdateError;
use crate::name::FileName;
use crate::name::VolumePath;
use crate::time::Timestamp;
use crate::update::resolve::Target;

/// Where [`Request::write_file`] puts its data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
	/// Replace the file's contents, creating the file when it does not exist. An existing file
	/// loses its clusters first and gets new timestamps, as create.c FatxOverwriteExistingFile
	/// does; it keeps its attributes.
	Replace,
	/// Write after the file's last byte. The file must exist.
	Append,
	/// Write at a byte offset of an existing file. Bytes between its end and the offset become
	/// zero.
	At(u64),
}

/// Attributes and timestamps to set; `None` keeps the current value (fileinfo.c
/// FatxSetBasicInformation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EntryInfo {
	/// Only [`FileAttributes::HIDDEN`] and [`FileAttributes::DIRECTORY`]. A directory keeps its
	/// directory attribute whether or not it is given.
	pub attributes: Option<FileAttributes>,
	pub created: Option<Timestamp>,
	pub modified: Option<Timestamp>,
	pub accessed: Option<Timestamp>,
}

/// One change to a volume. Paths are absolute volume paths with `/` separators; names match
/// without regard to case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request<'d> {
	kind: RequestKind<'d>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RequestKind<'d> {
	CreateDirectory { path: VolumePath, now: Timestamp },
	WriteFile { path: VolumePath, data: &'d [u8], mode: WriteMode, now: Timestamp },
	SetLength { path: VolumePath, length: u32 },
	Remove { path: VolumePath },
	Rename { from: VolumePath, to: VolumePath },
	SetInfo { path: VolumePath, info: EntryInfo },
}

impl<'d> Request<'d> {
	/// Creates an empty directory stamped `now`.
	pub fn create_directory(path: &str, now: Timestamp) -> Result<Self, UpdateError> {
		Ok(Self { kind: RequestKind::CreateDirectory { path: entry_path(path)?, now: valid(now)? } })
	}

	/// Writes `data` to a file; see [`WriteMode`]. `now` is the creation time of a new file and
	/// the last write time of every file.
	pub fn write_file(path: &str, data: &'d [u8], mode: WriteMode, now: Timestamp) -> Result<Self, UpdateError> {
		Ok(Self { kind: RequestKind::WriteFile { path: entry_path(path)?, data, mode, now: valid(now)? } })
	}

	/// Truncates or extends a file to `length` bytes and frees the clusters past its end, as
	/// a file's end of file and then its allocation size are set. New bytes are zero.
	pub fn set_length(path: &str, length: u32) -> Result<Self, UpdateError> {
		Ok(Self { kind: RequestKind::SetLength { path: entry_path(path)?, length } })
	}

	/// Deletes a file or an empty directory.
	pub fn remove(path: &str) -> Result<Self, UpdateError> {
		Ok(Self { kind: RequestKind::Remove { path: entry_path(path)? } })
	}

	/// Renames or moves a file or directory. The target must not exist.
	pub fn rename(from: &str, to: &str) -> Result<Self, UpdateError> {
		Ok(Self { kind: RequestKind::Rename { from: entry_path(from)?, to: entry_path(to)? } })
	}

	pub fn set_info(path: &str, info: EntryInfo) -> Result<Self, UpdateError> {
		for timestamp in [info.created, info.modified, info.accessed].into_iter().flatten() {
			valid(timestamp)?;
		}

		Ok(Self { kind: RequestKind::SetInfo { path: entry_path(path)?, info } })
	}

	pub(crate) fn kind(&self) -> &RequestKind<'d> {
		&self.kind
	}

	/// The directories the change reads: the parents of the paths it names, then a directory
	/// it removes.
	pub(crate) fn targets(&self) -> Vec<Target> {
		let parent = |path: &VolumePath| Target::Directory(split(path).0);

		match &self.kind {
			RequestKind::CreateDirectory { path, .. }
			| RequestKind::WriteFile { path, .. }
			| RequestKind::SetLength { path, .. }
			| RequestKind::SetInfo { path, .. } => vec![parent(path)],
			RequestKind::Remove { path } => vec![parent(path), Target::IfDirectory(path.clone())],
			RequestKind::Rename { from, to } => vec![parent(from), parent(to)],
		}
	}
}

/// What a change did, with the records as they are now on the volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
	Created {
		path: String,
		record: EntryRecord,
	},
	Modified {
		path: String,
		record: EntryRecord,
	},
	/// The record is marked deleted and keeps its first cluster and size.
	Removed {
		path: String,
		record: EntryRecord,
	},
	Renamed {
		from: String,
		to: String,
		record: EntryRecord,
	},
}

impl Change {
	/// The path of the entry after the change.
	pub fn path(&self) -> &str {
		match self {
			Self::Created { path, .. } | Self::Modified { path, .. } | Self::Removed { path, .. } => path,
			Self::Renamed { to, .. } => to,
		}
	}

	pub const fn record(&self) -> &EntryRecord {
		match self {
			Self::Created { record, .. }
			| Self::Modified { record, .. }
			| Self::Removed { record, .. }
			| Self::Renamed { record, .. } => record,
		}
	}
}

/// The parent and the name of a path other than the root.
pub(crate) fn split(path: &VolumePath) -> (VolumePath, &FileName) {
	path.split_last().expect("request paths name an entry")
}

fn entry_path(path: &str) -> Result<VolumePath, UpdateError> {
	let path = VolumePath::parse(path)?;
	if path.is_root() {
		return Err(UpdateError::RootDirectory);
	}

	Ok(path)
}

fn valid(timestamp: Timestamp) -> Result<Timestamp, UpdateError> {
	if !timestamp.is_valid() {
		return Err(UpdateError::InvalidTimestamp { raw: timestamp.raw() });
	}

	Ok(timestamp)
}
