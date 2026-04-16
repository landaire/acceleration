use bitflags::bitflags;
use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use num_enum::TryFromPrimitive;
#[cfg(feature = "serde")]
use serde::Serialize;
#[cfg(feature = "serde")]
use serde::Serializer;
use std::io::Cursor;
use std::io::Read;

use crate::header::OptionalHeaderKey;
use crate::header::Xex2Header;

#[cfg(feature = "serde")]
macro_rules! impl_bitflags_serialize {
	($t:ty) => {
		impl Serialize for $t {
			fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
				s.serialize_u32(self.bits())
			}
		}
	};
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct TlsInfo {
	pub slot_count: u32,
	pub raw_data_address: u32,
	pub data_size: u32,
	pub raw_data_size: u32,
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct ResourceInfo {
	pub resources: Vec<ResourceEntry>,
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct ResourceEntry {
	pub name: String,
	pub address: u32,
	pub size: u32,
}

/// A rating from a specific board.
///
/// Ordered by severity: `Rated(lowest) < Rated(highest) < Unknown(_) < Unrated`.
/// Unrated and Unknown values sort higher than any recognized rating so that
/// filtering "at most rating X" excludes content that can't be verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating<T> {
	Rated(T),
	/// The raw byte didn't match any known variant for this board. Used when
	/// a newer console firmware added rating tiers the current enum doesn't know.
	Unknown(u8),
	/// No rating assigned (0xFF).
	Unrated,
}

impl<T: Ord> PartialOrd for Rating<T> {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}

impl<T: Ord> Ord for Rating<T> {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		use std::cmp::Ordering::*;
		match (self, other) {
			(Rating::Rated(a), Rating::Rated(b)) => a.cmp(b),
			(Rating::Rated(_), _) => Less,
			(_, Rating::Rated(_)) => Greater,
			(Rating::Unknown(a), Rating::Unknown(b)) => a.cmp(b),
			(Rating::Unknown(_), Rating::Unrated) => Less,
			(Rating::Unrated, Rating::Unknown(_)) => Greater,
			(Rating::Unrated, Rating::Unrated) => Equal,
		}
	}
}

impl<T: std::fmt::Display> std::fmt::Display for Rating<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Rating::Rated(v) => v.fmt(f),
			Rating::Unknown(b) => write!(f, "Unknown(0x{:02x})", b),
			Rating::Unrated => f.write_str("Unrated"),
		}
	}
}

#[cfg(feature = "serde")]
impl<T: Serialize> Serialize for Rating<T> {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		match self {
			Rating::Rated(v) => v.serialize(s),
			Rating::Unknown(b) => s.serialize_u8(*b),
			Rating::Unrated => s.serialize_none(),
		}
	}
}

impl<T: TryFrom<u8>> From<u8> for Rating<T> {
	fn from(value: u8) -> Self {
		if value == 0xFF {
			Rating::Unrated
		} else {
			match T::try_from(value) {
				Ok(v) => Rating::Rated(v),
				Err(_) => Rating::Unknown(value),
			}
		}
	}
}

/// Entertainment Software Rating Board (North America).
/// Verified: Portal 2 = E10+ (0x04), Deus Ex HR = M (0x08).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[repr(u8)]
pub enum EsrbRating {
	EC = 0x00,
	E = 0x02,
	E10 = 0x04,
	T = 0x06,
	M = 0x08,
	/// Rating Pending. Dashboard has ESRB_RP.png icon.
	RP = 0x0A,
}

impl std::fmt::Display for EsrbRating {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::EC => f.write_str("eC"),
			Self::E => f.write_str("E"),
			Self::E10 => f.write_str("E10+"),
			Self::T => f.write_str("T"),
			Self::M => f.write_str("M"),
			Self::RP => f.write_str("RP"),
		}
	}
}

/// Pan European Game Information.
/// Verified: Portal 2 = Twelve (0x09), Deus Ex HR = Eighteen (0x0E).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[repr(u8)]
pub enum PegiRating {
	Three = 0x03,
	Seven = 0x05,
	Twelve = 0x09,
	Sixteen = 0x0D,
	Eighteen = 0x0E,
}

impl std::fmt::Display for PegiRating {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Three => f.write_str("3+"),
			Self::Seven => f.write_str("7+"),
			Self::Twelve => f.write_str("12+"),
			Self::Sixteen => f.write_str("16+"),
			Self::Eighteen => f.write_str("18+"),
		}
	}
}

