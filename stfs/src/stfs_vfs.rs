use std::fmt::Debug;
use std::io::Cursor;

use fskit::Metadata;
use fskit::VfsEntry;
use fskit::VfsTree;
use vfs::FileSystem;
use vfs::VfsMetadata;
use vfs::error::VfsErrorKind;

use crate::file_table::StfsFileEntry;
use crate::file_table::StfsFileTable;
use crate::io::ReadAt;
use crate::package::StfsPackage;

#[derive(Debug, Clone)]
pub struct StfsFileMeta {
	pub entry: StfsFileEntry,
}

impl Metadata for StfsFileMeta {
	fn len(&self) -> u64 {
		self.entry.file_size as u64
	}
}

#[derive(Debug)]
pub struct StfsVfs<S> {
	source: S,
	package: StfsPackage,
	tree: VfsTree<StfsFileMeta>,
}

impl<S> StfsVfs<S> {
	pub fn package(&self) -> &StfsPackage {
		&self.package
	}

	pub fn source(&self) -> &S {
		&self.source
	}

	pub fn tree(&self) -> &VfsTree<StfsFileMeta> {
		&self.tree
	}
}

impl<S: ReadAt> StfsVfs<S> {
	pub fn new(source: S, package: StfsPackage) -> Self {
		let tree = build_tree(&package.file_table);
		StfsVfs { source, package, tree }
	}
}

fn build_tree(file_table: &StfsFileTable) -> VfsTree<StfsFileMeta> {
	let mut builder = VfsTree::builder();
	for walk_entry in file_table.walk_files() {
		builder = builder.insert(walk_entry.path, StfsFileMeta { entry: walk_entry.entry });
	}
	builder.build()
}

impl<S: ReadAt + Debug + Send + Sync + 'static> FileSystem for StfsVfs<S> {
	fn read_dir(&self, path: &str) -> vfs::VfsResult<Box<dyn Iterator<Item = String> + Send>> {
		self.tree.vfs_read_dir(path)
	}

	fn open_file(&self, path: &str) -> vfs::VfsResult<Box<dyn vfs::SeekAndRead + Send>> {
		let entry = self.tree.vfs_lookup(path)?;
		let VfsEntry::File(meta) = entry else {
			return Err(VfsErrorKind::Other("not a file".into()).into());
		};

		let mut data = Vec::with_capacity(meta.entry.file_size);
		self.package
			.extract_file(&self.source, &mut data, &meta.entry)
			.map_err(|e| vfs::VfsError::from(VfsErrorKind::IoError(std::io::Error::other(e))))?;
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

	#[async_trait]
	impl<S: ReadAt + Debug + Send + Sync + 'static> AsyncFileSystem for StfsVfs<S> {
		async fn read_dir(&self, path: &str) -> vfs::VfsResult<Box<dyn Unpin + futures::Stream<Item = String> + Send>> {
			self.tree.async_vfs_read_dir(path)
		}

		async fn open_file(&self, path: &str) -> vfs::VfsResult<Box<dyn vfs::async_vfs::SeekAndRead + Send + Unpin>> {
			let entry = self.tree.vfs_lookup(path)?;
			let VfsEntry::File(meta) = entry else {
				return Err(VfsErrorKind::Other("not a file".into()).into());
			};

			let mut data = Vec::with_capacity(meta.entry.file_size);
			self.package
				.extract_file(&self.source, &mut data, &meta.entry)
				.map_err(|e| vfs::VfsError::from(VfsErrorKind::IoError(std::io::Error::other(e))))?;
			Ok(Box::new(futures::io::Cursor::new(data)))
		}

		async fn metadata(&self, path: &str) -> vfs::VfsResult<vfs::VfsMetadata> {
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
