use std::{ffi::CString, io::Read};

use bitflags::bitflags;
use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt};
use chrono::{DateTime, Utc};
use num_enum::TryFromPrimitive;
use std::io::{Cursor, Result as IOResult};
use thiserror::Error;
use serde::Serialize;

fn input_byte_ref<'a>(cursor: &mut Cursor<&'a [u8]>, input: &'a [u8], size: usize) -> &'a [u8] {
    let position: usize = cursor
        .position()
        .try_into()
        .expect("failed to convert position to usize");
    cursor.set_position(
        (position + size)
            .try_into()
            .expect("failed to convert pos into usize"),
    );
    &input[position..position + size]
}

fn read_utf16_cstr<'a>(cursor: &mut Cursor<&'a [u8]>, input: &'a [u8]) -> String {
    let position: usize = cursor
        .position()
        .try_into()
        .expect("failed to convert position to usize");

    let mut end_of_str_position = None;

    for i in (0..input.len()).step_by(2) {
        if input[position + i] == 0 && input[position + i + 1] == 0 {
            // We found the null terminator
            end_of_str_position = Some(position + i);
            break;
        }
    }

    let end_of_str_position = end_of_str_position.expect("failed to find null terminator");

    cursor.set_position(
        (position + end_of_str_position + 2)
            .try_into()
            .expect("failed to convert pos into usize"),
    );
    let byte_range = &input[position..end_of_str_position];

    let mut utf16_str = Vec::with_capacity(byte_range.len() / 2);
    for chunk in byte_range.chunks(2) {
        utf16_str.push(((chunk[0] as u16) << 8) | chunk[1] as u16);
    }

    String::from_utf16(utf16_str.as_slice()).expect("failed to convert data to utf16")
}

#[derive(Error, Debug)]
pub enum StfsError {
    #[error("Invalid STFS package header")]
    InvalidHeader,
    #[error("I/O error")]
    IoError(#[from] std::io::Error),
    #[error("Invalid package type")]
    InvalidPackageType,
}

#[derive(Debug, Serialize)]
pub enum PackageType {
    /// User container packages that are created by an Xbox 360 console and
    /// signed by the user's private key.
    Con,
    /// Xbox LIVE-distributed package that is signed by Microsoft's private key.
    Live,
    /// Offline-distributed package that is signed by Microsoft's private key.
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

#[derive(Debug, Serialize)]
pub enum StfsEntry {
    File(StfsFile),
    Folder(StfsFolder),
}

#[derive(Debug, Serialize)]
pub struct StfsFile {
    name: String,
}

#[derive(Debug, Serialize)]
pub struct StfsFolder {
    name: String,
}

#[derive(Debug, Serialize)]
enum StfsPackageSex {
    Female,
    Male,
}

impl<'a> TryFrom<&XContentHeader<'a>> for StfsPackageSex {
    type Error = StfsError;

    fn try_from(header: &XContentHeader) -> Result<Self, Self::Error> {
        if let FileSystem::STFS(stfs) = &header.volume_descriptor {
            if (!stfs.block_separation) & 1 == 0 {
                Ok(StfsPackageSex::Female)
            } else {
                Ok(StfsPackageSex::Male)
            }
        } else {
            Err(StfsError::InvalidPackageType)
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StfsPackage<'a> {
    header: XContentHeader<'a>,
    sex: StfsPackageSex,
    entries: Vec<StfsEntry>,
}

impl<'a> TryFrom<&'a [u8]> for StfsPackage<'a> {
    type Error = StfsError;

    fn try_from(input: &'a [u8]) -> Result<Self, Self::Error> {
        let mut cursor = Cursor::new(input);
        let xcontent_header = xcontent_header_parser(&mut cursor, input)?;
        // TODO: Don't unwrap
        let package_sex = StfsPackageSex::try_from(&xcontent_header).unwrap();

        Ok(StfsPackage {
            header: xcontent_header,
            sex: package_sex,
            entries: Vec::new(),
        })
    }
}

pub struct HashTable {
    level: HashTableLevel,
}

pub enum HashTableLevel {
    First,
    Second,
    Third,
}

fn certificate_parser<'a>(
    cursor: &mut Cursor<&'a [u8]>,
    input: &'a [u8],
) -> Result<Certificate<'a>, StfsError> {
    let pubkey_cert_size = cursor.read_u16::<BigEndian>()?;
    let mut owner_console_id = [0u8; 5];
    cursor.read_exact(&mut owner_console_id)?;