/// Computer Entertainment Rating Organization (Japan).
/// Verified: Portal 2 = A (0x00), Deus Ex HR = unrated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[repr(u8)]
pub enum CeroRating {
	A = 0x00,
	B = 0x02,
	C = 0x04,
	D = 0x06,
	Z = 0x08,
}

impl std::fmt::Display for CeroRating {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::A => f.write_str("A"),
			Self::B => f.write_str("B (12+)"),
			Self::C => f.write_str("C (15+)"),
			Self::D => f.write_str("D (17+)"),
			Self::Z => f.write_str("Z (18+)"),
		}
	}
}

/// Unterhaltungssoftware Selbstkontrolle (Germany).
/// Verified: Portal 2 = Twelve (0x04), Halo 3 = Sixteen (0x06), Deus Ex HR = Eighteen (0x08).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[repr(u8)]
pub enum UskRating {
	Zero = 0x00,
	Six = 0x02,
	Twelve = 0x04,
	Sixteen = 0x06,
	Eighteen = 0x08,
}

impl std::fmt::Display for UskRating {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Zero => f.write_str("0"),
			Self::Six => f.write_str("6+"),
			Self::Twelve => f.write_str("12+"),
			Self::Sixteen => f.write_str("16+"),
			Self::Eighteen => f.write_str("18+"),
		}
	}
}

/// Office of Film and Literature Classification (Australia).
/// Verified: Portal 2 = PG (0x03), Halo 3 = M (0x04), Deus Ex HR = MA15 (0x06).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[repr(u8)]
pub enum OflcAuRating {
	/// Dashboard icon: OFLC_AU_A.png
	A = 0x00,
	/// Dashboard icon: OFLC_AU_G8P.png (G with 8+ parental guidance)
	G8P = 0x02,
	/// Verified: Portal 2 = PG.
	PG = 0x03,
	/// Dashboard icon: OFLC_AU_M15P.png. Verified: Halo 3 = M.
	M15P = 0x04,
	/// Dashboard icon: OFLC_AU_MA15P.png. Verified: Deus Ex HR = MA15+.
	MA15P = 0x06,
}

impl std::fmt::Display for OflcAuRating {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::A => f.write_str("A"),
			Self::G8P => f.write_str("G8+"),
			Self::PG => f.write_str("PG"),
			Self::M15P => f.write_str("M"),
			Self::MA15P => f.write_str("MA15+"),
		}
	}
}

/// British Board of Film Classification.
/// Verified: Portal 2 = Twelve (0x09), Halo 3 = Fifteen (0x0C), Deus Ex HR = Fifteen (0x0C).
/// Values partially overlap with PEGI but are a distinct system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[repr(u8)]
pub enum BbfcRating {
	U = 0x03,
	PG = 0x05,
	Twelve = 0x09,
	TwelveA = 0x0B,
	Fifteen = 0x0C,
	Eighteen = 0x0E,
}

impl std::fmt::Display for BbfcRating {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::U => f.write_str("U"),
			Self::PG => f.write_str("PG"),
			Self::Twelve => f.write_str("12"),
			Self::TwelveA => f.write_str("12A"),
			Self::Fifteen => f.write_str("15"),
			Self::Eighteen => f.write_str("18"),
		}
	}
}

/// Finland VET rating system (used as PEGI-FI in Xbox 360 era).
/// Verified: Portal 2 = K11 (0x08), Halo 3 = K15 (0x0C), Deus Ex HR = K18 (0x0E).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[repr(u8)]
pub enum PegiFiRating {
	S = 0x03,
	K7 = 0x05,
	K11 = 0x08,
	K15 = 0x0C,
	K18 = 0x0E,
}

impl std::fmt::Display for PegiFiRating {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::S => f.write_str("S"),
			Self::K7 => f.write_str("K-7"),
			Self::K11 => f.write_str("K-11"),
			Self::K15 => f.write_str("K-15"),
			Self::K18 => f.write_str("K-18"),
		}
	}
}

/// Office of Film and Literature Classification (New Zealand).
/// Verified: Portal 2 = PG (0x02), Halo 3 = M (0x04), Deus Ex HR = R18 (0x20).
/// The R-restricted tiers use bit 5 (0x20) set; exact tier values for
/// R13/R15/R16 are unknown -- games with those ratings will appear as
/// `Rating::Unknown(byte)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[repr(u8)]
pub enum OflcNzRating {
	G = 0x00,
	PG = 0x02,
	M = 0x04,
	R18 = 0x20,
}

