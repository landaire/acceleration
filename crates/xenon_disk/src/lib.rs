//! Partition layouts of Xbox 360 hard disks, memory units and USB storage, as the kernel
//! creates their partition devices.
//!
//! - [`layout`] holds the fixed layouts that depend only on the device size: the retail hard
//!   disk, the transfer cable, and each kind of memory unit.
//! - [`devkit`] parses, validates, builds and serializes the devkit partition table in sector 0.
//! - [`usb_storage`] joins the `Data####` files of a USB storage device into one logical device
//!   and lays out its partitions.
//! - [`security`] parses the security sector of a hard disk and the logo region after it.
//! - [`josh`] parses the block at 0x800 of a hard disk that the kernel does not read.
//! - [`detect`](mod@detect) identifies the device a source holds, as a sans-io [`Machine`] with synchronous
//!   and asynchronous drivers.
//!
//! Every layout is a [`PartitionMap`] of named [`Partition`]s in bytes from the start of the
//! device. The crate reads no file system; a partition with a FATX volume is handed to the
//! `fatx` crate by its byte range and [`VolumeKind`].

mod bytes;
pub mod detect;
pub mod device;
pub mod devkit;
pub mod error;
pub mod josh;
pub mod layout;
pub mod partition;
pub mod security;
pub mod units;
pub mod usb_storage;

pub use xenon_io::AsyncReadAt;
pub use xenon_io::Checked;
pub use xenon_io::Machine;
pub use xenon_io::ReadAt;
pub use xenon_io::Step;

pub use detect::Detect;
pub use detect::DetectOptions;
pub use detect::Detection;
pub use detect::Evidence;
pub use detect::detect;
pub use detect::detect_async;
pub use device::DeviceKind;
pub use devkit::DevkitEntry;
pub use devkit::DevkitRequest;
pub use devkit::DevkitSlot;
pub use devkit::DevkitTable;
pub use devkit::MemoryDumpSize;
pub use devkit::RequestSize;
pub use error::DetectError;
pub use error::DevkitTableError;
pub use error::DiskWarning;
pub use error::JoshBlockError;
pub use error::LayoutError;
pub use error::LogoBitmapError;
pub use error::UsbStorageError;
pub use josh::CacheEntry;
pub use josh::CacheIndex;
pub use josh::JoshBlock;
pub use layout::DiskSectors;
pub use layout::MmcSectorLimit;
pub use partition::Partition;
pub use partition::PartitionMap;
pub use partition::VolumeKind;
pub use security::LogoBitmap;
pub use security::SecuritySector;
pub use units::ByteCount;
pub use units::ByteOffset;
pub use units::Lba;
pub use units::SECTOR_SIZE;
pub use units::SectorCount;
pub use usb_storage::JoinedSource;
pub use usb_storage::UsbStorageLayout;