    let owner_console_part_number = input_byte_ref(cursor, input, 0x11);
    let owner_console_part_number = std::str::from_utf8(
        &owner_console_part_number[..owner_console_part_number
            .iter()
            .position(|b| *b == 0x0)
            .unwrap_or_else(|| owner_console_part_number.len())],
    )
    .expect("console part number is invalid");

    let owner_console_type = cursor.read_u32::<BigEndian>()?;
    let console_type_flags = ConsoleTypeFlags::from_bits(owner_console_type & 0xFFFFFFFC)
        .expect("console type flags are invalid");
    let owner_console_type = ConsoleType::try_from((owner_console_type & 0x3) as u8)
        .expect("owner console type is invalid");

    let date_generation = input_byte_ref(cursor, input, 0x8);
    let date_generation =
        std::str::from_utf8(date_generation).expect("invalid date generation string");

    let public_exponent = cursor.read_u32::<BigEndian>()?;

    let public_modulus = input_byte_ref(cursor, input, 0x80);
    let certificate_signature = input_byte_ref(cursor, input, 0x100);
    let signature = input_byte_ref(cursor, input, 0x80);

    Ok(Certificate {
        pubkey_cert_size,
        owner_console_id,
        owner_console_part_number,
        owner_console_type,
        console_type_flags,
        date_generation,
        public_exponent,
        public_modulus,
        certificate_signature,
        signature,
    })
}

fn xcontent_header_parser<'a>(
    cursor: &mut Cursor<&'a [u8]>,
    input: &'a [u8],
) -> Result<XContentHeader<'a>, StfsError> {
    let mut package_type = [0u8; 4];
    cursor.read_exact(&mut package_type)?;
    let package_type = PackageType::try_from(package_type)?;

    let certificate = if let package_type = PackageType::Con {
        Some(certificate_parser(cursor, input)?)
    } else {
        None
    };

    let (input, package_signature) =
        if matches!(package_type, PackageType::Live | PackageType::Pirs) {
            let sig = input_byte_ref(cursor, input, 0x100);
            (input, Some(sig))
        } else {
            (input, None)
        };

    cursor.set_position(0x22c);

    let mut license_data = [LicenseEntry::default(); 16];
    for i in 0..license_data.len() {
        let license = cursor.read_u64::<BigEndian>()?;
        license_data[i].ty = LicenseType::try_from(
            u16::try_from(license >> 48).expect("failed to convert license type to u16"),
        )
        .expect("invalid LicenseType");
        license_data[i].data = license & 0xFFFFFFFFFFFF;
        license_data[i].bits = cursor.read_u32::<BigEndian>()?;
        license_data[i].flags = cursor.read_u32::<BigEndian>()?;
    }

    let header_hash = input_byte_ref(cursor, input, 0x14);
    let header_size = cursor.read_u32::<BigEndian>()?;

    let content_type =
        ContentType::try_from(cursor.read_u32::<BigEndian>()?).expect("invalid content type");
    let metadata_version = cursor.read_u32::<BigEndian>()?;
    let content_size = cursor.read_u64::<BigEndian>()?;
    let media_id = cursor.read_u32::<BigEndian>()?;
    let version = cursor.read_u32::<BigEndian>()?;
    let base_version = cursor.read_u32::<BigEndian>()?;
    let title_id = cursor.read_u32::<BigEndian>()?;
    let platform = cursor.read_u8()?;
    let executable_type = cursor.read_u8()?;
    let disc_number = cursor.read_u8()?;
    let disc_in_set = cursor.read_u8()?;
    let savegame_id = cursor.read_u32::<BigEndian>()?;

    let mut console_id = [0u8; 5];
    cursor.read_exact(&mut console_id)?;

    let mut profile_id = [0u8; 8];
    cursor.read_exact(&mut profile_id)?;

    // read the file system type
    cursor.set_position(0x3a9);
    let filesystem_type =
        FileSystemType::try_from(cursor.read_u32::<BigEndian>()?).expect("invalid filesystem type");

    let volume_descriptor = match filesystem_type {
        FileSystemType::STFS => {
            cursor.set_position(0x379);
            FileSystem::STFS(StfsVolumeDescriptor::parse(cursor, input)?)
        }
        FileSystemType::SVOD => FileSystem::SVOD(SvodVolumeDescriptor::parse(cursor, input)?),
        _ => panic!("Invalid filesystem type"),
    };

    let data_file_count = cursor.read_u32::<BigEndian>()?;
    let data_file_combined_size = cursor.read_u64::<BigEndian>()?;

    let content_metadata = match content_type {
        ContentType::AvatarItem => {
            cursor.set_position(0x3d9);
            Some(ContentMetadata::AvatarItem(AvatarAssetInformation::parse(
                cursor, input,
            )?))
        }
        ContentType::Video => {
            cursor.set_position(0x3d9);
            Some(ContentMetadata::Video(MediaInformation::parse(
                cursor, input,
            )?))
        }
        _ => None,
    };

    cursor.set_position(0x3fd);

    let device_id = input_byte_ref(cursor, input, 0x14);

    let display_name = read_utf16_cstr(cursor, input);

    cursor.set_position(0x311);
    let display_description = read_utf16_cstr(cursor, input);

    cursor.set_position(0x1611);
    let publisher_name = read_utf16_cstr(cursor, input);

    cursor.set_position(0x1691);
    let title_name = read_utf16_cstr(cursor, input);

    cursor.set_position(0x1711);
    let transfer_flags = cursor.read_u8()?;

    let thumbnail_image_size = cursor.read_u32::<BigEndian>()? as usize;
    let title_thumbnail_image_size = cursor.read_u32::<BigEndian>()? as usize;

    let thumbnail_image = input_byte_ref(cursor, input, thumbnail_image_size);
    cursor.set_position(0x571a);

    let title_image = input_byte_ref(cursor, input, title_thumbnail_image_size);
    cursor.set_position(0x971a);

    let mut installer_type = None;
    let mut installer_meta = None;
    if ((header_size + 0xFFF) & 0xFFFFF000) - 0x971A > 0x15F4 {
        installer_type = Some(InstallerType::try_from(cursor.read_u32::<BigEndian>()?).expect("invalid InstallerType"));
        installer_meta = match *installer_type.as_ref().unwrap() {
            InstallerType::SystemUpdate | InstallerType::TitleUpdate => {
                let installer_base_version = Version::from(cursor.read_u32::<BigEndian>()?);
                let installer_version= Version::from(cursor.read_u32::<BigEndian>()?);
                Some(InstallerMeta::FullInstaller(FullInstallerMeta {
                    installer_base_version,
                    installer_version,
                }))
            }
            InstallerType::SystemUpdateProgressCache | InstallerType::TitleUpdateProgressCache | InstallerType::TitleContentProgressCache => {
                let resume_state = OnlineContentResumeState::try_from(cursor.read_u32::<BigEndian>()?).expect("invalid resume state");
                let current_file_index = cursor.read_u32::<BigEndian>()?;
                let current_file_offset = cursor.read_u64::<BigEndian>()?;
                let bytes_processed = cursor.read_u64::<BigEndian>()?;

                let high_date_time = cursor.read_u32::<BigEndian>()?;
                let low_date_time = cursor.read_u32::<BigEndian>()?;

                // TODO: Fix
                let last_modified = Utc::now();

                Some(InstallerMeta::InstallerProgressCache(InstallerProgressCache {
                    resume_state,
                    current_file_index,
                    current_file_offset,
                    bytes_processed,
                    last_modified,
                    cab_resume_data: todo!("need to implement CAB resume data"),
                }));
            }
            _ => {
                // anything else is ok
                None
            }
        }
    }

    let enabled = false;
    Ok(XContentHeader {
        package_type,
        certificate,
        package_signature: package_signature,
        license_data,
        header_hash,
        header_size,
        content_type,
        metadata_version,
        content_size,
        media_id,
        version,
        base_version,
        title_id,
        platform,
        executable_type,
        disc_number,
        disc_in_set,
        savegame_id,
        console_id,
        profile_id,
        volume_descriptor,
        filesystem_type,
        enabled,
        data_file_count,
        data_file_combined_size,
        device_id,
        display_name,
        display_description,
        publisher_name,
        title_name,
        transfer_flags,
        thumbnail_image_size,
        thumbnail_image,
        title_thumbnail_image_size,
        title_image,
        installer_type,
        installer_meta,
        content_metadata,
    })
}

