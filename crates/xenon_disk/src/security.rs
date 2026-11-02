//! The security sector of a hard disk (`SATA_DISK_SECURITY_SECTOR`, `drivers/sata/disk.c`), which
//! names the drive, limits its sector count and carries the digest of the logo bitmap.
//!
//! The kernel verifies the signature with `XeKeysVerifyRSASignature(2, ...)`, which checks
//! public key slots 3 and 6 of the hypervisor key export page. Neither key is in this tree, so
//! the signature and the digest it signs are exposed but not verified. A development kit drive
//! was observed with an all-zero signature.

use std::fmt;

use sha1::Digest;
use sha1::Sha1;
use xenon_types::FixedText;

use crate::bytes::array;
use crate::error::DiskWarning;
use crate::error::Hex;
use crate::error::LogoBitmapError;
use crate::units::SectorCount;

/// Byte offset of the security sector on the disk.
pub const SECURITY_SECTOR_OFFSET: u64 = 0x2000;
pub const SECURITY_SECTOR_SIZE: usize = 0x200;
/// Byte offset of the logo bitmap whose digest the security sector holds.
pub const LOGO_BITMAP_OFFSET: u64 = 0x2200;
pub const LOGO_BITMAP_SIZE: usize = 0xc00;
/// The bytes the signature covers: every field before it.
pub const SIGNED_LENGTH: usize = 0x5c;
/// The sector count the kernel rejects outright (`FIELD_OFFSET(SATA_DISK_SECURITY_SECTOR, Reserved)`).
pub(crate) const REJECTED_SECTOR_COUNT: u32 = 0x15c;

#[derive(Clone, PartialEq, Eq)]
pub struct SecuritySector {
	pub serial_number: FixedText<20>,
	pub firmware_revision: FixedText<8>,
	pub model_number: FixedText<40>,
	/// SHA-1 digest of the logo bitmap.
	pub logo_digest: [u8; 20],
	/// The sector count the kernel limits the disk to once the sector verifies.
	pub user_addressable_sectors: SectorCount,
	/// RSA signature over the first [`SIGNED_LENGTH`] bytes.
	pub signature: [u8; 0x100],
	signed_digest: [u8; 20],
}

impl SecuritySector {
	pub fn parse(bytes: &[u8; SECURITY_SECTOR_SIZE]) -> Self {
		Self {
			serial_number: FixedText::new(array(bytes, 0x00)),
			firmware_revision: FixedText::new(array(bytes, 0x14)),
			model_number: FixedText::new(array(bytes, 0x1c)),
			logo_digest: array(bytes, 0x44),
			// The kernel reads this field byte-reversed (`__loadwordbytereverse`).
			user_addressable_sectors: SectorCount::new(u32::from_le_bytes(array(bytes, 0x58))),
			signature: array(bytes, 0x5c),
			signed_digest: Sha1::digest(&bytes[..SIGNED_LENGTH]).into(),
		}
	}

	/// The SHA-1 digest of the signed bytes, which the signature must verify against.
	pub const fn signed_digest(&self) -> &[u8; 20] {
		&self.signed_digest
	}

	/// Whether the signature is all zero, as no signer would produce.
	pub fn is_signature_absent(&self) -> bool {
		self.signature.iter().all(|byte| *byte == 0)
	}

	/// Whether the drive strings are printable and the sector count can describe a retail disk,
	/// as a security sector written at manufacture does. Zeroed or random sectors fail.
	pub fn is_plausible(&self) -> bool {
		self.serial_number.is_printable()
			&& self.model_number.is_printable()
			&& self.user_addressable_sectors.get() >= crate::layout::RETAIL_PARTITION1_START.get()
	}

	/// The checks the kernel makes that need no key: the logo bitmap digest and the sector count
	/// against the `device` sectors that IDENTIFY reports (`SataDiskAuthenticateDevice`).
	pub fn check(&self, logo_bitmap: &[u8; LOGO_BITMAP_SIZE], device: SectorCount) -> Vec<DiskWarning> {
		let mut warnings = Vec::new();

		let actual: [u8; 20] = Sha1::digest(logo_bitmap).into();
		if actual != self.logo_digest {
			warnings.push(DiskWarning::LogoDigestMismatch { expected: self.logo_digest, actual });
		}

		let security = self.user_addressable_sectors;
		if security > device {
			warnings.push(DiskWarning::SecurityCountExceedsDevice { security, device });
		}
		if security.get() == REJECTED_SECTOR_COUNT {
			warnings.push(DiskWarning::SecurityCountRejected { security });
		}

		warnings
	}
}

impl fmt::Debug for SecuritySector {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("SecuritySector")
			.field("serial_number", &self.serial_number)
			.field("firmware_revision", &self.firmware_revision)
			.field("model_number", &self.model_number)
			.field("logo_digest", &format_args!("{}", Hex(&self.logo_digest)))
			.field("user_addressable_sectors", &self.user_addressable_sectors)
			.field("signature", &format_args!("{}", Hex(&self.signature)))
			.field("signed_digest", &format_args!("{}", Hex(&self.signed_digest)))
			.finish()
	}
}

/// The contents of the logo bitmap region at [`LOGO_BITMAP_OFFSET`]. The kernel only hashes the
/// whole region; on a development kit drive it held a big-endian u32 length followed by a PNG
/// image of that length, and zeros after it.
#[derive(Clone, PartialEq, Eq)]
pub struct LogoBitmap {
	image: Vec<u8>,
}

impl LogoBitmap {
	pub fn parse(region: &[u8; LOGO_BITMAP_SIZE]) -> Result<Self, LogoBitmapError> {
		let length = u32::from_be_bytes(array(region, 0));
		let available = LOGO_BITMAP_SIZE - 4;
		let image = usize::try_from(length)
			.ok()
			.filter(|length| *length <= available)
			.map(|length| region[4..4 + length].to_vec())
			.ok_or(LogoBitmapError::LengthPastRegion { length, available })?;

		Ok(Self { image })
	}

	/// The image bytes after the length.
	pub fn image(&self) -> &[u8] {
		&self.image
	}

	/// Whether the image starts with the PNG file signature.
	pub fn is_png(&self) -> bool {
		self.image.starts_with(b"\x89PNG\r\n\x1a\n")
	}
}

impl fmt::Debug for LogoBitmap {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("LogoBitmap").field("length", &self.image.len()).field("is_png", &self.is_png()).finish()
	}
}
