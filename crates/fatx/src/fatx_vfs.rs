use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io;
use std::io::Cursor;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
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
use crate::error::UpdateError;
use crate::io::ReadAt;
use crate::io::WriteAt;
use crate::time::Timestamp;
use crate::update::request::Change;
use crate::update::request::EntryInfo;
use crate::update::request::Request;
use crate::update::request::WriteMode;
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

/// A volume's live files and directories as a [`vfs::FileSystem`].
///
/// Writes need a source that is also [`WriteAt`]; wrap a read-only source in
/// [`crate::io::ReadOnly`] to serve reads alone. Timestamps of changes are the host's clock
/// read as UTC. A writer from [`FileSystem::create_file`] or [`FileSystem::append_file`]
/// writes its bytes to the volume when flushed or dropped; only a flush reports a failure.
/// Changes run one at a time; each one updates the in-memory tree without walking the volume
/// again.
#[derive(Debug)]
pub struct FatxVfs<S> {
	shared: Arc<Shared<S>>,
}

#[derive(Debug)]
struct Shared<S> {
	source: S,
	state: Mutex<VfsState>,
	/// Held for the whole of a change, including an async change that drives its writes
	/// without holding `state`.
	#[cfg(feature = "async-vfs")]
	changing: futures::lock::Mutex<()>,
}

#[derive(Debug)]
struct VfsState {
	volume: Volume,
	entries: BTreeMap<String, EntryRecord>,
	tree: VfsTree<FatxFileMeta>,
	/// A change failed after some of its writes may have reached the source, so the
	/// in-memory FAT may no longer match it.
	poisoned: bool,
}

impl<S> FatxVfs<S> {
	pub fn new(source: S, volume: Volume, listing: Listing) -> Self {
		let entries: BTreeMap<String, EntryRecord> =
			listing.entries().iter().map(|entry| (entry.path().to_owned(), entry.record().clone())).collect();
		let tree = build_tree(&entries);
		let state = Mutex::new(VfsState { volume, entries, tree, poisoned: false });

		Self {
			shared: Arc::new(Shared {
				source,
				state,
				#[cfg(feature = "async-vfs")]
				changing: futures::lock::Mutex::new(()),
			}),
		}
	}

	pub fn source(&self) -> &S {
		&self.shared.source
	}

	/// A copy of the volume with the FAT as the changes so far left it.
	pub fn volume(&self) -> Volume {
		self.shared.state().volume.clone()
	}

	fn file_record(&self, path: &str) -> vfs::VfsResult<EntryRecord> {
		self.shared.state().file_record(path)
	}

	fn require_directory(&self, path: &str) -> vfs::VfsResult<()> {
		match self.shared.state().tree.vfs_lookup(path)? {
			VfsEntry::Directory { .. } => Ok(()),
			VfsEntry::File(_) => Err(VfsErrorKind::Other("not a directory".into()).into()),
		}
	}
}

impl<S> Shared<S> {
	fn state(&self) -> MutexGuard<'_, VfsState> {
		// A change updates the state only after its writes succeed, in one assignment.
		self.state.lock().unwrap_or_else(PoisonError::into_inner)
	}
}

impl<S> Shared<S>
where
	S: ReadAt + WriteAt,
{
	fn change(&self, request: Request<'_>) -> vfs::VfsResult<Change> {
		#[cfg(feature = "async-vfs")]
		let _changing = futures::executor::block_on(self.changing.lock());

		let mut state = self.state();
		state.check_poisoned()?;
		let result = state.volume.update(&self.source, request);
		state.poisoned = matches!(result, Err(FatxError::Write(_)));
		let change = result.map_err(to_vfs_error)?;
		state.apply(&change);

		Ok(change)
	}

	/// Reads the live file at `path`, finding its record and reading it while no change runs.
	fn read_file(&self, path: &str) -> vfs::VfsResult<Vec<u8>> {
		#[cfg(feature = "async-vfs")]
		let _changing = futures::executor::block_on(self.changing.lock());

		let state = self.state();
		let record = state.file_record(path)?;
		state.volume.read_file(&self.source, &record).map_err(to_vfs_error)
	}
}

impl VfsState {
	fn check_poisoned(&self) -> vfs::VfsResult<()> {
		if self.poisoned {
			return Err(VfsErrorKind::Other("an earlier change failed part way; reopen the volume".into()).into());
		}

		Ok(())
	}

