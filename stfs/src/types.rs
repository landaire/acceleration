use bitflags::bitflags;
use num_enum::TryFromPrimitive;
use serde::Serialize;

use crate::error::StfsError;

#[derive(Default, Debug, Serialize, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockNumber(pub usize);

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Sha1Digest(#[serde(with = "crate::serde_bytes::fixed")] pub [u8; 20]);

impl Sha1Digest {
	pub fn as_bytes(&self) -> &[u8; 20] {
		&self.0
	}
}

impl AsRef<[u8]> for Sha1Digest {
	fn as_ref(&self) -> &[u8] {
		&self.0
	}
}

impl From<[u8; 20]> for Sha1Digest {
	fn from(v: [u8; 20]) -> Self {
		Sha1Digest(v)
	}
}

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ConsoleId(#[serde(with = "crate::serde_hex::fixed")] pub [u8; 5]);

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ProfileId(#[serde(with = "crate::serde_hex::fixed")] pub [u8; 8]);

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct DeviceId(#[serde(with = "crate::serde_hex::fixed")] pub [u8; 0x14]);

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

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TitleId(pub u32);

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct MediaId(pub u32);

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct SavegameId(pub u32);

macro_rules! impl_id_hex_display {
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

impl_id_hex_display!(TitleId);
impl_id_hex_display!(MediaId);
impl_id_hex_display!(SavegameId);

impl BlockNumber {
	pub fn as_usize(self) -> usize {
		self.0
	}
}

impl From<u32> for BlockNumber {
	fn from(v: u32) -> Self {
		BlockNumber(v as usize)
	}
}

impl From<usize> for BlockNumber {
	fn from(v: usize) -> Self {
		BlockNumber(v)
	}
}

pub const BLOCK_SIZE: usize = 0x1000;
pub const HASHES_PER_HASH_TABLE: usize = 0xAA;
pub const HASHES_PER_HASH_TABLE_LEVEL: [usize; 3] = [
	HASHES_PER_HASH_TABLE,
	HASHES_PER_HASH_TABLE * HASHES_PER_HASH_TABLE,
	HASHES_PER_HASH_TABLE * HASHES_PER_HASH_TABLE * HASHES_PER_HASH_TABLE,
];
pub const DATA_BLOCKS_PER_HASH_TREE_LEVEL: [usize; 3] =
	[1, HASHES_PER_HASH_TABLE, HASHES_PER_HASH_TABLE * HASHES_PER_HASH_TABLE];

#[derive(Debug, Serialize)]
pub enum PackageType {
	/// User container packages created by an Xbox 360 console,
	/// signed by the user's private key.
	Con,
	/// Xbox LIVE-distributed package signed by Microsoft's private key.
	Live,
	/// Offline-distributed package signed by Microsoft's private key.
	Pirs,
}

impl TryFrom<[u8; 4]> for PackageType {
	type Error = StfsError;

	fn try_from(value: [u8; 4]) -> Result<Self, Self::Error> {
		match &value {
			b"CON " => Ok(PackageType::Con),
			b"LIVE" => Ok(PackageType::Live),
			b"PIRS" => Ok(PackageType::Pirs),
			_ => Err(StfsError::InvalidHeader),
		}
	}
}

#[derive(Debug, Serialize, Copy, Clone)]
pub enum StfsPackageSex {
	Female = 0,
	Male,
}

impl StfsPackageSex {
	/// The "block step" depends on the package's "sex". This determines
	/// which hash tables are used.
	pub const fn block_step(&self) -> [usize; 2] {
		match self {
			StfsPackageSex::Female => [0xAB, 0x718F],
			StfsPackageSex::Male => [0xAC, 0x723A],
		}
	}
}

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u32)]
pub enum ContentType {
	ArcadeGame = 0xD0000,
	AvatarAssetPack = 0x8000,
	AvatarItem = 0x9000,
	CacheFile = 0x40000,
	CommunityGame = 0x2000000,
	GameDemo = 0x80000,
	GameOnDemand = 0x7000,
	GamerPicture = 0x20000,
	GamerTitle = 0xA0000,
	GameTrailer = 0xC0000,
	GameVideo = 0x400000,
	InstalledGame = 0x4000,
	Installer = 0xB0000,
	IPTVPauseBuffer = 0x2000,
	LicenseStore = 0xF0000,
	MarketPlaceContent = 2,
	Movie = 0x100000,
	MusicVideo = 0x300000,
	PodcastVideo = 0x500000,
	Profile = 0x10000,
	Publisher = 3,
	SavedGame = 1,
	StorageDownload = 0x50000,
	Theme = 0x30000,
	Video = 0x200000,
	ViralVideo = 0x600000,
	XboxDownload = 0x70000,
	XboxOriginalGame = 0x5000,
	XboxSavedGame = 0x60000,
	Xbox360Title = 0x1000,
	XNA = 0xE0000,
}

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u32)]
pub enum InstallerType {
	None = 0,
	SystemUpdate = 0x53555044,
	TitleUpdate = 0x54555044,
	SystemUpdateProgressCache = 0x50245355,
	TitleUpdateProgressCache = 0x50245455,
	TitleContentProgressCache = 0x50245443,
}

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u32)]
pub enum FileSystemType {
	STFS = 0,
	SVOD,
	FATX,
}