impl std::fmt::Display for OflcNzRating {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::G => f.write_str("G"),
			Self::PG => f.write_str("PG"),
			Self::M => f.write_str("M"),
			Self::R18 => f.write_str("R18"),
		}
	}
}

/// Game content ratings from each regional rating board.
///
/// The full optional header data is 64 bytes, but only the first 12
/// are rating board fields; the rest are reserved (always 0xFF).
///
/// Rating values are board-specific enum indices, NOT literal ages.
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct GameRatings {
	pub esrb: Rating<EsrbRating>,
	pub pegi: Rating<PegiRating>,
	/// Finland VET system, not standard PEGI.
	pub pegi_fi: Rating<PegiFiRating>,
	/// PEGI Portugal (uses standard PEGI values).
	pub pegi_pt: Rating<PegiRating>,
	pub bbfc: Rating<BbfcRating>,
	pub cero: Rating<CeroRating>,
	pub usk: Rating<UskRating>,
	pub oflc_au: Rating<OflcAuRating>,
	pub oflc_nz: Rating<OflcNzRating>,
	/// Korea Media Rating Board. No observed values -- all test games unrated.
	pub kmrb: u8,
	/// Brazil (DJCTQ). Uses 0x40+ range. Observed: 0x40 (Deus Ex HR, real
	/// rating 18+) and 0x50 (unknown game, likely 18+ variant). Exact tier
	/// mapping unverified, left as raw byte.
	pub brazil: u8,
	/// Film and Publication Board (South Africa). Observed: 0x0A (Portal 2),
	/// 0x0D (Halo 3), 0x0E (Deus Ex HR). Exact tier mapping unverified.
	pub fpb: u8,
	pub reserved: [u8; 4],
}

bitflags! {
	/// XEX system flags (optional header 0x00030000).
	///
	/// These are privilege bits 0-31 checked by XeCheckExecutablePrivilege.
	/// The kernel reads them from the signed XEX header to control what
	/// system APIs the executable is allowed to call.
	#[derive(Debug, Clone, Copy, PartialEq, Eq)]
	pub struct SystemFlags: u32 {
		/// Bit 0. Gates NtSetSystemTime and XexTransformImageKey.
		/// Without it, titles cannot set the system clock or perform
		/// cryptographic key transformations.
		const NO_FORCED_REBOOT         = 0x0000_0001;
		/// Bit 1. Gates persistent task scheduling in XS::ScheduleTask.
		const FOREGROUND_TASKS         = 0x0000_0002;
		const NO_ODD_MAPPING           = 0x0000_0004;
		const HANDLES_GAMEPAD_DISCONNECT = 0x0000_0008;
		/// Bit 6. Allows creation of unencrypted network sockets.
		/// Also gates IP grey list configuration. Without it, all
		/// network traffic must use Xbox LIVE encryption.
		const INSECURE_SOCKETS         = 0x0000_0040;
		const XBOX1_INTEROPERABILITY   = 0x0000_0080;
		/// Bit 8. Allows XamSetDashContext -- controls what the
		/// dashboard displays about the current title's activity.
		const DASH_CONTEXT             = 0x0000_0100;
		/// Bit 9. Required for titles using Game Voice Channel.
		const USES_GAME_VOICE_CHANNEL  = 0x0000_0200;
		const PAL50_INCOMPATIBLE       = 0x0000_1000;
		const INSECURE_UTILITY_DRIVE   = 0x0000_2000;
		const XAM_HOOKS                = 0x0000_4000;
		const ACCESS_PIP               = 0x0000_8000;
		const PREFER_BIG_BUTTON_INPUT  = 0x0010_0000;
		const ALLOW_CONTROLLER_SWAPPING = 0x0200_0000;
		/// Bit 26. Enables Kinect (NUI) hardware access. Controls
		/// whether the Kinect health/safety message is shown and
		/// how Kinect initialization behaves.
		const ALLOW_KINECT             = 0x0400_0000;
		/// Bit 27. Fallback Kinect permission checked when
		/// ALLOW_KINECT is not set. Together they control NUI
		/// camera load timeout behavior.
		const ALLOW_KINECT_UNLESS_BOUND = 0x0800_0000;
	}
}
#[cfg(feature = "serde")]
impl_bitflags_serialize!(SystemFlags);

