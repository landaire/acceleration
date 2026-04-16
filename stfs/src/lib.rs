//! Xbox 360 STFS (Secure Transacted File System) parser and extractor.
//!
//! STFS is the filesystem format used inside Xbox 360 XContent packages
//! (CON, LIVE, PIRS). It's a hash-tree filesystem where every data block has
//! a SHA-1 hash stored in a separate hash table, making tampering detectable.
//!
//! This crate is sans-IO: parsing is driven through the [`ReadAt`] trait,
//! which can be implemented for any random-access data source. A
//! [`SliceReader`] impl is provided for in-memory use, and
//! [`BytesStfsReader`] wraps any `AsRef<[u8]>` (including `Vec<u8>` and
//! memory-mapped files) for convenience.
//!
//! # Example
//!
//! ```no_run
//! use stfs::{BytesStfsReader, StfsPackageReader};
//!
//! let data = std::fs::read("savegame.bin")?;
//! // XContent header is 0x971A bytes; STFS data starts after it
//! let stfs_start = 0x971A;
//! let reader = BytesStfsReader::open(&data[stfs_start..])?;
//!
//! for entry in reader.package().file_table.entries.iter() {
//!     println!("{} ({} bytes)", entry.name, entry.file_size);
//! }
//!
//! // Extract a specific file
//! let file = reader.package().file_table.entries.iter()
//!     .find(|e| e.name == "savegame.dat")
//!     .ok_or("not found")?;
//! let mut out = Vec::new();
//! reader.extract_file(&mut out, file)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # VFS integration
//!
//! With the `vfs` feature enabled, [`stfs_vfs::StfsVfs`] implements the
//! [`vfs::FileSystem`] trait, letting STFS packages be mounted as a
//! virtual filesystem.
//!
//! # Features
//!
//! - `vfs` -- enables [`stfs_vfs`] (builds a `VfsTree` from the file table
//!   and implements `vfs::FileSystem`).

pub mod error;
pub mod file_table;
pub mod hash;
pub mod hashing;
pub mod header;
pub mod io;
pub mod package;
pub(crate) mod serde_bytes;
pub(crate) mod serde_hex;
#[cfg(feature = "vfs")]
pub mod stfs_vfs;
pub mod types;
pub mod wrappers;

pub use error::StfsError;
pub use file_table::StfsFileEntry;
pub use file_table::StfsFileTable;
pub use file_table::StfsTreeNode;
pub use file_table::WalkEntry;
pub use hash::HashTableMeta;
pub use header::StfsVolumeDescriptor;
pub use header::SvodVolumeDescriptor;
pub use header::XContentHeader;
pub use io::ReadAt;
pub use io::SliceReader;
pub use package::StfsPackage;
pub use types::*;
pub use wrappers::StfsPackageReader;
pub use wrappers::bytes::BytesStfsReader;
