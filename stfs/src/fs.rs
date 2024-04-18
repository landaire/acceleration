use std::fmt::Debug;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use vfs::error::VfsErrorKind;
use vfs::FileSystem;
use vfs::VfsError;

use bytes::Bytes;

use crate::StfsEntry;
use crate::StfsEntryRef;
use crate::StfsFileEntry;
use crate::StfsPackage;

pub struct StFS<T> {
	pub package: StfsPackage,
	pub data: Arc<T>,
}

impl<T> std::fmt::Debug for StFS<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("StFS").field("package", &self.package).field("data", &"...").finish()
	}
}

impl<T> StFS<T> {
	fn find_file(&self, path: &str) -> vfs::VfsResult<StfsEntryRef> {
		let path = PathBuf::from(path);
		let mut current = Arc::clone(&self.package.files);

		for part in path.iter() {
			if part == "/" {
				continue;
			}
			// Look up this part of the path in our dir
			let current_copy = Arc::clone(&current);
			let node = current_copy.lock();
			match &*node {
				crate::StfsEntry::File(_, _) => return Err(VfsErrorKind::FileNotFound.into()),
				crate::StfsEntry::Folder { entry, files } => {
					if let Some(node) = files.iter().find(|file| file.lock().name() == part.to_string_lossy()) {
						current = Arc::clone(node)
					} else {
						return Err(VfsErrorKind::FileNotFound.into());
					}
				}
			}
		}

		Ok(current)
	}
}

impl<T: AsRef<[u8]> + Send + Sync + 'static> FileSystem for StFS<T> {
	fn read_dir(&self, path: &str) -> vfs::VfsResult<Box<dyn Iterator<Item = String> + Send>> {
		let dir = self.find_file(path)?;

		let dir = dir.lock();

		if let StfsEntry::Folder { entry, files } = &*dir {
			Ok(Box::new(files.iter().map(|file| file.lock().name()).collect::<Vec<_>>().into_iter()))
		} else {
			unreachable!("we should never have a file here")
		}
	}

	fn create_dir(&self, path: &str) -> vfs::VfsResult<()> {
		todo!()
	}

	fn open_file(&self, path: &str) -> vfs::VfsResult<Box<dyn vfs::SeekAndRead + Send>> {
		todo!()
	}

	fn create_file(&self, path: &str) -> vfs::VfsResult<Box<dyn vfs::SeekAndWrite + Send>> {
		todo!()
	}

	fn append_file(&self, path: &str) -> vfs::VfsResult<Box<dyn vfs::SeekAndWrite + Send>> {
		todo!()
	}

	fn metadata(&self, path: &str) -> vfs::VfsResult<vfs::VfsMetadata> {
		let file = self.find_file(path)?;
		let file = &*file.lock();

		let metadata = match file {
			StfsEntry::File(entry, _) => {
				let attr = entry.file_attributes.as_ref().unwrap();
				vfs::VfsMetadata {
					file_type: vfs::VfsFileType::File,
					len: attr.file_size as u64,
					created: Some(crate::util::stf_timestamp_to_chrono(attr.created_time_stamp).into()),
					modified: None,
					accessed: Some(crate::util::stf_timestamp_to_chrono(attr.access_time_stamp).into()),
				}
			}
			StfsEntry::Folder { entry, files } => {
				let attr = entry.file_attributes.as_ref().unwrap();
				vfs::VfsMetadata {
					file_type: vfs::VfsFileType::Directory,
					len: 0,
					created: Some(crate::util::stf_timestamp_to_chrono(attr.created_time_stamp).into()),
					modified: None,
					accessed: Some(crate::util::stf_timestamp_to_chrono(attr.access_time_stamp).into()),
				}
			}
		};

		Ok(metadata)
	}

	fn exists(&self, path: &str) -> vfs::VfsResult<bool> {
		todo!()
	}

	fn remove_file(&self, path: &str) -> vfs::VfsResult<()> {
		todo!()
	}

	fn remove_dir(&self, path: &str) -> vfs::VfsResult<()> {
		todo!()
	}
}