#[derive(Debug, Serialize)]
pub struct XContentHeader<'a> {
    package_type: PackageType,
    /// Only present in console-signed packages
    certificate: Option<Certificate<'a>>,
    /// Only present in strong-signed packages
    package_signature: Option<&'a [u8]>,

    license_data: [LicenseEntry; 0x10],
    header_hash: &'a [u8],
    header_size: u32,
    content_type: ContentType,
    metadata_version: u32,
    content_size: u64,
    media_id: u32,
    version: u32,
    base_version: u32,
    title_id: u32,
    platform: u8,
    executable_type: u8,
    disc_number: u8,
    disc_in_set: u8,
    savegame_id: u32,
    console_id: [u8; 5],
    profile_id: [u8; 8],
    volume_descriptor: FileSystem<'a>,
    filesystem_type: FileSystemType,
    /// Only in PEC -- not sure what this represents. This always needs to be set to 1
    enabled: bool,

    // Start metadata v1
    data_file_count: u32,
    data_file_combined_size: u64,
    device_id: &'a [u8],
    display_name: String,
    display_description: String,
    publisher_name: String,
    title_name: String,
    transfer_flags: u8,
    thumbnail_image_size: usize,
    thumbnail_image: &'a [u8],
    title_thumbnail_image_size: usize,
    title_image: &'a [u8],
    installer_type: Option<InstallerType>,
    installer_meta: Option<InstallerMeta<'a>>,
    content_metadata: Option<ContentMetadata<'a>>,
}

