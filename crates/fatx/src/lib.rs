//! Xbox 360 FATX volumes, read and written the way the console's FATX driver reads and writes
//! them, with forensic recovery of deleted entries and data.
//!
//! A volume is a byte range of any source: a hard disk partition, a memory unit partition, or a
//! volume file. The crate is sans-io. [`OpenVolume`], [`ReadDirectory`], [`WalkVolume`] and the
//! other machines name the byte ranges they need; [`io::drive`] and [`io::drive_async`] run them
//! against any [`ReadAt`] or [`AsyncReadAt`] source. [`Volume`] has synchronous and asynchronous
//! methods for the common operations.
//!
//! The partition length a volume is opened with is the one the console mounts it with: the
//! partition table's, or the file's length for a FATX file mounted as a partition. The console
//! does not check it against the volume; a walk reports clusters that no live chain reaches.
//!
//! A failed integrity check, such as a record the driver would skip or a broken file chain,
//! never stops a listing. Results come wrapped in [`Checked`], and the caller decides whether
//! to ignore the failures or treat them as errors.
//!
//! [`Volume::format`] creates a volume as XAM formats one, and [`Volume::update`] makes one
//! [`Request`] to it: a new directory, file data, a deletion, a rename, or new attributes and
//! timestamps. Both are [`WriteMachine`]s that [`io::drive_write`] and
//! [`io::drive_write_async`] run against a source that is also [`WriteAt`] or
//! [`AsyncWriteAt`]. The [`update`] module documents the order of their writes.
//!
//! # Example
//!
//! ```no_run
//! use fatx::Volume;
//! use fatx::io::SliceSource;
//!
//! let data = std::fs::read("partition.bin")?;
//! let source = SliceSource(data.as_slice());
//! let volume = Volume::open(&source, 0..data.len() as u64)?;
//! let listing = volume.walk(&source)?.deny_warnings()?;
//! for entry in listing.entries() {
//!     println!("{} ({} bytes)", entry.path(), entry.record().size());
//! }
//! for deleted in listing.deleted() {
//!     println!("deleted: {}", deleted.path());
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Writing to an image in memory:
//!
//! ```
//! use fatx::FormatOptions;
//! use fatx::Request;
//! use fatx::SerialNumber;
//! use fatx::Timestamp;
//! use fatx::Volume;
//! use fatx::WriteMode;
//! use fatx::io::MemorySource;
//!
//! let source = MemorySource::zeroed(0x0100_0000);
//! let mut volume = Volume::format(&source, 0..0x0100_0000, FormatOptions::new(SerialNumber::new(1)))?;
//! let now = Timestamp::from_fields(2010, 9, 9, 19, 0, 42).expect("a valid date");
//! volume.update(&source, Request::create_directory("/Content", now)?)?;
//! volume.update(&source, Request::write_file("/Content/save.bin", b"data", WriteMode::Replace, now)?)?;
//! let listing = volume.walk(&source)?.deny_warnings()?;
//! assert_eq!(listing.find("/Content/save.bin").unwrap().record().size(), 4);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Features
//!
//! - `vfs`: [`fatx_vfs::FatxVfs`] implements `vfs::FileSystem` over a volume's live files,
//!   with writes when the source is [`WriteAt`].
//! - `async-vfs`: also implements `vfs::async_vfs::AsyncFileSystem`. Implies `vfs`.

pub mod checked;
pub mod directory;
pub mod entry;
pub mod error;
pub mod fat;
#[cfg(feature = "vfs")]
pub mod fatx_vfs;
pub mod file;
pub mod forensic;
pub mod format;
pub mod name;
pub mod time;
pub mod update;
pub mod volume;
pub mod walk;

pub use xenon_io::FileSource;
pub use xenon_io::io;
pub use xenon_io::machine;

pub use checked::Checked;
pub use checked::IntegrityError;
pub use checked::IntegrityPolicy;
pub use checked::Warning;
pub use directory::Directory;
pub use directory::DirectoryMode;
pub use directory::ReadDirectory;
pub use entry::EntryLocation;
pub use entry::EntryRecord;
pub use entry::FileAttributes;
pub use error::ChainError;
pub use error::DirectoryError;
pub use error::FatxError;
pub use error::GeometryError;
pub use error::ReadError;
pub use error::ReadErrorKind;
pub use error::UpdateError;
pub use error::WriteError;
pub use fat::ClusterNumber;
pub use fat::ClusterRun;
pub use fat::FatEntry;
pub use fat::FatWidth;
pub use file::ReadRanges;
pub use forensic::Confidence;
pub use format::FormatOptions;
pub use format::FormatVolume;
pub use io::AsyncReadAt;
pub use io::AsyncWriteAt;
pub use io::ReadAt;
pub use io::WriteAt;
pub use machine::Effect;
pub use machine::Machine;
pub use machine::Step;
pub use machine::WriteMachine;
pub use name::FileName;
pub use name::NameError;
pub use name::PathError;
pub use name::VolumePath;
pub use time::Timestamp;
pub use update::Update;
pub use update::request::Change;
pub use update::request::EntryInfo;
pub use update::request::Request;
pub use update::request::WriteMode;
pub use volume::Geometry;
pub use volume::MAXIMUM_READ_SIZE;
pub use volume::OpenVolume;
pub use volume::PartitionKind;
pub use volume::SerialNumber;
pub use volume::Volume;
pub use volume::VolumeOptions;
pub use walk::DeletedEntry;
pub use walk::DeletedOrigin;
pub use walk::ListedEntry;
pub use walk::Listing;
pub use walk::WalkVolume;
