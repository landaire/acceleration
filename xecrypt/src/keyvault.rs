//! Xbox 360 keyvault parser.
//!
//! The keyvault is a 16KB (or 16KB-16 truncated) encrypted blob stored on
//! the console's flash, containing per-console cryptographic material:
//! serial numbers, RSA private keys, AES session keys, the DVD key, XSM3
//! device keys, and the signed console certificate.
//!
//! Both owned ([`KeyVault`]) and zerocopy ([`KeyVaultRef`]) variants are
//! provided. The zerocopy variant borrows from the input buffer without
//! copying large keys/certificates.
//!
//! The parser operates on the *decrypted* keyvault -- decryption happens
//! outside this module (the keyvault is wrapped in AES-CBC with an HMAC,
//! but users typically have already decrypted it via xeBuild or similar).
//!
//! # Example
//!
//! ```no_run
//! use xecrypt::keyvault::KeyVault;
//!
//! let data = std::fs::read("kv.bin")?;
//! let kv = KeyVault::parse(&data)?;
//!
//! println!("Console serial: {}", kv.console_serial());
//! println!("Part number:    {}", kv.console_certificate.console_part_number);
//! println!("Hardware rev:   {:?}", kv.revision());
//! println!("Is retail:      {}", kv.is_retail());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use bitflags::bitflags;
use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use std::io::Cursor;
use std::io::Read;
use std::ops::Range;
use thiserror::Error;

pub const KEYVAULT_SIZE_FULL: usize = 0x4000;
pub const KEYVAULT_SIZE_TRUNCATED: usize = 0x3FF0;
const DATA_START: usize = 0x18;

mod offsets {
	use std::ops::Range;

	pub const HMAC_SHA_HASH: Range<usize> = 0x00..0x10;
	pub const CONFOUNDER: Range<usize> = 0x10..0x20;

	pub const CONSOLE_SERIAL: Range<usize> = 0x98..0xA4;
	pub const MOBO_SERIAL: Range<usize> = 0xA4..0xB0;
	pub const GAME_REGION: Range<usize> = 0xB0..0xB2;

	pub const CONSOLE_OBFUSCATION_KEY: Range<usize> = 0xB8..0xC8;
	pub const KEY_OBFUSCATION_KEY: Range<usize> = 0xC8..0xD8;
	pub const ROAMABLE_OBFUSCATION_KEY: Range<usize> = 0xD8..0xE8;
	pub const DVD_KEY: Range<usize> = 0xE8..0xF8;
	pub const PRIMARY_ACTIVATION_KEY: Range<usize> = 0xF8..0x110;
	pub const SECONDARY_ACTIVATION_KEY: Range<usize> = 0x110..0x120;

	pub const GLOBAL_DEVICE_2DES_KEY1: Range<usize> = 0x120..0x130;
	pub const GLOBAL_DEVICE_2DES_KEY2: Range<usize> = 0x130..0x140;
	pub const WIRELESS_CONTROLLER_MS_2DES_KEY1: Range<usize> = 0x140..0x150;
	pub const WIRELESS_CONTROLLER_MS_2DES_KEY2: Range<usize> = 0x150..0x160;
	pub const WIRED_WEBCAM_MS_2DES_KEY: Range<usize> = 0x160..0x170;
	pub const WIRED_CONTROLLER_MS_2DES_KEY: Range<usize> = 0x170..0x180;
	pub const MEMORY_UNIT_MS_2DES_KEY: Range<usize> = 0x180..0x190;
	pub const OTHER_XSM3_DEVICE_MS_2DES_KEY: Range<usize> = 0x190..0x1A0;
	pub const WIRELESS_CONTROLLER_2DES_KEY1: Range<usize> = 0x1A0..0x1B0;
	pub const WIRELESS_CONTROLLER_2DES_KEY2: Range<usize> = 0x1B0..0x1C0;
	pub const WIRED_WEBCAM_2DES_KEY: Range<usize> = 0x1C0..0x1D0;
	pub const WIRED_CONTROLLER_2DES_KEY: Range<usize> = 0x1D0..0x1E0;
	pub const MEMORY_UNIT_2DES_KEY: Range<usize> = 0x1E0..0x1F0;
	pub const OTHER_XSM3_DEVICE_2DES_KEY: Range<usize> = 0x1F0..0x200;

	pub const CONSOLE_PRIVATE_KEY: Range<usize> = 0x0200..0x03D0;
	pub const XEIKA_PRIVATE_KEY: Range<usize> = 0x03D0..0x0760;
	pub const CARDEA_PRIVATE_KEY: Range<usize> = 0x0760..0x0930;

	pub const CONSOLE_CERTIFICATE: usize = 0x09B0;
	pub const CERT_CONSOLE_ID: Range<usize> = 0x02..0x07;
	pub const CERT_PART_NUMBER: Range<usize> = 0x07..0x12;
	pub const CERT_PRIVILEGES: Range<usize> = 0x12..0x16;
	pub const CERT_CONSOLE_TYPE: Range<usize> = 0x16..0x1A;
	pub const CERT_MFG_DATE: Range<usize> = 0x1E..0x26;

	pub const XEIKA_CERTIFICATE: Range<usize> = 0x0B58..0x1EE0;
	pub const CARDEA_CERTIFICATE: Range<usize> = 0x1EE0..0x3FE8;
}

