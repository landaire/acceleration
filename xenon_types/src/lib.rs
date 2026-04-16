//! Shared newtypes and utilities for Xbox 360 file formats.
//!
//! This crate provides common identifiers and data types used across
//! [`xex2`](https://docs.rs/xex2), [`xcontent`](https://docs.rs/xcontent),
//! [`xecrypt`](https://docs.rs/xecrypt), and other Xbox 360 crates. Keeping
//! them here avoids type-mismatch friction between crates (e.g. xcontent's
//! `MediaId` is the same type as xex2's `MediaId`).
//!
//! # Features
//!
//! - `jiff` (default off) -- enables [`filetime_to_timestamp`] for converting
//!   Windows FILETIME values to `jiff::Timestamp`.

mod serde_hex;

use bitflags::bitflags;
use serde::Deserialize;
use serde::Serialize;
use serde::Serializer;

/// Xbox 360 title identifier (32-bit).
///
/// The high 16 bits encode the publisher (e.g. `0x4D53` = Microsoft, `0x5351`
/// = Square Enix). The low 16 bits identify the game within that publisher.
/// Display formats as uppercase hex: `4D53885C`.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TitleId(pub u32);

/// Per-installation media identifier (32-bit).
///
/// Unique per copy of a game -- two physical copies of the same title have
/// different media IDs. Used for anti-piracy checks.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MediaId(pub u32);

/// Savegame/content identifier (32-bit).
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SavegameId(pub u32);

/// 5-byte console identifier from the keyvault.
///
/// Unique per console. Used in CON-signed content packages to bind them
/// to the specific console that created them.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConsoleId(#[serde(with = "serde_hex::fixed5")] pub [u8; 5]);

/// 8-byte Xbox LIVE profile identifier (XUID).
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileId(#[serde(with = "serde_hex::fixed8")] pub [u8; 8]);

/// 20-byte device identifier used for HDD/MU/USB device binding.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(#[serde(with = "serde_hex::fixed20")] pub [u8; 0x14]);

/// 32-bit virtual address in PowerPC address space (Xbox 360 is big-endian PPC).
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VirtualAddress(pub u32);

/// An absolute offset into an on-disk file (XEX, STFS, etc.). Distinguishes
/// file positions from in-memory sizes/indices, and from virtual addresses.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FileOffset(pub u64);

impl FileOffset {
	pub const ZERO: Self = Self(0);

	pub fn get(self) -> u64 {
		self.0
	}

	pub fn as_usize(self) -> usize {
		self.0 as usize
	}
}

impl From<usize> for FileOffset {
	fn from(v: usize) -> Self {
		Self(v as u64)
	}
}

impl From<u32> for FileOffset {
	fn from(v: u32) -> Self {
		Self(v as u64)
	}
}

impl From<u64> for FileOffset {
	fn from(v: u64) -> Self {
		Self(v)
	}
}

impl std::ops::Add<usize> for FileOffset {
	type Output = Self;
	fn add(self, rhs: usize) -> Self {
		Self(self.0 + rhs as u64)
	}
}

impl std::fmt::Display for FileOffset {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{:#x}", self.0)
	}
}

/// 128-bit AES key.
#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AesKey(pub [u8; 16]);

/// Xbox 360 version format (major.minor.build.revision).
///
/// Packed into a single u32 for storage: `[major:4][minor:4][build:16][revision:8]`.
///
/// # Example
///
/// ```
/// use xenon_types::Version;
///
/// let v = Version::from(0x20247000u32);
/// assert_eq!(v.major, 2);
/// assert_eq!(v.minor, 0);
/// assert_eq!(v.build, 0x2470);
/// assert_eq!(v.to_string(), "2.0.9328.0");
/// ```
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
	pub major: u16,
	pub minor: u16,
	pub build: u16,
	pub revision: u16,
}

impl std::fmt::Display for Version {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}.{}.{}.{}", self.major, self.minor, self.build, self.revision)
	}
}