#[derive(Debug, Serialize)]
struct AvatarAssetInformation<'a> {
    subcategory: AssetSubcategory,
    colorizable: u32,
    guid: &'a [u8],
    skeleton_version: SkeletonVersion,
}

impl<'a> AvatarAssetInformation<'a> {
    fn parse(
        cursor: &mut Cursor<&'a [u8]>,
        input: &'a [u8],
    ) -> Result<AvatarAssetInformation<'a>, StfsError> {
        // This data is little endian for some reason
        let subcategory = AssetSubcategory::try_from(cursor.read_u32::<LittleEndian>()?)
            .expect("invalid avatar asset subcategory");
        let colorizable = cursor.read_u32::<LittleEndian>()?;
        let guid = input_byte_ref(cursor, input, 0x10);
        let skeleton_version =
            SkeletonVersion::try_from(cursor.read_u8()?).expect("invalid skeleton version");

        Ok(AvatarAssetInformation {
            subcategory,
            colorizable,
            guid,
            skeleton_version,
        })
    }
}

#[derive(Debug, Serialize)]
struct MediaInformation<'a> {
    series_id: &'a [u8],
    season_id: &'a [u8],
    season_number: u16,
    episode_number: u16,
}

impl<'a> MediaInformation<'a> {
    fn parse(
        cursor: &mut Cursor<&'a [u8]>,
        input: &'a [u8],
    ) -> Result<MediaInformation<'a>, StfsError> {
        let series_id = input_byte_ref(cursor, input, 0x10);
        let season_id = input_byte_ref(cursor, input, 0x10);
        let season_number = cursor.read_u16::<BigEndian>()?;
        let episode_number = cursor.read_u16::<BigEndian>()?;

        Ok(MediaInformation {
            series_id,
            season_id,
            season_number,
            episode_number,
        })
    }
}

#[derive(Debug, Serialize)]
struct InstallerProgressCache<'a> {
    resume_state: OnlineContentResumeState,
    current_file_index: u32,
    current_file_offset: u64,
    bytes_processed: u64,
    last_modified: DateTime<Utc>,
    cab_resume_data: &'a [u8],
}

#[derive(Debug, Serialize)]
struct FullInstallerMeta {
    installer_base_version: Version,
    installer_version: Version,
}

#[derive(Debug, Serialize)]
enum InstallerMeta<'a> {
    FullInstaller(FullInstallerMeta),
    InstallerProgressCache(InstallerProgressCache<'a>),
}

#[derive(Debug, Serialize)]
struct Certificate<'a> {
    pubkey_cert_size: u16,
    owner_console_id: [u8; 5],
    owner_console_part_number: &'a str,
    owner_console_type: ConsoleType,
    console_type_flags: ConsoleTypeFlags,
    date_generation: &'a str,
    public_exponent: u32,
    public_modulus: &'a [u8],
    certificate_signature: &'a [u8],
    signature: &'a [u8],
}

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u8)]
enum ConsoleType {
    DevKit = 1,
    Retail = 2,
}