#[derive(Error, Debug)]
pub enum KeyVaultError {
	#[error("invalid keyvault size: expected {expected}, got {got}")]
	InvalidSize { expected: usize, got: usize },
	#[error("I/O error: {0}")]
	Io(#[from] std::io::Error),
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct KeyVault {
	pub header: KeyVaultHeader,
	pub config: KeyVaultConfig,
	pub keys: KeyVaultKeys,
	pub console_certificate: ConsoleCertificate,
	pub xeika_certificate: Vec<u8>,
	pub cardea_certificate: Vec<u8>,
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct KeyVaultHeader {
	#[cfg_attr(feature = "serde", serde(skip))]
	pub hmac_sha_hash: [u8; 0x10],
	#[cfg_attr(feature = "serde", serde(skip))]
	pub confounder: [u8; 0x10],
}

bitflags! {
	/// Controls how `restricted_privileges` is applied by the HV (kv_data+0x02).
	///
	/// Checked by `sub_2e1d0` when restricted_privileges is updated, and by
	/// `sub_75b8` during `HvxKeysInitialize` for the devkit flag.
	#[derive(Debug, Clone, Copy, PartialEq, Eq)]
	pub struct PrivilegeRestrictionFlags: u8 {
		/// When set, `sub_2e1d0` zeroes restricted_privileges entirely,
		/// overriding any dynamic updates.
		const CLEAR_ALL       = 0x01;
		/// When set, `sub_2e1d0` ANDs the incoming privilege value with
		/// a stored mask (r2+0x162f0), limiting which bits can be set.
		const MASK_WITH_STORED = 0x02;
		/// Marks the console as a development kit. `sub_75b8` sets
		/// shared page bit 5, causing the entire HV to treat this
		/// console as a devkit for all privilege and security checks.
		const DEVKIT          = 0x04;
	}
}

#[cfg(feature = "serde")]
impl serde::Serialize for PrivilegeRestrictionFlags {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.serialize_u8(self.bits())
	}
}

bitflags! {
	/// Console-level privilege restrictions stored in the keyvault config
	/// at kv_data+0x18 (file offset 0x30).
	///
	/// The HV checks these in `sub_35b0`, which:
	/// 1. Returns all-ones (grant all) if the HV flags word bit 15 is clear
	/// 2. Returns the actual restricted_privileges from keyvault if the console
	///    is a devkit (console type bit 5 set)
	/// 3. Returns zero (deny all) otherwise (retail consoles)
	///
	/// `HvxKeysSaveSystemUpdate` calls `sub_35b0(0x1, 0)` to check bit 0.
	/// The field is updated by the HV via `sub_2e800` / `sub_2e1d0`, which
	/// applies a (clear_mask, set_mask) pair to the stored value.
	///
	/// The XEX loader (`sub_8007c4f0`) also checks this value: when
	/// `ImageFlags::KV_PRIVILEGES_REQUIRED` is set, optional header 0x4004
	/// contains a (mask, expected) u64 pair and the kernel verifies
	/// `(restricted_privileges & mask) == expected`. This allows individual
	/// XEX executables to require arbitrary privilege bit combinations.
	///
	/// Only bit 0 has been identified from HV callers. The remaining bits
	/// are defined per-XEX via header key 0x4004. On retail consoles the
	/// HV returns 0 for all checks (sub_35b0).
	#[derive(Debug, Clone, Copy, PartialEq, Eq)]
	pub struct RestrictedPrivileges: u64 {
		const ALLOW_SYSTEM_UPDATE = 0x0000_0000_0000_0001;
	}
}

#[cfg(feature = "serde")]
impl serde::Serialize for RestrictedPrivileges {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.serialize_u64(self.bits())
	}
}

bitflags! {
	/// Console certificate privilege flags at cert+0x12 (u32).
	///
	/// Part of the signed console certificate in the keyvault. Covered by
	/// the RSA signature over the certificate -- cannot be modified without
	/// re-signing with Microsoft's private key.
	///
	/// The HV and kernel do not directly mask individual bits of this field
	/// in the 17559 binaries. `XeKeysConsoleSignatureVerification` verifies
	/// the certificate's RSA signature and passes the entire cert (including
	/// privileges) to `XeKeysSetKey(0x1000, ...)` for storage, but no
	/// bit-level checks on this u32 were found.
	///
	/// All test keyvaults have privileges = 0, suggesting this field is
	/// reserved for future use or only meaningful on special certificates.
	#[derive(Debug, Clone, Copy, PartialEq, Eq)]
	pub struct CertificatePrivileges: u32 {
		const _ = !0;
	}
}

#[cfg(feature = "serde")]
impl serde::Serialize for CertificatePrivileges {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.serialize_u32(self.bits())
	}
}

bitflags! {
	/// Optical disc drive feature flags at kv_data+0x04 (u16).
	///
	/// Read in `HvxKeysInitialize` as part of keyvault validation.
	/// The HV at 0x77d8 checks the console cert's console_type bit 27
	/// (0x10000000) and uses OddFeatures to determine the ODD auth mode.
	/// At 0x7808-0x7820, bit 2 (0x4) of OddAuthType controls whether
	/// bit 27 of the console cert's type is cleared.
	///
	/// No individual OddFeatures bits were identified by name in the
	/// 17559 HV. The field is compared as a whole (e.g., == 0x0102) to
	/// select ODD authentication behavior.
	#[derive(Debug, Clone, Copy, PartialEq, Eq)]
	pub struct OddFeatures: u16 {
		const _ = !0;
	}
}

#[cfg(feature = "serde")]
impl serde::Serialize for OddFeatures {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.serialize_u16(self.bits())
	}
}