	/// The record of the live file at `path`. A poisoned state refuses, so no read or writer
	/// starts from a FAT that may not match the source.
	fn file_record(&self, path: &str) -> vfs::VfsResult<EntryRecord> {
		self.check_poisoned()?;

		match self.tree.vfs_lookup(path)? {
			VfsEntry::File(meta) => Ok(meta.record.clone()),
			VfsEntry::Directory { .. } => Err(VfsErrorKind::Other("not a file".into()).into()),
		}
	}
}

impl VfsState {
	fn apply(&mut self, change: &Change) {
		match change {
			Change::Created { path, record } | Change::Modified { path, record } => {
				self.entries.insert(path.clone(), record.clone());
			}
			Change::Removed { path, .. } => {
				self.entries.remove(path);
			}
			Change::Renamed { from, to, record } => {
				self.entries.remove(from);
				let prefix = format!("{from}/");
				let moved: Vec<String> =
					self.entries.keys().filter(|path| path.starts_with(&prefix)).cloned().collect();
				for path in moved {
					let record = self.entries.remove(&path).expect("listed above");
					self.entries.insert(format!("{to}/{}", &path[prefix.len()..]), record);
				}
				self.entries.insert(to.clone(), record.clone());
			}
		}

		self.tree = build_tree(&self.entries);
	}
}

/// Bytes written through the VFS since the last commit. An append writer stores them at the
/// end of the file as it is when committed; a new file's writer at `offset`.
#[derive(Debug)]
struct PendingFile {
	path: String,
	append: bool,
	offset: u64,
	buffer: Vec<u8>,
}

impl PendingFile {
	fn request(&self, now: Timestamp) -> Result<Request<'_>, UpdateError> {
		let mode = if self.append { WriteMode::Append } else { WriteMode::At(self.offset) };

		Request::write_file(&self.path, &self.buffer, mode, now)
	}

	fn committed(&mut self) {
		self.offset += self.buffer.len() as u64;
		self.buffer.clear();
	}
}

/// A file opened for writing through [`FatxVfs`]. It holds the bytes written since the last
/// flush. Seeking reports the position; a seek elsewhere is unsupported.
struct FileWriter<S>
where
	S: ReadAt + WriteAt,
{
	shared: Arc<Shared<S>>,
	pending: PendingFile,
}

impl<S> FileWriter<S>
where
	S: ReadAt + WriteAt,
{
	fn commit(&mut self) -> io::Result<()> {
		if self.pending.buffer.is_empty() {
			return Ok(());
		}

		let result = self.pending.request(now().map_err(vfs_to_io)?).map_err(io::Error::other);
		let result = result.and_then(|request| self.shared.change(request).map(drop).map_err(vfs_to_io));
		if result.is_err() {
			// The failure was reported; a drop must not retry the bytes over a changed volume.
			self.pending.buffer.clear();
		}
		result?;
		self.pending.committed();

		Ok(())
	}
}

impl<S> Write for FileWriter<S>
where
	S: ReadAt + WriteAt,
{
	fn write(&mut self, data: &[u8]) -> io::Result<usize> {
		self.pending.buffer.extend_from_slice(data);

		Ok(data.len())
	}

	fn flush(&mut self) -> io::Result<()> {
		self.commit()
	}
}

impl<S> Seek for FileWriter<S>
where
	S: ReadAt + WriteAt,
{
	fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
		let current = self.pending.offset + self.pending.buffer.len() as u64;
		match position {
			SeekFrom::Current(0) | SeekFrom::End(0) => Ok(current),
			SeekFrom::Start(offset) if offset == current => Ok(current),
			_ => Err(io::Error::new(io::ErrorKind::Unsupported, "a FATX file writer only appends")),
		}
	}
}

impl<S> Drop for FileWriter<S>
where
	S: ReadAt + WriteAt,
{
	fn drop(&mut self) {
		// The documented contract: a flush reports failures, a drop cannot.
		let _ = self.commit();
	}
}

fn system_time(timestamp: Timestamp) -> Option<SystemTime> {
	timestamp.unix_seconds().map(fskit::system_time_from_unix)
}

/// The host's clock as a volume timestamp.
fn now() -> vfs::VfsResult<Timestamp> {
	timestamp(SystemTime::now())
}

fn timestamp(time: SystemTime) -> vfs::VfsResult<Timestamp> {
	Timestamp::from_system_time(time).ok_or_else(|| {
		VfsErrorKind::Other("the time is outside the years 1980 to 2107 a FATX timestamp holds".into()).into()
	})
}