bitflags! {
    #[derive(Serialize)]
    struct ConsoleTypeFlags: u32 {
        const TESTKIT = 0x40000000;
        const RECOVERY_GENERATED = 0x80000000;
    }
}

#[derive(Debug, Serialize, Clone, Copy, TryFromPrimitive)]
#[repr(u16)]
enum LicenseType {
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

impl Default for LicenseType {
    fn default() -> Self {
        Self::Unused
    }
}

#[derive(Default, Debug, Serialize, Clone, Copy)]
struct LicenseEntry {
    ty: LicenseType,
    data: u64,
    bits: u32,
    flags: u32,
}

#[derive(Debug, Serialize)]
enum ContentMetadata<'a> {
    AvatarItem(AvatarAssetInformation<'a>),
    Video(MediaInformation<'a>),
}

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u32)]
enum ContentType {
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
enum InstallerType {
    None = 0,
    SystemUpdate = 0x53555044,
    TitleUpdate = 0x54555044,
    SystemUpdateProgressCache = 0x50245355,
    TitleUpdateProgressCache = 0x50245455,
    TitleContentProgressCache = 0x50245443,
}

#[derive(Debug, Serialize)]
struct Version {
    major: u16,
    minor: u16,
    build: u16,
    revision: u16,
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
enum OnlineContentResumeState {
    FileHeadersNotReady = 0x46494C48,
    NewFolder = 0x666F6C64,
    NewFolderResumeAttempt1 = 0x666F6C31,
    NewFolderResumeAttempt2 = 0x666F6C32,
    NewFolderResumeAttemptUnknown = 0x666F6C3F,
    NewFolderResumeAttemptSpecific = 0x666F6C40,
}
#[derive(Debug, Serialize)]
enum XContentFlags {
    MetadataIsPEC = 1,
    MetadataSkipRead = 2,
    MetadataDontFreeThumbnails = 4,
}

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u32)]
enum FileSystemType {
    STFS = 0,
    SVOD,
    FATX,
}

#[derive(Debug, Serialize)]
enum FileSystem<'a> {
    STFS(StfsVolumeDescriptor<'a>),
    SVOD(SvodVolumeDescriptor<'a>),
}

#[derive(Debug, Serialize)]
struct StfsVolumeDescriptor<'a> {
    size: u8,
    reserved: u8,
    block_separation: u8,
    file_table_block_count: u16,
    /// This is encoded as a 24-bit integer
    file_table_block_num: u32,
    top_hash_table_hash: &'a [u8],
    allocated_block_count: u32,
    unallocated_block_count: u32,
}

