//! Xbox 360 NAND flash images, as the flash controller (SFCX) and its kernel driver see them.
//!
//! An image is either the logical flash device, as XBDM `getfile FLASH:\` returns it, or a raw
//! NAND image with a 16-byte spare area after every 512 bytes. A raw image is read the way the
//! console's driver reads it: spare areas are removed, bad blocks are replaced by the spare
//! blocks that hold them, and every sector's ECC is checked.
//!
//! The crate is sans-io. [`OpenImage`] and the machine that [`FlashImage::reader`] returns
//! name the byte ranges they need; [`io::drive`] and [`io::drive_async`] run them against any
//! [`ReadAt`] or [`AsyncReadAt`] source. [`FlashImage`] has synchronous and asynchronous
//! methods for the common operations.
//!
//! A failed integrity check, such as an ECC mismatch, never stops a read. Results come wrapped
//! in [`Checked`], and the caller decides whether to ignore the failures or treat them as
//! errors.
//!
//! # Example
//!
//! ```no_run
//! use sfcx::FlashImage;
//! use sfcx::FormatSelection;
//! use sfcx::io::SliceSource;
//!
//! let data = std::fs::read("nand.bin")?;
//! let source = SliceSource(data.as_slice());
//! let image = FlashImage::open(&source, data.len() as u64, FormatSelection::Auto)?.deny_warnings()?;
//! let filesystem = image.filesystem(&source)?.deny_warnings()?;
//! for entry in filesystem.entries() {
//!     println!("{} ({} bytes)", entry.name(), entry.size());
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Features
//!
//! - `vfs`: [`sfcx_vfs::SfcxVfs`] implements `vfs::FileSystem` over the flash file system.
//! - `async-vfs`: also implements `vfs::async_vfs::AsyncFileSystem`. Implies `vfs`.
//! - `clap`: [`FormatSelection`] implements `clap::ValueEnum`.

pub mod boot;
pub mod checked;
pub mod error;
pub mod format;
pub mod fs;
pub mod image;
pub mod mu;
pub mod raw;
#[cfg(feature = "vfs")]
pub mod sfcx_vfs;

pub use xenon_io::io;
pub use xenon_io::machine;

pub use boot::BootRegion;
pub use boot::BootStage;
pub use boot::ImageHeader;
pub use checked::Checked;
pub use checked::IntegrityError;
pub use checked::IntegrityPolicy;
pub use error::MemoryUnitError;
pub use error::ParseError;
pub use error::RawError;
pub use error::ReadError;
pub use error::ReadErrorKind;
pub use error::SfcxError;
pub use format::FormatSelection;
pub use format::ImageFormat;
pub use fs::DirectoryEntry;
pub use fs::Filesystem;
pub use image::FlashImage;
pub use image::Layout;
pub use image::OpenImage;
pub use image::ReadLogical;
pub use io::AsyncReadAt;
pub use io::ReadAt;
pub use machine::Machine;
pub use machine::Step;
pub use mu::AsyncMemoryUnitSource;
pub use mu::MemoryUnit;
pub use mu::MemoryUnitPartitions;
pub use mu::MemoryUnitSource;
pub use mu::MuBlockIndex;
pub use mu::OpenMemoryUnit;
pub use mu::ReadMemoryUnit;
pub use raw::RawWarning;