bitflags! {
	/// Optical disc drive authentication type at kv_data+0x06 (u16).
	///
	/// Stored in the keyvault config area. During `HvxKeysInitialize`, the
	/// HV copies this value to an internal data area (at `r2+0x6A9C`) and
	/// checks specific bits:
	///
	/// - Bit 2 (0x4): Checked in `HvxKeysInitialize` (0x7810). Controls
	///   whether bit 27 of the console cert's ConsoleType (ODD auth type
	///   extension) is cleared during initialization. When OddAuthType
	///   bit 2 is clear AND OddFeatures != 0x0102, the console type's
	///   bit 27 is forced off.
	/// - Bit 15 (0x8000): Checked in `sub_3100` (0x3188). When the MSB
	///   is clear, the HV calls `sub_3040` which configures hardware
	///   registers for ODD authentication (related to AP2.0/2.1 setup).
	///
	/// Note: the HV shared page halfword at offset 6 (read via `lhz rX,
	/// 6(0)` throughout the HV) is a SEPARATE runtime register populated
	/// from multiple sources during initialization. Its bits include a
	/// devkit flag (bit 5, from `restricted_privileges_flags` bit 2),
	/// update eligibility (bits 12-14), and lock status (bit 15). These
	/// are NOT the OddAuthType field itself, even though some older
	/// documentation conflates them.
	#[derive(Debug, Clone, Copy, PartialEq, Eq)]
	pub struct OddAuthType: u16 {
		/// ODD auth extension control. When clear (and OddFeatures !=
		/// 0x0102), `HvxKeysInitialize` clears bit 27 of the console
		/// cert's ConsoleType during key initialization.
		const ODD_AUTH_EXT_CONTROL = 0x0004;
		/// Allow remaining bits.
		const _ = !0;
	}
}