bitflags! {
	/// ImageInfo flags (SecurityInfo + 0x10C).
	///
	/// Verified by the kernel during XEX loading (sub_8007c4f0). The low 16
	/// bits must be zero, and bits 29-31 encode the module type (must be
	/// 0x80000000 for normal title modules).
	#[derive(Debug, Clone, Copy, PartialEq, Eq)]
	pub struct ImageFlags: u32 {
		/// Image uses 4K pages instead of the default 64K. Must be consistent
		/// between SecurityInfo offsets 0x10C and 0x110 or the kernel rejects
		/// the image. System modules use 4K pages because they're small and
		/// numerous -- saves memory vs wasting up to 60K per module with 64K
		/// granularity. Game titles use 64K pages for better TLB efficiency
		/// with large code/data segments.
		const SMALL_PAGES              = 0x1000_0000;
		/// When set, the kernel reads optional header 0x4004 which
		/// contains a (mask, expected) u64 pair and checks it against
		/// the keyvault's restricted_privileges (shared page 0x8E038630).
		/// This is how XEXs gate execution to consoles with specific
		/// keyvault privilege levels. Corresponds to xextool's `-v`.
		const KV_PRIVILEGES_REQUIRED   = 0x0400_0000;
		/// Cleared by kernel if the HV reports the keyvault is unsigned
		/// (shared page 0x8E038614 bit 8 clear). Corresponds to
		/// xextool's `-k`.
		const SIGNED_KEYVAULT_REQUIRED = 0x0800_0000;
	}
}
#[cfg(feature = "serde")]
impl_bitflags_serialize!(ImageFlags);

bitflags! {
	/// Allowed media types bitmask (ImageInfo.allowed_media_types).
	///
	/// Checked by XexpVerifyMediaType against the current boot media.
	/// Setting all bits (0xFFFFFFFF) allows execution from any media.
	#[derive(Debug, Clone, Copy, PartialEq, Eq)]
	pub struct AllowedMediaTypes: u32 {
		const HARD_DISK                = 0x0000_0001;
		/// DVDX2 disc (Xbox 360 game disc format).
		const DVD_X2                   = 0x0000_0002;
		const DVD_CD                   = 0x0000_0004;
		/// Single-layer DVD (4.7 GB).
		const DVD_5                    = 0x0000_0008;
		/// Dual-layer DVD (8.5 GB).
		const DVD_9                    = 0x0000_0010;
		/// Internal NAND flash filesystem.
		const SYSTEM_FLASH             = 0x0000_0020;
		/// Xbox 360 Memory Unit (removable flash storage).
		const MEMORY_UNIT              = 0x0000_0080;
		const USB_MASS_STORAGE         = 0x0000_0100;
		const NETWORK                  = 0x0000_0200;
		/// Loaded directly from memory (not from a filesystem).
		const DIRECT_FROM_MEMORY       = 0x0000_0400;
		const RAM_DRIVE                = 0x0000_1000;
		/// Streamed VOD (SVOD) container format.
		const SVOD                     = 0x0000_2000;
		/// Allows loading from writable storage (HDD, MU, USB). Without
		/// this, XexpVerifyMediaType rejects writable media even if the
		/// individual media bit is set -- the kernel treats writable
		/// storage as untrusted by default since files can be modified
		/// after download. This flag says "trust the content signature
		/// instead of the physical media." Used by Games on Demand,
		/// title updates, and DLC that are LIVE-signed but stored on HDD.
		const INSECURE_PACKAGE         = 0x0000_4000;
		/// Same as INSECURE_PACKAGE but for savegame-adjacent content.
		const SAVEGAME_PACKAGE         = 0x0000_8000;
		/// CON-signed content package (console-signed).
		const LOCALLY_SIGNED_PACKAGE   = 0x0001_0000;
		/// LIVE-signed content package (Microsoft-signed).
		const LIVE_SIGNED_PACKAGE      = 0x0002_0000;
		const XBOX_PLATFORM_PACKAGE    = 0x0004_0000;
	}
}
#[cfg(feature = "serde")]
impl_bitflags_serialize!(AllowedMediaTypes);

