use std::borrow::Cow;
use std::fmt::Debug;
use std::io::Cursor;

use fskit::Metadata;
use fskit::VfsEntry;
use fskit::VfsTree;
use vfs::FileSystem;
use vfs::VfsError;
use vfs::VfsMetadata;
use vfs::error::VfsErrorKind;

use crate::checked::IntegrityPolicy;
use crate::error::SfcxError;
use crate::fs::DirectoryEntry;
use crate::fs::Filesystem;
use crate::image::FlashImage;
use crate::io::ReadAt;

#[derive(Debug, Clone)]
pub struct SfcxFileMeta {
	pub entry: DirectoryEntry,
}

impl Metadata for SfcxFileMeta {
	fn len(&self) -> u64 {
		self.entry.size()
	}
}

/// The flash file system as a read-only [`vfs::FileSystem`]. A file that fails an integrity
/// check opens or fails as `policy` says.
#[derive(Debug)]
pub struct SfcxVfs<S> {
	source: S,
	image: FlashImage,
	filesystem: Filesystem,
	policy: IntegrityPolicy,
	tree: VfsTree<SfcxFileMeta>,
}

impl<S> SfcxVfs<S> {
	pub fn new(source: S, image: FlashImage, filesystem: Filesystem, policy: IntegrityPolicy) -> Self {
		let mut builder = VfsTree::builder();
		for entry in filesystem.entries() {
			builder = builder.insert(entry.name(), SfcxFileMeta { entry: entry.clone() });
		}
		let tree = builder.build();

		Self { source, image, filesystem, policy, tree }
	}

	pub fn source(&self) -> &S {
		&self.source
	}

	pub fn image(&self) -> &FlashImage {
		&self.image
	}

	pub fn filesystem(&self) -> &Filesystem {
		&self.filesystem
	}

	pub fn tree(&self) -> &VfsTree<SfcxFileMeta> {
		&self.tree
	}

	/// The path of a root file with its stored case. The flash file system matches names
	/// without regard to ASCII case.
	fn stored_path<'p>(&self, path: &'p str) -> Cow<'p, str> {
		let name = path.strip_prefix('/').unwrap_or(path);
		match self.filesystem.find(name) {
			Some(entry) if entry.name() != name => Cow::Owned(format!("/{}", entry.name())),
			_ => Cow::Borrowed(path),
		}
	}

	fn file_entry(&self, path: &str) -> vfs::VfsResult<&DirectoryEntry> {
		match self.tree.vfs_lookup(&self.stored_path(path))? {
			VfsEntry::File(meta) => Ok(&meta.entry),
			VfsEntry::Directory { .. } => Err(VfsErrorKind::Other("not a file".into()).into()),
		}
	}
}

fn to_vfs_error(error: SfcxError) -> VfsError {
	VfsErrorKind::IoError(std::io::Error::other(error)).into()
}

impl<S> FileSystem for SfcxVfs<S>
where
	S: ReadAt + Debug + Send + Sync + 'static,
{
	fn read_dir(&self, path: &str) -> vfs::VfsResult<Box<dyn Iterator<Item = String> + Send>> {
		self.tree.vfs_read_dir(path)
	}

	fn open_file(&self, path: &str) -> vfs::VfsResult<Box<dyn vfs::SeekAndRead + Send>> {
		let entry = self.file_entry(path)?;
		let data = self.image.read_file(&self.source, &self.filesystem, entry).map_err(to_vfs_error)?;
		let data = data.resolve(self.policy).map_err(|error| VfsErrorKind::IoError(std::io::Error::other(error)))?;
		Ok(Box::new(Cursor::new(data)))
	}

	fn metadata(&self, path: &str) -> vfs::VfsResult<VfsMetadata> {
		self.tree.vfs_metadata(&self.stored_path(path))
	}

	fn exists(&self, path: &str) -> vfs::VfsResult<bool> {
		self.tree.vfs_exists(&self.stored_path(path))
	}

	fskit::read_only_fs_stubs!();
}

#[cfg(feature = "async-vfs")]
const _: () = {
	use async_trait::async_trait;
	use vfs::async_vfs::AsyncFileSystem;

	use crate::io::AsyncReadAt;

	#[async_trait]
	impl<S> AsyncFileSystem for SfcxVfs<S>
	where
		S: AsyncReadAt + Debug + Send + Sync + 'static,
	{
		async fn read_dir(&self, path: &str) -> vfs::VfsResult<Box<dyn Unpin + futures::Stream<Item = String> + Send>> {
			self.tree.async_vfs_read_dir(path)
		}

		async fn open_file(&self, path: &str) -> vfs::VfsResult<Box<dyn vfs::async_vfs::SeekAndRead + Send + Unpin>> {
			let entry = self.file_entry(path)?;
			let data = self.image.read_file_async(&self.source, &self.filesystem, entry).await.map_err(to_vfs_error)?;
			let data =
				data.resolve(self.policy).map_err(|error| VfsErrorKind::IoError(std::io::Error::other(error)))?;
			Ok(Box::new(futures::io::Cursor::new(data)))
		}

		async fn metadata(&self, path: &str) -> vfs::VfsResult<VfsMetadata> {
			self.tree.vfs_metadata(&self.stored_path(path))
		}

		async fn exists(&self, path: &str) -> vfs::VfsResult<bool> {
			self.tree.vfs_exists(&self.stored_path(path))
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