#[cfg(feature = "serde")]
impl serde::Serialize for OddAuthType {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.serialize_u16(self.bits())
	}
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct KeyVaultConfig {
	/// Manufacturing mode byte (kv_data+0x00). Set to 1 during factory
	/// provisioning. The HV checks this in `HvxKeysExCreateKeyVault` and
	/// `HvxKeysExLoadKeyVault` (bit 23 of flags word, mask 0x100).
	pub manufacturing_mode: u8,
	/// Alternate keyvault indicator (kv_data+0x01). Checked by
	/// `HvxKeysInitialize` at kv_ptr+0x19.
	pub alternate_key_vault: u8,
	/// Privilege restriction flags (kv_data+0x02, u8).
	///
	pub restricted_privileges_flags: PrivilegeRestrictionFlags,
	pub reserved_byte4: u8,
	pub odd_features: OddFeatures,
	pub odd_authtype: OddAuthType,
	pub restricted_hvext_loader: u32,
	pub policy_flash_size: u32,
	pub policy_builtin_usbmu_size: u32,
	pub reserved_dword4: u32,
	pub restricted_privileges: RestrictedPrivileges,
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_qword2: u64,
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_qword3: u64,
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_qword4: u64,
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_key1: [u8; 0x10],
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_key2: [u8; 0x10],
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_key3: [u8; 0x10],
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_key4: [u8; 0x10],
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_random_key1: [u8; 0x10],
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_random_key2: [u8; 0x10],
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct KeyVaultKeys {
	pub console_serial_number: String,
	pub mobo_serial_number: [u8; 0x0C],
	pub game_region: xenon_types::GameRegion,
	pub console_obfuscation_key: [u8; 0x10],
	pub key_obfuscation_key: [u8; 0x10],
	pub roamable_obfuscation_key: [u8; 0x10],
	pub dvd_key: [u8; 0x10],
	pub primary_activation_key: [u8; 0x18],
	pub secondary_activation_key: [u8; 0x10],
	pub global_device_2des_key1: [u8; 0x10],
	pub global_device_2des_key2: [u8; 0x10],
	pub wireless_controller_ms_2des_key1: [u8; 0x10],
	pub wireless_controller_ms_2des_key2: [u8; 0x10],
	pub wired_webcam_ms_2des_key: [u8; 0x10],
	pub wired_controller_ms_2des_key: [u8; 0x10],
	pub memory_unit_ms_2des_key: [u8; 0x10],
	pub other_xsm3_device_ms_2des_key: [u8; 0x10],
	pub wireless_controller_2des_key1: [u8; 0x10],
	pub wireless_controller_2des_key2: [u8; 0x10],
	pub wired_webcam_2des_key: [u8; 0x10],
	pub wired_controller_2des_key: [u8; 0x10],
	pub memory_unit_2des_key: [u8; 0x10],
	pub other_xsm3_device_2des_key: [u8; 0x10],
	pub console_private_key: Vec<u8>,
	pub xeika_private_key: Vec<u8>,
	pub cardea_private_key: Vec<u8>,
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ConsoleCertificate {
	pub cert_size: u16,
	pub console_id: xenon_types::ConsoleId,
	pub console_part_number: String,
	pub privileges: CertificatePrivileges,
	pub console_type: ConsoleType,
	pub manufacturing_date: String,
}

/// Console type identifier at cert+0x16 (u32).
///
/// Part of the signed console certificate. The HV and kernel verify
/// consistency of this field between the local certificate and any
/// incoming certificate during `XeKeysConsoleSignatureVerification`:
///
/// - **Retail path** (HV flags bit 15 clear): bits 0-23 must match
///   exactly between the two certificates.
/// - **Devkit path** (HV flags bit 15 set): bits 0-23 and bit 30
///   (TESTKIT) must match. Bits 24-29 may differ. Bit 31
///   (RECOVERY_GENERATED) is explicitly ignored in the comparison.
///
/// ## Bit layout
///
/// | Bits  | Mask         | Name                | Description |
/// |-------|--------------|---------------------|-------------|
/// | 0-1   | 0x0000_0003  | Console kind        | 1=devkit, 2=retail |
/// | 2-13  | 0x0000_3FFC  | Reserved            | Must match between certs |
/// | 14    | 0x0000_4000  | (unknown)           | Seen set in test keyvaults. Must match between certs. |
/// | 15-23 | 0x00FF_8000  | Reserved            | Must match between certs |
/// | 24-26 | 0x0700_0000  | (variable)          | May differ on devkit; must match on retail |
/// | 27    | 0x0800_0000  | ODD auth type       | Checked in `HvxKeysInitialize`. May be cleared based on OddAuthType bit 2. |
/// | 28    | 0x1000_0000  | (variable)          | Checked in `HvxKeysInitialize` (0x77d8). May differ on devkit. |
/// | 29    | 0x2000_0000  | (reserved)          | May differ on devkit |
/// | 30    | 0x4000_0000  | Testkit             | Console is a test kit. Must match between certs on both retail and devkit. |
/// | 31    | 0x8000_0000  | Recovery generated  | Certificate was recovery-generated. Selects key 0x3a instead of 0x3c for verification. Ignored in cert comparison. |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ConsoleType(pub u32);

impl ConsoleType {
	/// Console kind mask (bits 0-1).
	pub const KIND_MASK: u32 = 0x0000_0003;
	/// Bit 27: ODD authentication type. Checked and potentially cleared in
	/// `HvxKeysInitialize` based on OddAuthType bit 2.
	pub const ODD_AUTH_TYPE: u32 = 0x0800_0000;
	/// Bit 30: Console is a test kit.
	pub const TESTKIT: u32 = 0x4000_0000;
	/// Bit 31: Certificate was recovery-generated. When set, the HV uses
	/// key ID 0x3a (recovery console cert key) instead of 0x3c (normal
	/// console cert key) for RSA signature verification.
	pub const RECOVERY_GENERATED: u32 = 0x8000_0000;

	pub fn kind(&self) -> ConsoleKind {
		match self.0 & Self::KIND_MASK {
			1 => ConsoleKind::Devkit,
			2 => ConsoleKind::Retail,
			_ => ConsoleKind::Unknown,
		}
	}

	pub fn is_devkit(&self) -> bool {
		self.0 & Self::KIND_MASK == 1
	}

	pub fn is_retail(&self) -> bool {
		self.0 & Self::KIND_MASK == 2
	}

	pub fn is_testkit(&self) -> bool {
		self.0 & Self::TESTKIT != 0
	}

	pub fn is_recovery_generated(&self) -> bool {
		self.0 & Self::RECOVERY_GENERATED != 0
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ConsoleKind {
	Devkit,
	Retail,
	Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ConsoleRevision {
	/// Original Xbox 360 (2005). 90nm CPU/GPU.
	Xenon,
	/// Second revision (2007). 80nm GPU, HDMI added.
	Zephyr,
	/// Third revision (2007). 65nm CPU.
	Falcon,
	/// Fourth revision (2008). 65nm CPU, 65nm GPU.
	Jasper,
	/// Xbox 360 S "Slim" (2010). CPU+GPU combined into single die.
	Trinity,
	/// Xbox 360 S revision (2011). Smaller combined die.
	Corona,
	/// Xbox 360 E (2013). Final hardware revision.
	Winchester,
	Unknown,
}

impl ConsoleCertificate {
	pub fn revision(&self) -> ConsoleRevision {
		revision_from_part_number(&self.console_part_number)
	}
}

impl KeyVault {
	pub fn parse(data: impl AsRef<[u8]>) -> Result<Self, KeyVaultError> {
		Self::parse_inner(data.as_ref())
	}

	fn parse_inner(data: &[u8]) -> Result<Self, KeyVaultError> {
		if data.len() != KEYVAULT_SIZE_FULL && data.len() != KEYVAULT_SIZE_TRUNCATED {
			return Err(KeyVaultError::InvalidSize { expected: KEYVAULT_SIZE_FULL, got: data.len() });
		}

		let header = KeyVaultHeader::parse(data)?;
		let kv = &data[DATA_START..];
		let config = KeyVaultConfig::parse(kv)?;
		let keys = KeyVaultKeys::parse(kv)?;
		let console_certificate = ConsoleCertificate::parse(&kv[offsets::CONSOLE_CERTIFICATE..])?;

		let xeika_end = std::cmp::min(offsets::XEIKA_CERTIFICATE.end, kv.len());
		let xeika_certificate = kv[offsets::XEIKA_CERTIFICATE.start..xeika_end].to_vec();

		let cardea_end = std::cmp::min(offsets::CARDEA_CERTIFICATE.end, kv.len());
		let cardea_certificate = kv[offsets::CARDEA_CERTIFICATE.start..cardea_end].to_vec();

		Ok(KeyVault { header, config, keys, console_certificate, xeika_certificate, cardea_certificate })
	}

	pub fn console_id(&self) -> &xenon_types::ConsoleId {
		&self.console_certificate.console_id
	}

	pub fn console_serial(&self) -> &str {
		&self.keys.console_serial_number
	}

	pub fn dvd_key(&self) -> &[u8; 0x10] {
		&self.keys.dvd_key
	}

	pub fn game_region(&self) -> xenon_types::GameRegion {
		self.keys.game_region
	}

	pub fn console_type(&self) -> &ConsoleType {
		&self.console_certificate.console_type
	}

	pub fn is_devkit(&self) -> bool {
		self.console_certificate.console_type.is_devkit()
	}

	pub fn is_retail(&self) -> bool {
		self.console_certificate.console_type.is_retail()
	}

	pub fn revision(&self) -> ConsoleRevision {
		self.console_certificate.revision()
	}
}

/// Zerocopy keyvault view that borrows the underlying data.
///
/// Validates and parses the structure upfront, storing parsed scalars
/// and validated sub-structure references. Variable-length and large
/// fields (keys, certificates) are borrowed from the original buffer.
#[derive(Debug)]
pub struct KeyVaultRef<'a> {
	pub hmac_sha_hash: &'a [u8; 0x10],
	pub confounder: &'a [u8; 0x10],
	pub config: KeyVaultConfig,
	pub keys: KeyVaultKeysRef<'a>,
	pub console_certificate: ConsoleCertificateRef<'a>,
	pub xeika_certificate: &'a [u8],
	pub cardea_certificate: &'a [u8],
}

#[derive(Debug)]
pub struct KeyVaultKeysRef<'a> {
	pub console_serial_number: &'a str,
	pub mobo_serial_number: &'a [u8; 0x0C],
	pub game_region: xenon_types::GameRegion,
	pub console_obfuscation_key: &'a [u8; 0x10],
	pub key_obfuscation_key: &'a [u8; 0x10],
	pub roamable_obfuscation_key: &'a [u8; 0x10],
	pub dvd_key: &'a [u8; 0x10],
	pub primary_activation_key: &'a [u8; 0x18],
	pub secondary_activation_key: &'a [u8; 0x10],
	pub global_device_2des_key1: &'a [u8; 0x10],
	pub global_device_2des_key2: &'a [u8; 0x10],
	pub wireless_controller_ms_2des_key1: &'a [u8; 0x10],
	pub wireless_controller_ms_2des_key2: &'a [u8; 0x10],
	pub wired_webcam_ms_2des_key: &'a [u8; 0x10],
	pub wired_controller_ms_2des_key: &'a [u8; 0x10],
	pub memory_unit_ms_2des_key: &'a [u8; 0x10],
	pub other_xsm3_device_ms_2des_key: &'a [u8; 0x10],
	pub wireless_controller_2des_key1: &'a [u8; 0x10],
	pub wireless_controller_2des_key2: &'a [u8; 0x10],
	pub wired_webcam_2des_key: &'a [u8; 0x10],
	pub wired_controller_2des_key: &'a [u8; 0x10],
	pub memory_unit_2des_key: &'a [u8; 0x10],
	pub other_xsm3_device_2des_key: &'a [u8; 0x10],
	pub console_private_key: &'a [u8],
	pub xeika_private_key: &'a [u8],
	pub cardea_private_key: &'a [u8],
}

#[derive(Debug)]
pub struct ConsoleCertificateRef<'a> {
	pub cert_size: u16,
	pub console_id: xenon_types::ConsoleId,
	pub console_part_number: &'a str,
	pub privileges: CertificatePrivileges,
	pub console_type: ConsoleType,
	pub manufacturing_date: &'a str,
}

impl<'a> KeyVaultRef<'a> {
	pub fn parse(data: &'a [u8]) -> Result<Self, KeyVaultError> {
		if data.len() != KEYVAULT_SIZE_FULL && data.len() != KEYVAULT_SIZE_TRUNCATED {
			return Err(KeyVaultError::InvalidSize { expected: KEYVAULT_SIZE_FULL, got: data.len() });
		}

		let kv = &data[DATA_START..];

		let config = KeyVaultConfig::parse(kv)?;

		let serial_raw = &kv[offsets::CONSOLE_SERIAL];
		let serial_end = serial_raw.iter().position(|b| *b == 0).unwrap_or(serial_raw.len());
		let console_serial_number = std::str::from_utf8(&serial_raw[..serial_end])
			.map_err(|_| KeyVaultError::InvalidSize { expected: KEYVAULT_SIZE_FULL, got: data.len() })?;

		let keys = KeyVaultKeysRef {
			console_serial_number,
			mobo_serial_number: kv[offsets::MOBO_SERIAL].try_into().unwrap(),
			game_region: xenon_types::GameRegion::from_bits_retain(u16::from_be_bytes(
				kv[offsets::GAME_REGION].try_into().unwrap(),
			) as u32),
			console_obfuscation_key: kv[offsets::CONSOLE_OBFUSCATION_KEY].try_into().unwrap(),
			key_obfuscation_key: kv[offsets::KEY_OBFUSCATION_KEY].try_into().unwrap(),
			roamable_obfuscation_key: kv[offsets::ROAMABLE_OBFUSCATION_KEY].try_into().unwrap(),
			dvd_key: kv[offsets::DVD_KEY].try_into().unwrap(),
			primary_activation_key: kv[offsets::PRIMARY_ACTIVATION_KEY].try_into().unwrap(),
			secondary_activation_key: kv[offsets::SECONDARY_ACTIVATION_KEY].try_into().unwrap(),
			global_device_2des_key1: kv[offsets::GLOBAL_DEVICE_2DES_KEY1].try_into().unwrap(),
			global_device_2des_key2: kv[offsets::GLOBAL_DEVICE_2DES_KEY2].try_into().unwrap(),
			wireless_controller_ms_2des_key1: kv[offsets::WIRELESS_CONTROLLER_MS_2DES_KEY1].try_into().unwrap(),
			wireless_controller_ms_2des_key2: kv[offsets::WIRELESS_CONTROLLER_MS_2DES_KEY2].try_into().unwrap(),
			wired_webcam_ms_2des_key: kv[offsets::WIRED_WEBCAM_MS_2DES_KEY].try_into().unwrap(),
			wired_controller_ms_2des_key: kv[offsets::WIRED_CONTROLLER_MS_2DES_KEY].try_into().unwrap(),
			memory_unit_ms_2des_key: kv[offsets::MEMORY_UNIT_MS_2DES_KEY].try_into().unwrap(),
			other_xsm3_device_ms_2des_key: kv[offsets::OTHER_XSM3_DEVICE_MS_2DES_KEY].try_into().unwrap(),
			wireless_controller_2des_key1: kv[offsets::WIRELESS_CONTROLLER_2DES_KEY1].try_into().unwrap(),
			wireless_controller_2des_key2: kv[offsets::WIRELESS_CONTROLLER_2DES_KEY2].try_into().unwrap(),
			wired_webcam_2des_key: kv[offsets::WIRED_WEBCAM_2DES_KEY].try_into().unwrap(),
			wired_controller_2des_key: kv[offsets::WIRED_CONTROLLER_2DES_KEY].try_into().unwrap(),
			memory_unit_2des_key: kv[offsets::MEMORY_UNIT_2DES_KEY].try_into().unwrap(),
			other_xsm3_device_2des_key: kv[offsets::OTHER_XSM3_DEVICE_2DES_KEY].try_into().unwrap(),
			console_private_key: &kv[offsets::CONSOLE_PRIVATE_KEY],
			xeika_private_key: &kv[offsets::XEIKA_PRIVATE_KEY],
			cardea_private_key: &kv[offsets::CARDEA_PRIVATE_KEY],
		};

		let cert = &kv[offsets::CONSOLE_CERTIFICATE..];
		let cert_size = u16::from_be_bytes([cert[0], cert[1]]);
		let pn_raw = &cert[offsets::CERT_PART_NUMBER];
		let pn_end = pn_raw.iter().position(|b| *b == 0).unwrap_or(pn_raw.len());
		let date_raw = &cert[offsets::CERT_MFG_DATE];
		let date_end = date_raw.iter().position(|b| *b == 0).unwrap_or(date_raw.len());

		let console_certificate = ConsoleCertificateRef {
			cert_size,
			console_id: xenon_types::ConsoleId(cert[offsets::CERT_CONSOLE_ID].try_into().unwrap()),
			console_part_number: std::str::from_utf8(&pn_raw[..pn_end]).unwrap_or(""),
			privileges: CertificatePrivileges::from_bits_retain(u32::from_be_bytes(
				cert[offsets::CERT_PRIVILEGES].try_into().unwrap(),
			)),
			console_type: ConsoleType(u32::from_be_bytes(cert[offsets::CERT_CONSOLE_TYPE].try_into().unwrap())),
			manufacturing_date: std::str::from_utf8(&date_raw[..date_end]).unwrap_or(""),
		};

		let xeika_end = std::cmp::min(offsets::XEIKA_CERTIFICATE.end, kv.len());
		let cardea_end = std::cmp::min(offsets::CARDEA_CERTIFICATE.end, kv.len());

		Ok(KeyVaultRef {
			hmac_sha_hash: data[offsets::HMAC_SHA_HASH].try_into().unwrap(),
			confounder: data[offsets::CONFOUNDER].try_into().unwrap(),
			config,
			keys,
			console_certificate,
			xeika_certificate: &kv[offsets::XEIKA_CERTIFICATE.start..xeika_end],
			cardea_certificate: &kv[offsets::CARDEA_CERTIFICATE.start..cardea_end],
		})
	}

	pub fn into_owned(self) -> KeyVault {
		KeyVault {
			header: KeyVaultHeader { hmac_sha_hash: *self.hmac_sha_hash, confounder: *self.confounder },
			config: self.config,
			keys: KeyVaultKeys {
				console_serial_number: self.keys.console_serial_number.to_owned(),
				mobo_serial_number: *self.keys.mobo_serial_number,
				game_region: self.keys.game_region,
				console_obfuscation_key: *self.keys.console_obfuscation_key,
				key_obfuscation_key: *self.keys.key_obfuscation_key,
				roamable_obfuscation_key: *self.keys.roamable_obfuscation_key,
				dvd_key: *self.keys.dvd_key,
				primary_activation_key: *self.keys.primary_activation_key,
				secondary_activation_key: *self.keys.secondary_activation_key,
				global_device_2des_key1: *self.keys.global_device_2des_key1,
				global_device_2des_key2: *self.keys.global_device_2des_key2,
				wireless_controller_ms_2des_key1: *self.keys.wireless_controller_ms_2des_key1,
				wireless_controller_ms_2des_key2: *self.keys.wireless_controller_ms_2des_key2,
				wired_webcam_ms_2des_key: *self.keys.wired_webcam_ms_2des_key,
				wired_controller_ms_2des_key: *self.keys.wired_controller_ms_2des_key,
				memory_unit_ms_2des_key: *self.keys.memory_unit_ms_2des_key,
				other_xsm3_device_ms_2des_key: *self.keys.other_xsm3_device_ms_2des_key,
				wireless_controller_2des_key1: *self.keys.wireless_controller_2des_key1,
				wireless_controller_2des_key2: *self.keys.wireless_controller_2des_key2,
				wired_webcam_2des_key: *self.keys.wired_webcam_2des_key,
				wired_controller_2des_key: *self.keys.wired_controller_2des_key,
				memory_unit_2des_key: *self.keys.memory_unit_2des_key,
				other_xsm3_device_2des_key: *self.keys.other_xsm3_device_2des_key,
				console_private_key: self.keys.console_private_key.to_vec(),
				xeika_private_key: self.keys.xeika_private_key.to_vec(),
				cardea_private_key: self.keys.cardea_private_key.to_vec(),
			},
			console_certificate: ConsoleCertificate {
				cert_size: self.console_certificate.cert_size,
				console_id: self.console_certificate.console_id,
				console_part_number: self.console_certificate.console_part_number.to_owned(),
				privileges: self.console_certificate.privileges,
				console_type: self.console_certificate.console_type,
				manufacturing_date: self.console_certificate.manufacturing_date.to_owned(),
			},
			xeika_certificate: self.xeika_certificate.to_vec(),
			cardea_certificate: self.cardea_certificate.to_vec(),
		}
	}

	pub fn console_id(&self) -> &xenon_types::ConsoleId {
		&self.console_certificate.console_id
	}

	pub fn console_serial(&self) -> &str {
		self.keys.console_serial_number
	}

	pub fn dvd_key(&self) -> &[u8; 0x10] {
		self.keys.dvd_key
	}

	pub fn game_region(&self) -> xenon_types::GameRegion {
		self.keys.game_region
	}

	pub fn console_type(&self) -> ConsoleType {
		self.console_certificate.console_type
	}

	pub fn is_devkit(&self) -> bool {
		self.console_certificate.console_type.is_devkit()
	}

	pub fn is_retail(&self) -> bool {
		self.console_certificate.console_type.is_retail()
	}

	pub fn revision(&self) -> ConsoleRevision {
		revision_from_part_number(self.console_certificate.console_part_number)
	}
}

impl<'a> ConsoleCertificateRef<'a> {
	pub fn revision(&self) -> ConsoleRevision {
		revision_from_part_number(self.console_part_number)
	}
}

fn revision_from_part_number(pn: &str) -> ConsoleRevision {
	if pn.starts_with("X803") || pn.starts_with("X800") {
		if pn.contains("955") || pn.contains("953") {
			return ConsoleRevision::Xenon;
		}
		if pn.contains("885") || pn.contains("878") {
			return ConsoleRevision::Zephyr;
		}
	}
	if pn.starts_with("X804") || pn.starts_with("X810") {
		return ConsoleRevision::Falcon;
	}
	if pn.starts_with("X811") || pn.starts_with("X812") || pn.starts_with("X815") {
		return ConsoleRevision::Jasper;
	}
	if pn.starts_with("X816") || pn.starts_with("X818") {
		return ConsoleRevision::Trinity;
	}
	if pn.starts_with("X819") || pn.starts_with("X820") || pn.starts_with("X850") {
		return ConsoleRevision::Corona;
	}
	if pn.starts_with("X851") || pn.starts_with("X852") || pn.starts_with("X86") {
		return ConsoleRevision::Winchester;
	}
	if pn.starts_with("004") {
		return ConsoleRevision::Xenon;
	}
	ConsoleRevision::Unknown
}

impl KeyVaultHeader {
	fn parse(data: &[u8]) -> Result<Self, KeyVaultError> {
		let mut hmac_sha_hash = [0u8; 0x10];
		hmac_sha_hash.copy_from_slice(&data[offsets::HMAC_SHA_HASH]);
		let mut confounder = [0u8; 0x10];
		confounder.copy_from_slice(&data[offsets::CONFOUNDER]);
		Ok(KeyVaultHeader { hmac_sha_hash, confounder })
	}
}

impl KeyVaultConfig {
	fn parse(data: &[u8]) -> Result<Self, KeyVaultError> {
		let mut c = Cursor::new(data);
		let manufacturing_mode = c.read_u8()?;
		let alternate_key_vault = c.read_u8()?;
		let restricted_privileges_flags = PrivilegeRestrictionFlags::from_bits_retain(c.read_u8()?);
		let reserved_byte4 = c.read_u8()?;
		let odd_features = OddFeatures::from_bits_retain(c.read_u16::<BigEndian>()?);
		let odd_authtype = OddAuthType::from_bits_retain(c.read_u16::<BigEndian>()?);
		let restricted_hvext_loader = c.read_u32::<BigEndian>()?;
		let policy_flash_size = c.read_u32::<BigEndian>()?;
		let policy_builtin_usbmu_size = c.read_u32::<BigEndian>()?;
		let reserved_dword4 = c.read_u32::<BigEndian>()?;
		let restricted_privileges = RestrictedPrivileges::from_bits_retain(c.read_u64::<BigEndian>()?);

		fn read_qword(c: &mut Cursor<&[u8]>) -> Result<u64, std::io::Error> {
			c.read_u64::<BigEndian>()
		}
		fn read_key(c: &mut Cursor<&[u8]>) -> Result<[u8; 0x10], std::io::Error> {
			let mut buf = [0u8; 0x10];
			c.read_exact(&mut buf)?;
			Ok(buf)
		}

		Ok(KeyVaultConfig {
			manufacturing_mode,
			alternate_key_vault,
			restricted_privileges_flags,
			reserved_byte4,
			odd_features,
			odd_authtype,
			restricted_hvext_loader,
			policy_flash_size,
			policy_builtin_usbmu_size,
			reserved_dword4,
			restricted_privileges,
			reserved_qword2: read_qword(&mut c)?,
			reserved_qword3: read_qword(&mut c)?,
			reserved_qword4: read_qword(&mut c)?,
			reserved_key1: read_key(&mut c)?,
			reserved_key2: read_key(&mut c)?,
			reserved_key3: read_key(&mut c)?,
			reserved_key4: read_key(&mut c)?,
			reserved_random_key1: read_key(&mut c)?,
			reserved_random_key2: read_key(&mut c)?,
		})
	}
}

impl KeyVaultKeys {
	fn parse(data: &[u8]) -> Result<Self, KeyVaultError> {
		fn read_range<const N: usize>(data: &[u8], range: Range<usize>) -> [u8; N] {
			let mut buf = [0u8; N];
			buf.copy_from_slice(&data[range]);
			buf
		}

		let serial_raw = &data[offsets::CONSOLE_SERIAL];
		let serial_end = serial_raw.iter().position(|b| *b == 0).unwrap_or(serial_raw.len());
		let console_serial_number = String::from_utf8_lossy(&serial_raw[..serial_end]).into_owned();

		let game_region = xenon_types::GameRegion::from_bits_retain(u16::from_be_bytes(
			data[offsets::GAME_REGION].try_into().unwrap(),
		) as u32);

		Ok(KeyVaultKeys {
			console_serial_number,
			mobo_serial_number: read_range(data, offsets::MOBO_SERIAL),
			game_region,
			console_obfuscation_key: read_range(data, offsets::CONSOLE_OBFUSCATION_KEY),
			key_obfuscation_key: read_range(data, offsets::KEY_OBFUSCATION_KEY),
			roamable_obfuscation_key: read_range(data, offsets::ROAMABLE_OBFUSCATION_KEY),
			dvd_key: read_range(data, offsets::DVD_KEY),
			primary_activation_key: read_range(data, offsets::PRIMARY_ACTIVATION_KEY),
			secondary_activation_key: read_range(data, offsets::SECONDARY_ACTIVATION_KEY),
			global_device_2des_key1: read_range(data, offsets::GLOBAL_DEVICE_2DES_KEY1),
			global_device_2des_key2: read_range(data, offsets::GLOBAL_DEVICE_2DES_KEY2),
			wireless_controller_ms_2des_key1: read_range(data, offsets::WIRELESS_CONTROLLER_MS_2DES_KEY1),
			wireless_controller_ms_2des_key2: read_range(data, offsets::WIRELESS_CONTROLLER_MS_2DES_KEY2),
			wired_webcam_ms_2des_key: read_range(data, offsets::WIRED_WEBCAM_MS_2DES_KEY),
			wired_controller_ms_2des_key: read_range(data, offsets::WIRED_CONTROLLER_MS_2DES_KEY),
			memory_unit_ms_2des_key: read_range(data, offsets::MEMORY_UNIT_MS_2DES_KEY),
			other_xsm3_device_ms_2des_key: read_range(data, offsets::OTHER_XSM3_DEVICE_MS_2DES_KEY),
			wireless_controller_2des_key1: read_range(data, offsets::WIRELESS_CONTROLLER_2DES_KEY1),
			wireless_controller_2des_key2: read_range(data, offsets::WIRELESS_CONTROLLER_2DES_KEY2),
			wired_webcam_2des_key: read_range(data, offsets::WIRED_WEBCAM_2DES_KEY),
			wired_controller_2des_key: read_range(data, offsets::WIRED_CONTROLLER_2DES_KEY),
			memory_unit_2des_key: read_range(data, offsets::MEMORY_UNIT_2DES_KEY),
			other_xsm3_device_2des_key: read_range(data, offsets::OTHER_XSM3_DEVICE_2DES_KEY),
			console_private_key: data[offsets::CONSOLE_PRIVATE_KEY].to_vec(),
			xeika_private_key: data[offsets::XEIKA_PRIVATE_KEY].to_vec(),
			cardea_private_key: data[offsets::CARDEA_PRIVATE_KEY].to_vec(),
		})
	}
}

impl ConsoleCertificate {
	fn parse(data: &[u8]) -> Result<Self, KeyVaultError> {
		let mut c = Cursor::new(data);
		let cert_size = c.read_u16::<BigEndian>()?;
		let mut console_id_raw = [0u8; 5];
		c.read_exact(&mut console_id_raw)?;
		let console_id = xenon_types::ConsoleId(console_id_raw);

		let mut part_number = [0u8; 0x0B];
		c.read_exact(&mut part_number)?;
		let pn_end = part_number.iter().position(|b| *b == 0).unwrap_or(part_number.len());
		let console_part_number = String::from_utf8_lossy(&part_number[..pn_end]).into_owned();

		let privileges = CertificatePrivileges::from_bits_retain(c.read_u32::<BigEndian>()?);
		let console_type = ConsoleType(c.read_u32::<BigEndian>()?);

		let mut date_raw = [0u8; 0x08];
		c.read_exact(&mut date_raw)?;
		let date_end = date_raw.iter().position(|b| *b == 0).unwrap_or(date_raw.len());
		let manufacturing_date = String::from_utf8_lossy(&date_raw[..date_end]).into_owned();

		Ok(ConsoleCertificate {
			cert_size,
			console_id,
			console_part_number,
			privileges,
			console_type,
			manufacturing_date,
		})
	}
}