bitflags! {
	/// XEX module flags (XEX header offset 0x04).
	///
	/// Identifies the module type. Checked during load to ensure
	/// consistency with the loader's expectations.
	#[derive(Debug, Clone, Copy, PartialEq, Eq)]
	pub struct ModuleFlags: u32 {
		/// User-mode title executable. The kernel enforces an isolation
		/// boundary: title modules can only load other title modules, and
		/// system modules can only load system modules. The TITLE bit in
		/// the XEX header must match the loading context or the load
		/// fails with STATUS_ACCESS_DENIED.
		const TITLE            = 0x0000_0001;
		/// Exports symbols visible to title modules. Without this, the
		/// import resolver rejects cross-boundary imports -- a title
		/// trying to import from a system module without this flag gets
		/// STATUS_ACCESS_DENIED. This is why xboxkrnl.exe and xam.xex
		/// have it set: they export APIs to games.
		const EXPORTS_TO_TITLE = 0x0000_0002;
		/// System debugger module (e.g. xbdm.xex). Creates a separate
		/// trust domain: debugger modules can only be loaded by other
		/// debugger modules. The kernel XORs this bit between the XEX
		/// header and the loading context, rejecting mismatches. This
		/// prevents debug tools from being injected into non-debug
		/// execution contexts.
		const SYSTEM_DEBUGGER  = 0x0000_0004;
		/// Dynamic link library.
		const DLL              = 0x0000_0008;
		/// Module is a patch (XEXP). The kernel rejects direct loads
		/// of modules with this flag -- they must be applied through
		/// the patch system.
		const PATCH            = 0x0000_0010;
		/// Delta patch -- contains binary diffs against a base XEX.
		const PATCH_DELTA      = 0x0000_0020;
		/// Full patch -- complete replacement of the base XEX.
		const PATCH_FULL       = 0x0000_0040;
		/// Module is locked to a specific filesystem path (header key
		/// 0x80FF). The kernel compares the actual load path against
		/// the bound path, resolving symbolic links. If they don't
		/// match, the load fails. Prevents a signed disc game from
		/// being copied to HDD and run from there to bypass disc auth.
		const BOUND_PATH       = 0x4000_0000;
		/// Module is bound to a specific physical device (header key
		/// 0x8105). The kernel reads a 0x14-byte device identifier
		/// and compares it against the boot device.
		const DEVICE_ID        = 0x2000_0000;
	}
}
#[cfg(feature = "serde")]
impl_bitflags_serialize!(ModuleFlags);

/// Date range restriction for time-limited executables (header key 0x4104).
///
/// Timestamps are Windows FILETIME values (100ns intervals since 1601-01-01),
/// stored as (high_u32_LE, low_u32_LE) pairs in the XEX header.
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct DateRange {
	pub not_before: Option<u64>,
	pub not_after: Option<u64>,
}

impl DateRange {
	pub fn not_before_unix(&self) -> Option<i64> {
		self.not_before.and_then(xenon_types::filetime_to_unix_secs)
	}

	pub fn not_after_unix(&self) -> Option<i64> {
		self.not_after.and_then(xenon_types::filetime_to_unix_secs)
	}
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct PageHeapOptions {
	pub size: u32,
	pub flags: u32,
}

impl Xex2Header {
	pub fn tls_info(&self) -> Option<TlsInfo> {
		let data = self.get_optional_data(OptionalHeaderKey::TlsInfo)?;
		if data.len() < 20 {
			return None;
		}
		let mut c = Cursor::new(data);
		let _size = c.read_u32::<BigEndian>().ok()?;
		Some(TlsInfo {
			slot_count: c.read_u32::<BigEndian>().ok()?,
			raw_data_address: c.read_u32::<BigEndian>().ok()?,
			data_size: c.read_u32::<BigEndian>().ok()?,
			raw_data_size: c.read_u32::<BigEndian>().ok()?,
		})
	}

	pub fn resource_info(&self) -> Option<ResourceInfo> {
		let data = self.get_optional_data(OptionalHeaderKey::ResourceInfo)?;
		if data.len() < 4 {
			return None;
		}
		let mut c = Cursor::new(data);
		let size = c.read_u32::<BigEndian>().ok()? as usize;
		let entry_count = (size - 4) / 16;
		let mut resources = Vec::with_capacity(entry_count);
		for _ in 0..entry_count {
			let mut name_buf = [0u8; 8];
			c.read_exact(&mut name_buf).ok()?;
			// Resource names are fixed 8-byte fields; if no null terminator, use full width
			let name_end = name_buf.iter().position(|b| *b == 0).unwrap_or(8);
			let name = String::from_utf8_lossy(&name_buf[..name_end]).into_owned();
			let address = c.read_u32::<BigEndian>().ok()?;
			let size = c.read_u32::<BigEndian>().ok()?;
			resources.push(ResourceEntry { name, address, size });
		}
		Some(ResourceInfo { resources })
	}