impl<'a> StfsVolumeDescriptor<'a> {
    fn parse(
        cursor: &mut Cursor<&'a [u8]>,
        input: &'a [u8],
    ) -> Result<StfsVolumeDescriptor<'a>, StfsError> {
        Ok(StfsVolumeDescriptor {
            size: cursor.read_u8()?,
            reserved: cursor.read_u8()?,
            block_separation: cursor.read_u8()?,
            file_table_block_count: cursor.read_u16::<BigEndian>()?,
            file_table_block_num: cursor.read_u32::<BigEndian>()?,
            top_hash_table_hash: input_byte_ref(cursor, input, 0x14),
            allocated_block_count: cursor.read_u32::<BigEndian>()?,
            unallocated_block_count: cursor.read_u32::<BigEndian>()?,
        })
    }
}

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u32)]
enum AssetSubcategory {
    CarryableCarryable = 0x44c,
    // CarryableFirst = 0x44c,
    // CarryableLast = 0x44c,
    CostumeCasualSuit = 0x68,
    CostumeCostume = 0x69,
    // CostumeFirst = 100,
    CostumeFormalSuit = 0x67,
    // CostumeLast = 0x6a,
    CostumeLongDress = 0x65,
    CostumeShortDress = 100,
    EarringsDanglers = 0x387,
    // EarringsFirst = 900,
    EarringsLargehoops = 0x38b,
    // EarringsLast = 0x38b,
    EarringsSingleDangler = 0x386,
    EarringsSingleLargeHoop = 0x38a,
    EarringsSingleSmallHoop = 0x388,
    EarringsSingleStud = 900,
    EarringsSmallHoops = 0x389,
    EarringsStuds = 0x385,
    GlassesCostume = 0x2be,
    // GlassesFirst = 700,
    GlassesGlasses = 700,
    // GlassesLast = 0x2be,
    GlassesSunglasses = 0x2bd,
    GlovesFingerless = 600,
    // GlovesFirst = 600,
    GlovesFullFingered = 0x259,
    // GlovesLast = 0x259,
    HatBaseballCap = 0x1f6,
    HatBeanie = 500,
    HatBearskin = 0x1fc,
    HatBrimmed = 0x1f8,
    HatCostume = 0x1fb,
    HatFez = 0x1f9,
    // HatFirst = 500,
    HatFlatCap = 0x1f5,
    HatHeadwrap = 0x1fa,
    HatHelmet = 0x1fd,
    // HatLast = 0x1fd,
    HatPeakCap = 0x1f7,
    // RingFirst = 0x3e8,
    RingLast = 0x3ea,
    RingLeft = 0x3e9,
    RingRight = 0x3e8,
    ShirtCoat = 210,
    // ShirtFirst = 200,
    ShirtHoodie = 0xd0,
    ShirtJacket = 0xd1,
    // ShirtLast = 210,
    ShirtLongSleeveShirt = 0xce,
    ShirtLongSleeveTee = 0xcc,
    ShirtPolo = 0xcb,
    ShirtShortSleeveShirt = 0xcd,
    ShirtSportsTee = 200,
    ShirtSweater = 0xcf,
    ShirtTee = 0xc9,
    ShirtVest = 0xca,
    ShoesCostume = 0x197,
    // ShoesFirst = 400,
    ShoesFormal = 0x193,
    ShoesHeels = 0x191,
    ShoesHighBoots = 0x196,
    // ShoesLast = 0x197,
    ShoesPumps = 0x192,
    ShoesSandals = 400,
    ShoesShortBoots = 0x195,
    ShoesTrainers = 0x194,
    TrousersCargo = 0x131,
    // TrousersFirst = 300,
    TrousersHotpants = 300,
    TrousersJeans = 0x132,
    TrousersKilt = 0x134,
    // TrousersLast = 0x135,
    TrousersLeggings = 0x12f,
    TrousersLongShorts = 0x12e,
    TrousersLongSkirt = 0x135,
    TrousersShorts = 0x12d,
    TrousersShortSkirt = 0x133,
    TrousersTrousers = 0x130,
    WristwearBands = 0x322,
    WristwearBracelet = 800,
    // WristwearFirst = 800,
    // WristwearLast = 0x323,
    WristwearSweatbands = 0x323,
    WristwearWatch = 0x321,
}

#[derive(Debug, Serialize)]
enum BinaryAssetType {
    Component = 1,
    Texture = 2,
    ShapeOverride = 3,
    Animation = 4,
    ShapeOverridePost = 5,
}

#[derive(Debug, Serialize, TryFromPrimitive)]
#[repr(u8)]
enum SkeletonVersion {
    Nxe = 1,
    Natal,
    NxeAndNatal,
}

#[derive(Debug, Serialize)]
enum AssetGender {
    Male = 1,
    Female,
    Both,
}

#[derive(Debug, Serialize)]
struct SvodVolumeDescriptor<'a> {
    size: u8,
    block_cache_element_count: u8,
    worker_thread_processor: u8,
    worker_thread_priority: u8,
    root_hash: &'a [u8],
    flags: u8,
    /// Encoded as an int24
    data_block_count: u32,
    /// Encoded as an int24
    data_block_offset: u32,
    reserved: [u8; 5],
}

impl<'a> SvodVolumeDescriptor<'a> {
    fn parse(
        cursor: &mut Cursor<&'a [u8]>,
        input: &'a [u8],
    ) -> Result<SvodVolumeDescriptor<'a>, StfsError> {
        let size = cursor.read_u8()?;
        let block_cache_element_count = cursor.read_u8()?;
        let worker_thread_processor = cursor.read_u8()?;
        let worker_thread_priority = cursor.read_u8()?;
        let root_hash = input_byte_ref(cursor, input, 0x14);
        let flags = cursor.read_u8()?;
        let data_block_count = cursor.read_u24::<BigEndian>()?;
        let data_block_offset = cursor.read_u24::<BigEndian>()?;
        let mut reserved = [0u8; 5];
        cursor.read_exact(&mut reserved)?;

        Ok(SvodVolumeDescriptor {
            size,
            block_cache_element_count,
            worker_thread_processor,
            worker_thread_priority,
            root_hash,
            flags,
            data_block_count,
            data_block_offset,
            reserved,
        })
    }
}