fn build_tree(entries: &BTreeMap<String, EntryRecord>) -> VfsTree<FatxFileMeta> {
	let mut builder = VfsTree::builder();
	for (path, record) in entries {
		let meta = FatxFileMeta { record: record.clone() };
		builder = if record.is_directory() { builder.insert_dir(path, Some(meta)) } else { builder.insert(path, meta) };
	}

	builder.build()
}

fn to_vfs_error(error: FatxError) -> VfsError {
	match error {
		FatxError::Update(UpdateError::NotFound { .. }) => VfsErrorKind::FileNotFound.into(),
		FatxError::Update(UpdateError::AlreadyExists { .. }) => VfsErrorKind::FileExists.into(),
		error => VfsErrorKind::IoError(io::Error::other(error)).into(),
	}
}

fn update_to_vfs_error(error: UpdateError) -> VfsError {
	to_vfs_error(FatxError::Update(error))
}

fn vfs_to_io(error: VfsError) -> io::Error {
	io::Error::other(error)
}

impl<S> FileSystem for FatxVfs<S>
where
	S: ReadAt + WriteAt + Debug + Send + Sync + 'static,
{
	fn read_dir(&self, path: &str) -> vfs::VfsResult<Box<dyn Iterator<Item = String> + Send>> {
		self.shared.state().tree.vfs_read_dir(path)
	}

	fn create_dir(&self, path: &str) -> vfs::VfsResult<()> {
		let request = Request::create_directory(path, now()?).map_err(update_to_vfs_error)?;
		match self.shared.change(request) {
			Ok(_) => Ok(()),
			Err(error) if matches!(error.kind(), VfsErrorKind::FileExists) => Err(VfsErrorKind::DirectoryExists.into()),
			Err(error) => Err(error),
		}
	}

	fn open_file(&self, path: &str) -> vfs::VfsResult<Box<dyn vfs::SeekAndRead + Send>> {
		let data = self.shared.read_file(path)?;

		Ok(Box::new(Cursor::new(data)))
	}

	fn create_file(&self, path: &str) -> vfs::VfsResult<Box<dyn vfs::SeekAndWrite + Send>> {
		let request = Request::write_file(path, &[], WriteMode::Replace, now()?).map_err(update_to_vfs_error)?;
		let change = self.shared.change(request)?;
		let pending = PendingFile { path: change.path().to_owned(), append: false, offset: 0, buffer: Vec::new() };

		Ok(Box::new(FileWriter { shared: Arc::clone(&self.shared), pending }))
	}

	fn append_file(&self, path: &str) -> vfs::VfsResult<Box<dyn vfs::SeekAndWrite + Send>> {
		let record = self.file_record(path)?;
		let pending =
			PendingFile { path: path.to_owned(), append: true, offset: u64::from(record.size()), buffer: Vec::new() };

		Ok(Box::new(FileWriter { shared: Arc::clone(&self.shared), pending }))
	}

	fn metadata(&self, path: &str) -> vfs::VfsResult<VfsMetadata> {
		self.shared.state().tree.vfs_metadata(path)
	}

	fn set_creation_time(&self, path: &str, time: SystemTime) -> vfs::VfsResult<()> {
		let info = EntryInfo { created: Some(timestamp(time)?), ..EntryInfo::default() };
		self.shared.change(Request::set_info(path, info).map_err(update_to_vfs_error)?).map(drop)
	}

	fn set_modification_time(&self, path: &str, time: SystemTime) -> vfs::VfsResult<()> {
		let info = EntryInfo { modified: Some(timestamp(time)?), ..EntryInfo::default() };
		self.shared.change(Request::set_info(path, info).map_err(update_to_vfs_error)?).map(drop)
	}

	fn set_access_time(&self, path: &str, time: SystemTime) -> vfs::VfsResult<()> {
		let info = EntryInfo { accessed: Some(timestamp(time)?), ..EntryInfo::default() };
		self.shared.change(Request::set_info(path, info).map_err(update_to_vfs_error)?).map(drop)
	}

	fn exists(&self, path: &str) -> vfs::VfsResult<bool> {
		self.shared.state().tree.vfs_exists(path)
	}

	fn remove_file(&self, path: &str) -> vfs::VfsResult<()> {
		self.file_record(path)?;
		self.shared.change(Request::remove(path).map_err(update_to_vfs_error)?).map(drop)
	}

	fn remove_dir(&self, path: &str) -> vfs::VfsResult<()> {
		self.require_directory(path)?;
		self.shared.change(Request::remove(path).map_err(update_to_vfs_error)?).map(drop)
	}

	fn move_file(&self, from: &str, to: &str) -> vfs::VfsResult<()> {
		self.file_record(from)?;
		self.shared.change(Request::rename(from, to).map_err(update_to_vfs_error)?).map(drop)
	}

	fn move_dir(&self, from: &str, to: &str) -> vfs::VfsResult<()> {
		self.require_directory(from)?;
		self.shared.change(Request::rename(from, to).map_err(update_to_vfs_error)?).map(drop)
	}
}