#[derive(Debug, Serialize, Copy, Clone, PartialEq, Eq, Hash)]
pub enum HashTableLevel {
	First,
	Second,
	Third,
}

#[derive(Debug, Serialize, Clone, Copy, TryFromPrimitive)]
#[repr(u16)]
#[derive(Default)]
pub enum LicenseType {
	#[default]
	Unused = 0x0000,
	Unrestricted = 0xFFFF,
	ConsoleProfileLicense = 0x0009,
	WindowsProfileLicense = 0x0003,
	ConsoleLicense = 0xF000,
	MediaFlags = 0xE000,
	KeyVaultPrivileges = 0xD000,
	HyperVisorFlags = 0xC000,
	UserPrivileges = 0xB000,
}

#[derive(Default, Debug, Serialize, Clone, Copy)]
pub struct LicenseEntry {
	pub ty: LicenseType,
	pub data: u64,
	pub bits: u32,
	pub flags: u32,
}

#[derive(Debug, Serialize)]
pub struct Version {
	pub major: u16,
	pub minor: u16,
	pub build: u16,
	pub revision: u16,
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

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u32)]
pub enum OnlineContentResumeState {
	FileHeadersNotReady = 0x46494C48,
	NewFolder = 0x666F6C64,
	NewFolderResumeAttempt1 = 0x666F6C31,
	NewFolderResumeAttempt2 = 0x666F6C32,
	NewFolderResumeAttemptUnknown = 0x666F6C3F,
	NewFolderResumeAttemptSpecific = 0x666F6C40,
}

#[derive(Debug, Serialize)]
pub enum XContentFlags {
	MetadataIsPEC = 1,
	MetadataSkipRead = 2,
	MetadataDontFreeThumbnails = 4,
}

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u8)]
pub enum ConsoleType {
	DevKit = 1,
	Retail = 2,
}

#[derive(Debug, Serialize)]
pub struct ConsoleTypeFlags(u32);

bitflags! {
	impl ConsoleTypeFlags: u32 {
		const TESTKIT = 0x40000000;
		const RECOVERY_GENERATED = 0x80000000;
	}
}

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u32)]
pub enum AssetSubcategory {
	CarryableCarryable = 0x44c,
	CostumeCasualSuit = 0x68,
	CostumeCostume = 0x69,
	CostumeFormalSuit = 0x67,
	CostumeLongDress = 0x65,
	CostumeShortDress = 100,
	EarringsDanglers = 0x387,
	EarringsLargehoops = 0x38b,
	EarringsSingleDangler = 0x386,
	EarringsSingleLargeHoop = 0x38a,
	EarringsSingleSmallHoop = 0x388,
	EarringsSingleStud = 900,
	EarringsSmallHoops = 0x389,
	EarringsStuds = 0x385,
	GlassesCostume = 0x2be,
	GlassesGlasses = 700,
	GlassesSunglasses = 0x2bd,
	GlovesFingerless = 600,
	GlovesFullFingered = 0x259,
	HatBaseballCap = 0x1f6,
	HatBeanie = 500,
	HatBearskin = 0x1fc,
	HatBrimmed = 0x1f8,
	HatCostume = 0x1fb,
	HatFez = 0x1f9,
	HatFlatCap = 0x1f5,
	HatHeadwrap = 0x1fa,
	HatHelmet = 0x1fd,
	HatPeakCap = 0x1f7,
	RingLast = 0x3ea,
	RingLeft = 0x3e9,
	RingRight = 0x3e8,
	ShirtCoat = 210,
	ShirtHoodie = 0xd0,
	ShirtJacket = 0xd1,
	ShirtLongSleeveShirt = 0xce,
	ShirtLongSleeveTee = 0xcc,
	ShirtPolo = 0xcb,
	ShirtShortSleeveShirt = 0xcd,
	ShirtSportsTee = 200,
	ShirtSweater = 0xcf,
	ShirtTee = 0xc9,
	ShirtVest = 0xca,
	ShoesCostume = 0x197,
	ShoesFormal = 0x193,
	ShoesHeels = 0x191,
	ShoesHighBoots = 0x196,
	ShoesPumps = 0x192,
	ShoesSandals = 400,
	ShoesShortBoots = 0x195,
	ShoesTrainers = 0x194,
	TrousersCargo = 0x131,
	TrousersHotpants = 300,
	TrousersJeans = 0x132,
	TrousersKilt = 0x134,
	TrousersLeggings = 0x12f,
	TrousersLongShorts = 0x12e,
	TrousersLongSkirt = 0x135,
	TrousersShorts = 0x12d,
	TrousersShortSkirt = 0x133,
	TrousersTrousers = 0x130,
	WristwearBands = 0x322,
	WristwearBracelet = 800,
	WristwearSweatbands = 0x323,
	WristwearWatch = 0x321,
}

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u8)]
pub enum SkeletonVersion {
	Nxe = 1,
	Natal,
	NxeAndNatal,
}
