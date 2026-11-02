use std::fmt::Debug;
use std::io::Cursor;
use std::time::SystemTime;

use fskit::Metadata;
use fskit::VfsEntry;
use fskit::VfsTree;
use vfs::FileSystem;
use vfs::VfsError;
use vfs::VfsMetadata;
use vfs::error::VfsErrorKind;

use crate::entry::EntryRecord;
use crate::error::FatxError;
use crate::io::ReadAt;
use crate::time::Timestamp;
use crate::volume::Volume;
use crate::walk::Listing;

#[derive(Debug, Clone)]
pub struct FatxFileMeta {
	pub record: EntryRecord,
}

impl Metadata for FatxFileMeta {
	fn len(&self) -> u64 {
		u64::from(self.record.size())
	}

	fn created(&self) -> Option<SystemTime> {
		system_time(self.record.created())
	}

	fn modified(&self) -> Option<SystemTime> {
		system_time(self.record.modified())
	}

	fn accessed(&self) -> Option<SystemTime> {
		system_time(self.record.accessed())
	}
}

/// A volume's live files and directories as a read-only [`vfs::FileSystem`].
#[derive(Debug)]
pub struct FatxVfs<S> {
	source: S,
	volume: Volume,
	listing: Listing,
	tree: VfsTree<FatxFileMeta>,
}

impl<S> FatxVfs<S> {
	pub fn new(source: S, volume: Volume, listing: Listing) -> Self {
		let mut builder = VfsTree::builder();
		for entry in listing.entries() {
			let meta = FatxFileMeta { record: entry.record().clone() };
			builder = if entry.record().is_directory() {
				builder.insert_dir(entry.path(), Some(meta))
			} else {
				builder.insert(entry.path(), meta)
			};
		}
		let tree = builder.build();

		Self { source, volume, listing, tree }
	}

	pub fn source(&self) -> &S {
		&self.source
	}

	pub fn volume(&self) -> &Volume {
		&self.volume
	}

	pub fn listing(&self) -> &Listing {
		&self.listing
	}

	pub fn tree(&self) -> &VfsTree<FatxFileMeta> {
		&self.tree
	}

	fn file_record(&self, path: &str) -> vfs::VfsResult<&EntryRecord> {
		match self.tree.vfs_lookup(path)? {
			VfsEntry::File(meta) => Ok(&meta.record),
			VfsEntry::Directory { .. } => Err(VfsErrorKind::Other("not a file".into()).into()),
		}
	}
}

fn system_time(timestamp: Timestamp) -> Option<SystemTime> {
	timestamp.unix_seconds().map(fskit::system_time_from_unix)
}

fn to_vfs_error(error: FatxError) -> VfsError {
	VfsErrorKind::IoError(std::io::Error::other(error)).into()
}

impl<S> FileSystem for FatxVfs<S>
where
	S: ReadAt + Debug + Send + Sync + 'static,
{
	fn read_dir(&self, path: &str) -> vfs::VfsResult<Box<dyn Iterator<Item = String> + Send>> {
		self.tree.vfs_read_dir(path)
	}

	fn open_file(&self, path: &str) -> vfs::VfsResult<Box<dyn vfs::SeekAndRead + Send>> {
		let record = self.file_record(path)?;
		let data = self.volume.read_file(&self.source, record).map_err(to_vfs_error)?;
		Ok(Box::new(Cursor::new(data)))
	}

	fn metadata(&self, path: &str) -> vfs::VfsResult<VfsMetadata> {
		self.tree.vfs_metadata(path)
	}

	fn exists(&self, path: &str) -> vfs::VfsResult<bool> {
		self.tree.vfs_exists(path)
	}

	fskit::read_only_fs_stubs!();
}

#[cfg(feature = "async-vfs")]
const _: () = {
	use async_trait::async_trait;
	use vfs::async_vfs::AsyncFileSystem;

	use crate::io::AsyncReadAt;

	#[async_trait]
	impl<S> AsyncFileSystem for FatxVfs<S>
	where
		S: AsyncReadAt + Debug + Send + Sync + 'static,
	{
		async fn read_dir(&self, path: &str) -> vfs::VfsResult<Box<dyn Unpin + futures::Stream<Item = String> + Send>> {
			self.tree.async_vfs_read_dir(path)
		}

		async fn open_file(&self, path: &str) -> vfs::VfsResult<Box<dyn vfs::async_vfs::SeekAndRead + Send + Unpin>> {
			let record = self.file_record(path)?;
			let data = self.volume.read_file_async(&self.source, record).await.map_err(to_vfs_error)?;
			Ok(Box::new(futures::io::Cursor::new(data)))
		}

		async fn metadata(&self, path: &str) -> vfs::VfsResult<VfsMetadata> {
			self.tree.vfs_metadata(path)
		}

		async fn exists(&self, path: &str) -> vfs::VfsResult<bool> {
			self.tree.vfs_exists(path)
		}

		async fn create_dir(&self, _path: &str) -> vfs::VfsResult<()> {
			Err(VfsErrorKind::NotSupported.into())
		}

		async fn create_file(&self, _path: &str) -> vfs::VfsResult<Box<dyn futures::io::AsyncWrite + Send + Unpin>> {
			Err(VfsErrorKind::NotSupported.into())
		}

		async fn append_file(&self, _path: &str) -> vfs::VfsResult<Box<dyn futures::io::AsyncWrite + Send + Unpin>> {
			Err(VfsErrorKind::NotSupported.into())
		}

		async fn remove_file(&self, _path: &str) -> vfs::VfsResult<()> {
			Err(VfsErrorKind::NotSupported.into())
		}

		async fn remove_dir(&self, _path: &str) -> vfs::VfsResult<()> {
			Err(VfsErrorKind::NotSupported.into())
		}

		async fn copy_file(&self, _src: &str, _dest: &str) -> vfs::VfsResult<()> {
			Err(VfsErrorKind::NotSupported.into())
		}

		async fn move_file(&self, _src: &str, _dest: &str) -> vfs::VfsResult<()> {
			Err(VfsErrorKind::NotSupported.into())
		}

		async fn move_dir(&self, _src: &str, _dest: &str) -> vfs::VfsResult<()> {
			Err(VfsErrorKind::NotSupported.into())
		}
	}
};