#[cfg(feature = "async-vfs")]
const _: () = {
	use std::pin::Pin;
	use std::task::Context;
	use std::task::Poll;

	use async_trait::async_trait;
	use futures::FutureExt;
	use futures::future::BoxFuture;
	use vfs::async_vfs::AsyncFileSystem;

	use crate::io::AsyncReadAt;
	use crate::io::AsyncWriteAt;

	impl<S> Shared<S>
	where
		S: AsyncReadAt + AsyncWriteAt,
	{
		/// Drives the change on a copy of the volume, so that readers can use the state while
		/// it runs, and stores the copy once the writes succeed. The state is poisoned while
		/// the change runs, so a change dropped part way leaves it poisoned.
		async fn change_async(&self, request: Request<'_>) -> vfs::VfsResult<Change> {
			let _changing = self.changing.lock().await;

			let mut volume = {
				let mut state = self.state();
				state.check_poisoned()?;
				state.poisoned = true;
				state.volume.clone()
			};
			let result = volume.update_async(&self.source, request).await;

			let mut state = self.state();
			// Only a write failure can leave the source changed part way.
			state.poisoned = matches!(result, Err(FatxError::Write(_)));
			let change = result.map_err(to_vfs_error)?;
			state.volume = volume;
			state.apply(&change);

			Ok(change)
		}
	}

	/// The async form of the file writer. It writes its bytes to the volume when flushed or
	/// closed; a writer dropped without either discards them.
	struct AsyncFileWriter<S> {
		shared: Arc<Shared<S>>,
		pending: Option<PendingFile>,
		committing: Option<BoxFuture<'static, (PendingFile, vfs::VfsResult<()>)>>,
	}

	impl<S> AsyncFileWriter<S>
	where
		S: AsyncReadAt + AsyncWriteAt + 'static,
	{
		fn poll_commit(&mut self, context: &mut Context<'_>) -> Poll<io::Result<()>> {
			if self.committing.is_none() {
				let pending = self.pending.take().expect("a writer holds its file between commits");
				if pending.buffer.is_empty() {
					self.pending = Some(pending);
					return Poll::Ready(Ok(()));
				}

				let shared = Arc::clone(&self.shared);
				self.committing = Some(
					async move {
						let result = async {
							let request = pending.request(now()?).map_err(update_to_vfs_error)?;
							shared.change_async(request).await.map(drop)
						}
						.await;
						(pending, result)
					}
					.boxed(),
				);
			}

			let committing = self.committing.as_mut().expect("set above");
			let (mut pending, result) = match committing.as_mut().poll(context) {
				Poll::Pending => return Poll::Pending,
				Poll::Ready(done) => done,
			};
			self.committing = None;
			if result.is_ok() {
				pending.committed();
			}
			self.pending = Some(pending);

			Poll::Ready(result.map_err(vfs_to_io))
		}
	}

	impl<S> futures::io::AsyncWrite for AsyncFileWriter<S>
	where
		S: AsyncReadAt + AsyncWriteAt + 'static,
	{
		fn poll_write(self: Pin<&mut Self>, _context: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
			let this = self.get_mut();
			match this.pending.as_mut() {
				Some(pending) => {
					pending.buffer.extend_from_slice(data);
					Poll::Ready(Ok(data.len()))
				}
				None => Poll::Ready(Err(io::Error::other("a write cannot start while the writer commits"))),
			}
		}

		fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
			self.get_mut().poll_commit(context)
		}

		fn poll_close(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
			self.get_mut().poll_commit(context)
		}
	}

	#[async_trait]
	impl<S> AsyncFileSystem for FatxVfs<S>
	where
		S: AsyncReadAt + AsyncWriteAt + Debug + Send + Sync + 'static,
	{
		async fn read_dir(&self, path: &str) -> vfs::VfsResult<Box<dyn Unpin + futures::Stream<Item = String> + Send>> {
			self.shared.state().tree.async_vfs_read_dir(path)
		}

		async fn create_dir(&self, path: &str) -> vfs::VfsResult<()> {
			let request = Request::create_directory(path, now()?).map_err(update_to_vfs_error)?;
			match self.shared.change_async(request).await {
				Ok(_) => Ok(()),
				Err(error) if matches!(error.kind(), VfsErrorKind::FileExists) => {
					Err(VfsErrorKind::DirectoryExists.into())
				}
				Err(error) => Err(error),
			}
		}

		async fn open_file(&self, path: &str) -> vfs::VfsResult<Box<dyn vfs::async_vfs::SeekAndRead + Send + Unpin>> {
			let _changing = self.shared.changing.lock().await;
			let reader = {
				let state = self.shared.state();
				let record = state.file_record(path)?;
				state.volume.file_reader(&record).map_err(to_vfs_error)?
			};
			let data = crate::io::drive_async(reader, &self.shared.source).await.map_err(to_vfs_error)?;

			Ok(Box::new(futures::io::Cursor::new(data)))
		}

		async fn create_file(&self, path: &str) -> vfs::VfsResult<Box<dyn futures::io::AsyncWrite + Send + Unpin>> {
			let request = Request::write_file(path, &[], WriteMode::Replace, now()?).map_err(update_to_vfs_error)?;
			let change = self.shared.change_async(request).await?;
			let pending = PendingFile { path: change.path().to_owned(), append: false, offset: 0, buffer: Vec::new() };

			Ok(Box::new(AsyncFileWriter { shared: Arc::clone(&self.shared), pending: Some(pending), committing: None }))
		}

		async fn append_file(&self, path: &str) -> vfs::VfsResult<Box<dyn futures::io::AsyncWrite + Send + Unpin>> {
			let record = self.file_record(path)?;
			let pending = PendingFile {
				path: path.to_owned(),
				append: true,
				offset: u64::from(record.size()),
				buffer: Vec::new(),
			};

			Ok(Box::new(AsyncFileWriter { shared: Arc::clone(&self.shared), pending: Some(pending), committing: None }))
		}

		async fn metadata(&self, path: &str) -> vfs::VfsResult<VfsMetadata> {
			self.shared.state().tree.vfs_metadata(path)
		}

		async fn set_creation_time(&self, path: &str, time: SystemTime) -> vfs::VfsResult<()> {
			let info = EntryInfo { created: Some(timestamp(time)?), ..EntryInfo::default() };
			self.shared.change_async(Request::set_info(path, info).map_err(update_to_vfs_error)?).await.map(drop)
		}

		async fn set_modification_time(&self, path: &str, time: SystemTime) -> vfs::VfsResult<()> {
			let info = EntryInfo { modified: Some(timestamp(time)?), ..EntryInfo::default() };
			self.shared.change_async(Request::set_info(path, info).map_err(update_to_vfs_error)?).await.map(drop)
		}

		async fn set_access_time(&self, path: &str, time: SystemTime) -> vfs::VfsResult<()> {
			let info = EntryInfo { accessed: Some(timestamp(time)?), ..EntryInfo::default() };
			self.shared.change_async(Request::set_info(path, info).map_err(update_to_vfs_error)?).await.map(drop)
		}

		async fn exists(&self, path: &str) -> vfs::VfsResult<bool> {
			self.shared.state().tree.vfs_exists(path)
		}

		async fn remove_file(&self, path: &str) -> vfs::VfsResult<()> {
			self.file_record(path)?;
			self.shared.change_async(Request::remove(path).map_err(update_to_vfs_error)?).await.map(drop)
		}

		async fn remove_dir(&self, path: &str) -> vfs::VfsResult<()> {
			self.require_directory(path)?;
			self.shared.change_async(Request::remove(path).map_err(update_to_vfs_error)?).await.map(drop)
		}

		async fn move_file(&self, from: &str, to: &str) -> vfs::VfsResult<()> {
			self.file_record(from)?;
			self.shared.change_async(Request::rename(from, to).map_err(update_to_vfs_error)?).await.map(drop)
		}

		async fn move_dir(&self, from: &str, to: &str) -> vfs::VfsResult<()> {
			self.require_directory(from)?;
			self.shared.change_async(Request::rename(from, to).map_err(update_to_vfs_error)?).await.map(drop)
		}
	}
};
