//! Xbox 360 FATX volumes, read the way the console's FATX driver reads them, with forensic
//! recovery of deleted entries and data.
//!
//! A volume is a byte range of any source: a hard disk partition, a memory unit partition, or a
//! volume file. The crate is sans-io. [`OpenVolume`], [`ReadDirectory`], [`WalkVolume`] and the
//! other machines name the byte ranges they need; [`io::drive`] and [`io::drive_async`] run them
//! against any [`ReadAt`] or [`AsyncReadAt`] source. [`Volume`] has synchronous and asynchronous
//! methods for the common operations.
//!
//! A volume file whose FAT is sized for a longer partition than the file, such as a flash
//! `.fdf` file, opens with the partition length that [`Volume::infer_partition_length`] finds.
//!
//! A failed integrity check, such as a record the driver would skip or a broken file chain,
//! never stops a listing. Results come wrapped in [`Checked`], and the caller decides whether
//! to ignore the failures or treat them as errors.
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
//! # Features
//!
//! - `vfs`: [`fatx_vfs::FatxVfs`] implements `vfs::FileSystem` over a volume's live files.
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
pub mod time;
pub mod volume;
pub mod volume_file;
pub mod walk;

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
pub use fat::ClusterNumber;
pub use fat::ClusterRun;
pub use fat::FatEntry;
pub use fat::FatWidth;
pub use file::ReadRanges;
pub use forensic::Confidence;
pub use io::AsyncReadAt;
pub use io::ReadAt;
pub use machine::Machine;
pub use machine::Step;
pub use time::Timestamp;
pub use volume::Geometry;
pub use volume::OpenVolume;
pub use volume::PartitionKind;
pub use volume::SerialNumber;
pub use volume::Volume;
pub use volume::VolumeOptions;
pub use volume_file::InferPartitionLength;
pub use volume_file::PartitionInference;
pub use walk::DeletedEntry;
pub use walk::DeletedOrigin;
pub use walk::ListedEntry;
pub use walk::Listing;
pub use walk::WalkVolume;