	pub fn game_ratings(&self) -> Option<GameRatings> {
		let data = self.get_optional_data(OptionalHeaderKey::GameRatings)?;
		if data.len() < 16 {
			return None;
		}
		Some(GameRatings {
			esrb: data[0].into(),
			pegi: data[1].into(),
			pegi_fi: data[2].into(),
			pegi_pt: data[3].into(),
			bbfc: data[4].into(),
			cero: data[5].into(),
			usk: data[6].into(),
			oflc_au: data[7].into(),
			oflc_nz: data[8].into(),
			kmrb: data[9],
			brazil: data[10],
			fpb: data[11],
			reserved: [data[12], data[13], data[14], data[15]],
		})
	}

	pub fn system_flags(&self) -> Option<SystemFlags> {
		// System flags are stored in the image_info.image_flags field,
		// not as a separate optional header.
		None
	}

	pub fn lan_key(&self) -> Option<[u8; 16]> {
		let data = self.get_optional_data(OptionalHeaderKey::LanKey)?;
		if data.len() < 16 {
			return None;
		}
		let mut key = [0u8; 16];
		key.copy_from_slice(&data[..16]);
		Some(key)
	}

	pub fn xbox_360_logo(&self) -> Option<&[u8]> {
		let data = self.get_optional_data(OptionalHeaderKey::Xbox360Logo)?;
		if data.len() < 4 {
			return None;
		}
		let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
		if data.len() < size {
			return None;
		}
		Some(&data[4..size])
	}

	pub fn bounding_path(&self) -> Option<String> {
		let data = self.get_optional_data(OptionalHeaderKey::BoundingPath)?;
		if data.len() < 4 {
			return None;
		}
		let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
		let str_data = &data[4..std::cmp::min(size, data.len())];
		// Bounding path may fill the entire buffer without null terminator
		let end = str_data.iter().position(|b| *b == 0).unwrap_or(str_data.len());
		Some(String::from_utf8_lossy(&str_data[..end]).into_owned())
	}

	pub fn device_id(&self) -> Option<[u8; 20]> {
		let data = self.get_optional_data(OptionalHeaderKey::DeviceId)?;
		if data.len() < 20 {
			return None;
		}
		let mut id = [0u8; 20];
		id.copy_from_slice(&data[..20]);
		Some(id)
	}

	pub fn default_fs_cache_size(&self) -> Option<u32> {
		let data = self.get_optional_data(OptionalHeaderKey::DefaultFsCacheSize)?;
		if data.len() < 8 {
			return None;
		}
		let mut c = Cursor::new(data);
		let _size = c.read_u32::<BigEndian>().ok()?;
		c.read_u32::<BigEndian>().ok()
	}

	pub fn default_heap_size(&self) -> Option<u32> {
		self.get_optional_inline(OptionalHeaderKey::DefaultHeapSize)
	}

	pub fn page_heap_options(&self) -> Option<PageHeapOptions> {
		let data = self.get_optional_data(OptionalHeaderKey::PageHeapSizeAndFlags)?;
		if data.len() < 8 {
			return None;
		}
		let mut c = Cursor::new(data);
		Some(PageHeapOptions { size: c.read_u32::<BigEndian>().ok()?, flags: c.read_u32::<BigEndian>().ok()? })
	}

	pub fn title_workspace_size(&self) -> Option<u32> {
		self.get_optional_inline(OptionalHeaderKey::TitleWorkspaceSize)
	}

	pub fn additional_title_memory(&self) -> Option<u32> {
		self.get_optional_inline(OptionalHeaderKey::AdditionalTitleMemory)
	}

	pub fn multidisc_media_ids(&self) -> Option<Vec<[u8; 4]>> {
		let data = self.get_optional_data(OptionalHeaderKey::MultidiscMediaIds)?;
		if data.len() < 4 {
			return None;
		}
		let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
		let count = (size - 4) / 4;
		let mut ids = Vec::with_capacity(count);
		for i in 0..count {
			let off = 4 + i * 4;
			if off + 4 > data.len() {
				break;
			}
			ids.push([data[off], data[off + 1], data[off + 2], data[off + 3]]);
		}
		Some(ids)
	}

	pub fn alternate_title_ids(&self) -> Option<Vec<u32>> {
		let data = self.get_optional_data(OptionalHeaderKey::AlternateTitleIds)?;
		if data.len() < 4 {
			return None;
		}
		let size = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
		let count = (size - 4) / 4;
		let mut ids = Vec::with_capacity(count);
		let mut c = Cursor::new(&data[4..]);
		for _ in 0..count {
			if let Ok(id) = c.read_u32::<BigEndian>() {
				ids.push(id);
			}
		}
		Some(ids)
	}
}