impl From<u32> for Version {
	fn from(input: u32) -> Self {
		Version {
			major: ((input & 0xF000_0000) >> 28) as u16,
			minor: ((input & 0x0F00_0000) >> 24) as u16,
			build: ((input & 0x00FF_FF00) >> 8) as u16,
			revision: (input & 0xFF) as u16,
		}
	}
}

impl From<Version> for u32 {
	fn from(v: Version) -> Self {
		((v.major as u32) << 28) | ((v.minor as u32) << 24) | ((v.build as u32) << 8) | (v.revision as u32)
	}
}

impl std::fmt::Display for ConsoleId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		for b in &self.0 {
			write!(f, "{:02x}", b)?;
		}
		Ok(())
	}
}

impl std::fmt::Display for ProfileId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		for b in &self.0 {
			write!(f, "{:02x}", b)?;
		}
		Ok(())
	}
}

impl std::fmt::Display for DeviceId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		for b in &self.0 {
			write!(f, "{:02x}", b)?;
		}
		Ok(())
	}
}

bitflags! {
	/// Game region bitmask. Used in both keyvault (u16) and XEX ImageInfo (u32).
	///
	/// The console's keyvault stores which regions it supports. XEX executables
	/// store which regions they're allowed to run in. The kernel ANDs them
	/// together during load.
	#[derive(Debug, Clone, Copy, PartialEq, Eq)]
	pub struct GameRegion: u32 {
		const NTSC_US = 0x000000FF;
		const NTSC_JP = 0x0000FF00;
		const PAL     = 0x00FE0000;
		const PAL_AU  = 0x01000000;
		const ALL     = 0x01FEFFFF;
	}
}

impl Serialize for GameRegion {
	fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.serialize_u32(self.bits())
	}
}

impl<'de> Deserialize<'de> for GameRegion {
	fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		Ok(GameRegion::from_bits_retain(u32::deserialize(d)?))
	}
}

/// Windows FILETIME: 100-nanosecond intervals since 1601-01-01 00:00:00 UTC.
///
/// The Xbox 360 stores FILETIME values with each u32 half in little-endian
/// byte order, arranged as (high, low). Use [`filetime_from_xe_bytes`] to
/// decode from raw XEX header bytes.
pub const FILETIME_UNIX_EPOCH_DELTA: u64 = 116_444_736_000_000_000;

/// Decode a FILETIME from Xbox 360 on-disk format: two little-endian u32s
/// stored as (high, low).
pub fn filetime_from_xe_bytes(bytes: &[u8; 8]) -> u64 {
	let hi = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
	let lo = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
	((hi as u64) << 32) | lo as u64
}

/// Convert a Windows FILETIME to a Unix timestamp in seconds.
pub fn filetime_to_unix_secs(ft: u64) -> Option<i64> {
	ft.checked_sub(FILETIME_UNIX_EPOCH_DELTA).map(|v| (v / 10_000_000) as i64)
}

/// Convert a Windows FILETIME to a `jiff::Timestamp`.
#[cfg(feature = "jiff")]
pub fn filetime_to_timestamp(ft: u64) -> Option<jiff::Timestamp> {
	let unix_secs = filetime_to_unix_secs(ft)?;
	let nanos_remainder = ((ft - FILETIME_UNIX_EPOCH_DELTA) % 10_000_000) * 100;
	jiff::Timestamp::new(unix_secs, nanos_remainder as i32).ok()
}

macro_rules! impl_u32_hex_display {
	($ty:ty) => {
		impl std::fmt::Display for $ty {
			fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				write!(f, "{:08X}", self.0)
			}
		}

		impl std::fmt::LowerHex for $ty {
			fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				write!(f, "{:08x}", self.0)
			}
		}

		impl std::fmt::UpperHex for $ty {
			fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				write!(f, "{:08X}", self.0)
			}
		}
	};
}

impl_u32_hex_display!(TitleId);
impl_u32_hex_display!(MediaId);
impl_u32_hex_display!(SavegameId);

impl std::fmt::LowerHex for VirtualAddress {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{:08x}", self.0)
	}
}

impl std::fmt::UpperHex for VirtualAddress {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{:08X}", self.0)
	}
}
