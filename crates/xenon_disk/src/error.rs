use std::fmt;

use thiserror::Error;

use crate::device::DeviceKind;
use crate::devkit::DevkitSlot;
use crate::units::ByteCount;
use crate::units::Lba;
use crate::units::SectorCount;

pub use xenon_io::ReadError;
pub use xenon_io::ReadErrorKind;

/// A device size that the kernel would not lay out.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
	#[error("a {device} of {length:#x} bytes is smaller than the {minimum:#x} bytes the kernel mounts")]
	TooSmall { device: DeviceKind, length: ByteCount, minimum: ByteCount },
	#[error("a {device} of {length:#x} bytes is larger than the {maximum:#x} bytes the kernel mounts")]
	TooLarge { device: DeviceKind, length: ByteCount, maximum: ByteCount },
	#[error("a {device} of {length:#x} bytes is not a multiple of {alignment:#x} bytes")]
	Unaligned { device: DeviceKind, length: ByteCount, alignment: ByteCount },
	#[error("{length:#x} bytes is not a whole number of sectors that a 32-bit sector count holds")]
	NotWholeSectors { length: ByteCount },
	#[error("a {device} has no layout of its own")]
	NoFixedLayout { device: DeviceKind },
	#[error("the console disables its {device}")]
	Disabled { device: DeviceKind },
	#[error(transparent)]
	UsbStorage(#[from] UsbStorageError),
}

/// A devkit partition table that the kernel rejects, or one that would not lay out the disk.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum DevkitTableError {
	#[error("table version {version:#010x} is not {expected:#010x}")]
	VersionMismatch { version: u32, expected: u32 },
	#[error("table minimum {minimum:#010x} is below {required:#010x}")]
	MinimumTooLow { minimum: u32, required: u32 },
	#[error("{slot} at sector {start} with {count} sectors is not inside the {disk}-sector disk")]
	OutOfBounds { slot: DevkitSlot, start: Lba, count: SectorCount, disk: SectorCount },
	#[error(
		"{slot} at sector {start} overlaps the first {reserved} sectors, which hold the table and the security sector"
	)]
	OverlapsReserved { slot: DevkitSlot, start: Lba, reserved: SectorCount },
	#[error("{first} and {second} overlap")]
	Overlap { first: DevkitSlot, second: DevkitSlot },
	#[error("DumpPartition of {length:#x} bytes cannot hold its {required:#x}-byte cache partitions")]
	DumpPartitionTooSmall { length: ByteCount, required: ByteCount },
	#[error("{slot} is requested twice")]
	DuplicateSlot { slot: DevkitSlot },
	#[error("{slot} size {size:#x} is not a positive multiple of the 0x200-byte sector")]
	UnalignedSize { slot: DevkitSlot, size: ByteCount },
	#[error("{alignment} sectors is not a power of two")]
	InvalidAlignment { alignment: SectorCount },
	#[error("{slot} takes the rest of the disk, so it must be the last request")]
	RestNotLast { slot: DevkitSlot },
	#[error("{slot} would end at sector {end:#x}, past the {disk}-sector disk")]
	NoSpace { slot: DevkitSlot, end: u64, disk: SectorCount },
	#[error("no sectors are left for {slot}")]
	NoSectorsLeft { slot: DevkitSlot },
}

/// A set of USB storage files that the fatw driver would not mount.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum UsbStorageError {
	#[error("{count} files cannot hold the reserved file and the metadata file")]
	TooFewFiles { count: usize },
	#[error("{count} files exceed the {maximum} that the driver mounts")]
	TooManyFiles { count: usize, maximum: usize },
	#[error("Data{index:04} holds {length:#x} bytes; the layout needs {required:#x}")]
	FileTooShort { index: usize, length: ByteCount, required: ByteCount },
	#[error("data length {length:#x} is not a multiple of the 0x4000-byte cluster")]
	UnalignedDataLength { length: ByteCount },
	#[error("data length {length:#x} exceeds the {maximum:#x}-byte maximum")]
	DataLengthTooLarge { length: ByteCount, maximum: ByteCount },
	#[error("the layout has {expected} files but {actual} were given")]
	FileCount { expected: usize, actual: usize },
	#[error("{length:#x} bytes is not the length of any USB storage layout")]
	NotALayoutLength { length: ByteCount },
}

/// A logo bitmap region whose contents do not hold a length-prefixed image.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum LogoBitmapError {
	#[error("the logo image length {length:#x} exceeds the {available:#x} bytes after it")]
	LengthPastRegion { length: u32, available: usize },
}

/// Bytes at 0x800 that are not a Josh block.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum JoshBlockError {
	#[error("magic {magic:02x?} is not Josh")]
	BadMagic { magic: [u8; 4] },
}

/// A device that detection cannot identify.
#[derive(Debug, Error)]
pub enum DetectError {
	#[error(transparent)]
	Read(#[from] ReadError),
	#[error("no known layout matches the {length:#x}-byte source")]
	Unrecognized { length: ByteCount },
	#[error("the {length:#x}-byte source starts with a FATX volume and no other partition of a known layout does")]
	LoneVolume { length: ByteCount },
}

/// A failed integrity check of a disk's metadata that does not stop the layout from being read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskWarning {
	/// The SHA-1 digest of the logo bitmap at 0x2200 differs from the security sector's.
	LogoDigestMismatch { expected: [u8; 20], actual: [u8; 20] },
	/// The security sector limits the disk to more sectors than the device has; the kernel then
	/// fails authentication.
	SecurityCountExceedsDevice { security: SectorCount, device: SectorCount },
	/// The security sector count is the value the kernel rejects (`FIELD_OFFSET(Reserved)`).
	SecurityCountRejected { security: SectorCount },
	/// The devkit table assigns a partition the kernel would place outside the disk or over
	/// another partition.
	DevkitTable(DevkitTableError),
	/// The console certificate of the Josh block is invalid or its authority key is not
	/// available.
	JoshCertificate(xecrypt::CertificateStatus),
	/// The console signature of the Josh block does not verify; a title would rewrite the block
	/// and reformat the caches.
	JoshConsoleSignature(xecrypt::ConsoleSignatureError),
	/// The Josh block version is not the one titles accept.
	JoshVersion { version: u32, expected: u32 },
}

impl fmt::Display for DiskWarning {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::LogoDigestMismatch { expected, actual } => write!(
				f,
				"logo bitmap digest {} differs from the security sector digest {}",
				Hex(actual),
				Hex(expected)
			),
			Self::SecurityCountExceedsDevice { security, device } => {
				write!(f, "security sector count {security} exceeds the {device}-sector device")
			}
			Self::SecurityCountRejected { security } => {
				write!(f, "security sector count {security} is the value the kernel rejects")
			}
			Self::DevkitTable(error) => write!(f, "devkit partition table: {error}"),
			Self::JoshCertificate(status) => write!(f, "Josh block console certificate {status}"),
			Self::JoshConsoleSignature(error) => write!(f, "Josh block console signature: {error}"),
			Self::JoshVersion { version, expected } => {
				write!(f, "Josh block version {version} is not {expected}")
			}
		}
	}
}

/// Bytes printed as lowercase hexadecimal digits.
pub(crate) struct Hex<'a>(pub &'a [u8]);

impl fmt::Display for Hex<'_> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		for byte in self.0 {
			write!(f, "{byte:02x}")?;
		}

		Ok(())
	}
}
