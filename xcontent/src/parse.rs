use byteorder::BigEndian;
use byteorder::LittleEndian;
use byteorder::ReadBytesExt;
use sha1::Digest;
use sha1::Sha1;
use std::io::Cursor;
use std::io::Read;
use std::sync::Arc;
use stfs::StfsPackage;
use vfs::VfsPath;

use serde::Serialize;

use xecrypt::XContentKeyMaterial;
use xecrypt::XContentSignatureType;

use crate::error::XContentError;

const LICENSE_ENTRY_COUNT: usize = 0x10;
const CONTENT_ID_SIZE: usize = 0x14;
const SHA1_DIGEST_SIZE: usize = 0x14;
const DEVICE_ID_SIZE: usize = 0x14;
const DISPLAY_STRING_COUNT: usize = 12;
const WIDE_STRING_SIZE: usize = 128;
const MAX_THUMBNAIL_SIZE: usize = 0x4000;
const PAGE_ALIGNMENT: usize = 0x1000;
const PAGE_ALIGNMENT_MASK: usize = PAGE_ALIGNMENT - 1;

mod metadata_offsets {
	pub const VOLUME_KIND: u64 = 0x3A9;
	pub const VOLUME_DESCRIPTOR: u64 = 0x379;
	pub const DATA_FILE_COUNT: u64 = 0x3AD;
	pub const DEVICE_ID: u64 = 0x3FD;
	pub const DISPLAY_DESCRIPTION: u64 = 0xD11;
	pub const PUBLISHER_NAME: u64 = 0x1611;
	pub const TITLE_NAME: u64 = 0x1691;
	pub const TRANSFER_FLAGS: u64 = 0x1711;
	pub const INSTALLER_TYPE_THRESHOLD: usize = 0x971A;
	pub const INSTALLER_METADATA_SIZE: usize = 0x15F4;
}

fn read_wide_string_fixed(cursor: &mut Cursor<&[u8]>, byte_len: usize) -> std::io::Result<String> {
	let start = cursor.position();
	let mut chars = Vec::new();
	for _ in 0..byte_len / 2 {
		let c = cursor.read_u16::<BigEndian>()?;
		if c == 0 {
			break;
		}
		chars.push(c);
	}
	cursor.set_position(start + byte_len as u64);
	Ok(String::from_utf16_lossy(&chars))
}

fn read_null_wide_string(cursor: &mut Cursor<&[u8]>, input: &[u8]) -> String {
	let position = cursor.position() as usize;
	let mut end = None;
	for i in (0..input.len() - position).step_by(2) {
		if input[position + i] == 0 && input[position + i + 1] == 0 {
			end = Some(position + i);
			break;
		}
	}
	let end = end.unwrap_or(input.len());
	cursor.set_position((end + 2) as u64);

	let byte_range = &input[position..end];
	let mut utf16 = Vec::with_capacity(byte_range.len() / 2);
	for chunk in byte_range.chunks(2) {
		utf16.push(((chunk[0] as u16) << 8) | chunk[1] as u16);
	}
	String::from_utf16_lossy(&utf16)
}

#[derive(Debug, Serialize)]
pub struct FixedLengthNullWideString(String);

impl std::ops::Deref for FixedLengthNullWideString {
	type Target = str;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl FixedLengthNullWideString {
	fn parse(cursor: &mut Cursor<&[u8]>) -> std::io::Result<Self> {
		Ok(Self(read_wide_string_fixed(cursor, WIDE_STRING_SIZE)?))
	}
}

#[derive(Debug, Serialize)]
pub struct XContentHeader {
	pub signature_type: XContentSignatureType,
	pub key_material: XContentKeyMaterial,
	pub license_data_offset: usize,
	pub license_data: [LicenseEntry; LICENSE_ENTRY_COUNT],
	pub content_id: [u8; CONTENT_ID_SIZE],
	pub header_size: u32,
	pub end_of_header_offset: usize,
	pub metadata: XContentMetadata,
}

impl XContentHeader {
	pub fn parse(input: impl AsRef<[u8]>) -> Result<Self, XContentError> {
		Self::parse_inner(input.as_ref())
	}

	fn parse_inner(input: &[u8]) -> Result<Self, XContentError> {
		let mut cursor = Cursor::new(input);

		let mut magic = [0u8; 4];
		cursor.read_exact(&mut magic)?;
		let signature_type = XContentSignatureType::parse(&magic).ok_or(XContentError::InvalidMagic)?;

		let key_material = XContentKeyMaterial::parse(&mut cursor, signature_type)?;

		let license_data_offset = cursor.position() as usize;
		let mut license_data = [LicenseEntry::default(); LICENSE_ENTRY_COUNT];
		for entry in &mut license_data {
			let ty_raw = cursor.read_u16::<BigEndian>()?;
			let mut data = [0u8; 6];
			cursor.read_exact(&mut data)?;
			let bits = cursor.read_u32::<BigEndian>()?;
			let flags = cursor.read_u32::<BigEndian>()?;
			*entry = LicenseEntry { ty_raw, data, bits, flags };
		}

		let mut content_id = [0u8; CONTENT_ID_SIZE];
		cursor.read_exact(&mut content_id)?;
		let header_size = cursor.read_u32::<BigEndian>()?;
		let end_of_header_offset = cursor.position() as usize;

		let metadata = XContentMetadata::parse(&mut cursor, input, header_size)?;

		Ok(XContentHeader {
			signature_type,
			key_material,
			license_data_offset,
			license_data,
			content_id,
			header_size,
			end_of_header_offset,
			metadata,
		})
	}

	pub fn data_start_offset(&self) -> usize {
		((self.header_size as usize) + PAGE_ALIGNMENT_MASK) & !PAGE_ALIGNMENT_MASK
	}

	pub fn header_hash(&self, data: &[u8]) -> [u8; SHA1_DIGEST_SIZE] {
		let mut hasher = Sha1::new();
		hasher.update(&data[self.license_data_offset..self.end_of_header_offset]);
		hasher.finalize().into()
	}
}

#[derive(Debug, Serialize)]
pub struct XContentMetadata {
	pub content_type: ContentType,
	pub metadata_version: u32,
	pub content_size: u64,
	pub media_id: xenon_types::MediaId,
	pub version: xenon_types::Version,
	pub base_version: xenon_types::Version,
	pub title_id: xenon_types::TitleId,
	pub platform: u8,
	pub executable_type: u8,
	pub disc_number: u8,
	pub disc_in_set: u8,
	pub savegame_id: xenon_types::SavegameId,
	pub console_id: xenon_types::ConsoleId,
	pub creator_xuid: u64,
	pub volume_kind: FileSystemKind,
	pub volume_descriptor: FileSystem,
	pub data_file_count: u32,
	pub data_file_combined_size: u64,
	pub device_id: xenon_types::DeviceId,
	pub display_name: [FixedLengthNullWideString; 12],
	pub display_description: [FixedLengthNullWideString; 12],
	pub publisher_name: String,
	pub title_name: String,
	pub transfer_flags: u8,
	pub thumbnail_image_size: u32,
	pub title_thumbnail_image_size: u32,
	pub thumbnail_image: Vec<u8>,
	pub title_image: Vec<u8>,
	pub installer_type: Option<InstallerType>,
}

impl XContentMetadata {
	fn parse(cursor: &mut Cursor<&[u8]>, input: &[u8], header_size: u32) -> Result<Self, XContentError> {
		let content_type =
			ContentType::try_from(cursor.read_u32::<BigEndian>()?).map_err(|_| XContentError::InvalidHeader)?;
		let metadata_version = cursor.read_u32::<BigEndian>()?;
		let content_size = cursor.read_u64::<BigEndian>()?;
		let media_id = xenon_types::MediaId(cursor.read_u32::<BigEndian>()?);
		let version = xenon_types::Version::from(cursor.read_u32::<BigEndian>()?);
		let base_version = xenon_types::Version::from(cursor.read_u32::<BigEndian>()?);
		let title_id = xenon_types::TitleId(cursor.read_u32::<BigEndian>()?);
		let platform = cursor.read_u8()?;
		let executable_type = cursor.read_u8()?;
		let disc_number = cursor.read_u8()?;
		let disc_in_set = cursor.read_u8()?;
		let savegame_id = xenon_types::SavegameId(cursor.read_u32::<BigEndian>()?);
		let console_id = xenon_types::ConsoleId({
			let mut buf = [0u8; 5];
			cursor.read_exact(&mut buf)?;
			buf
		});
		let creator_xuid = cursor.read_u64::<BigEndian>()?;

		cursor.set_position(metadata_offsets::VOLUME_KIND);
		let volume_kind =
			FileSystemKind::try_from(cursor.read_u32::<BigEndian>()?).map_err(|_| XContentError::InvalidHeader)?;

		cursor.set_position(metadata_offsets::VOLUME_DESCRIPTOR);
		let volume_descriptor = match volume_kind {
			FileSystemKind::Stfs => FileSystem::Stfs(StfsVolumeDescriptor::parse(cursor)?),
			FileSystemKind::Svod => FileSystem::Svod(SvodVolumeDescriptor::parse(cursor)?),
			FileSystemKind::Fatx => FileSystem::Fatx,
		};

		// After volume descriptor parsing, continue from the right position
		cursor.set_position(metadata_offsets::DATA_FILE_COUNT);
		let data_file_count = cursor.read_u32::<BigEndian>()?;
		let data_file_combined_size = cursor.read_u64::<BigEndian>()?;

		cursor.set_position(metadata_offsets::DEVICE_ID);
		let device_id = xenon_types::DeviceId({
			let mut buf = [0u8; DEVICE_ID_SIZE];
			cursor.read_exact(&mut buf)?;
			buf
		});

		let mut display_name: [FixedLengthNullWideString; DISPLAY_STRING_COUNT] =
			std::array::from_fn(|_| FixedLengthNullWideString(String::new()));
		for name in &mut display_name {
			*name = FixedLengthNullWideString::parse(cursor)?;
		}

		cursor.set_position(metadata_offsets::DISPLAY_DESCRIPTION);
		let mut display_description: [FixedLengthNullWideString; DISPLAY_STRING_COUNT] =
			std::array::from_fn(|_| FixedLengthNullWideString(String::new()));
		for desc in &mut display_description {
			*desc = FixedLengthNullWideString::parse(cursor)?;
		}

		cursor.set_position(metadata_offsets::PUBLISHER_NAME);
		let publisher_name = read_null_wide_string(cursor, input);

		cursor.set_position(metadata_offsets::TITLE_NAME);
		let title_name = read_null_wide_string(cursor, input);

		cursor.set_position(metadata_offsets::TRANSFER_FLAGS);
		let transfer_flags = cursor.read_u8()?;
		let thumbnail_image_size = cursor.read_u32::<BigEndian>()?;
		let title_thumbnail_image_size = cursor.read_u32::<BigEndian>()?;

		let mut thumbnail_image = vec![0u8; thumbnail_image_size as usize];
		cursor.read_exact(&mut thumbnail_image)?;
		cursor.set_position(cursor.position() + (MAX_THUMBNAIL_SIZE - thumbnail_image_size as usize) as u64);

		let mut title_image = vec![0u8; title_thumbnail_image_size as usize];
		cursor.read_exact(&mut title_image)?;
		cursor.set_position(cursor.position() + (MAX_THUMBNAIL_SIZE - title_thumbnail_image_size as usize) as u64);

		let aligned_header = ((header_size as usize) + PAGE_ALIGNMENT_MASK) & !PAGE_ALIGNMENT_MASK;
		let installer_type = if aligned_header - metadata_offsets::INSTALLER_TYPE_THRESHOLD
			> metadata_offsets::INSTALLER_METADATA_SIZE
		{
			Some(InstallerType::try_from(cursor.read_u32::<BigEndian>()?).map_err(|_| XContentError::InvalidHeader)?)
		} else {
			None
		};

		Ok(XContentMetadata {
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
			creator_xuid,
			volume_kind,
			volume_descriptor,
			data_file_count,
			data_file_combined_size,
			device_id,
			display_name,
			display_description,
			publisher_name,
			title_name,
			transfer_flags,
			thumbnail_image_size,
			title_thumbnail_image_size,
			thumbnail_image,
			title_image,
			installer_type,
		})
	}
}

#[derive(Debug, Serialize)]
pub enum XboxFilesystem {
	Stfs(StfsPackage),
}

#[derive(Debug, Serialize)]
pub struct XContentPackage {
	pub header: XContentHeader,
	pub inner_file_system: XboxFilesystem,
}

impl XContentPackage {
	pub fn to_vfs_path<T>(&self, _data: Arc<T>) -> VfsPath
	where
		T: AsRef<[u8]> + Send + Sync + 'static,
	{
		todo!("port to new stfs VFS API")
	}

	pub fn to_vfs<T>(&self, _data: Arc<T>) -> Box<dyn vfs::FileSystem>
	where
		T: AsRef<[u8]> + Send + Sync + 'static,
	{
		todo!("port to new stfs VFS API")
	}

	pub fn verify_signature(&self, data: &[u8]) -> Result<xecrypt::ConsoleKind, xecrypt::Error> {
		xecrypt::verify_xcontent_signature(
			self.header.signature_type,
			&self.header.key_material,
			&self.header.header_hash(data),
		)
	}

	pub fn storage_path(&self) -> String {
		format!(
			"Content/{:016X}/{:08X}/{:08X}/{}",
			self.header.metadata.creator_xuid,
			self.header.metadata.title_id,
			self.header.metadata.content_type as u32,
			self.header.content_id.iter().map(|b| format!("{:02X}", b)).collect::<String>(),
		)
	}
}

impl XContentPackage {
	pub fn parse(input: impl AsRef<[u8]>) -> Result<Self, XContentError> {
		Self::parse_inner(input.as_ref())
	}

	fn parse_inner(input: &[u8]) -> Result<Self, XContentError> {
		let header = XContentHeader::parse(input)?;

		let inner_fs = match &header.metadata.volume_descriptor {
			FileSystem::Stfs(_) => {
				let reader = stfs::SliceReader(input);
				let package = StfsPackage::open(&reader)?;
				XboxFilesystem::Stfs(package)
			}
			FileSystem::Svod(_) => todo!(),
			FileSystem::Fatx => todo!(),
		};

		Ok(XContentPackage { header, inner_file_system: inner_fs })
	}
}

#[derive(Default, Clone, Debug, Serialize)]
pub struct StfsVolumeDescriptor {
	pub size: u8,
	pub version: u8,
	pub flags: u8,
	pub file_table_block_count: u16,
	pub file_table_block_num: u32,
	pub top_hash_table_hash: [u8; SHA1_DIGEST_SIZE],
	pub allocated_block_count: u32,
	pub unallocated_block_count: u32,
}

impl StfsVolumeDescriptor {
	fn parse(cursor: &mut Cursor<&[u8]>) -> std::io::Result<Self> {
		Ok(StfsVolumeDescriptor {
			size: cursor.read_u8()?,
			version: cursor.read_u8()?,
			flags: cursor.read_u8()?,
			file_table_block_count: cursor.read_u16::<LittleEndian>()?,
			file_table_block_num: cursor.read_u24::<LittleEndian>()?,
			top_hash_table_hash: {
				let mut buf = [0u8; 0x14];
				cursor.read_exact(&mut buf)?;
				buf
			},
			allocated_block_count: cursor.read_u32::<BigEndian>()?,
			unallocated_block_count: cursor.read_u32::<BigEndian>()?,
		})
	}
}

#[derive(Debug, Serialize)]
pub struct SvodVolumeDescriptor {
	pub size: u8,
	pub block_cache_element_count: u8,
	pub worker_thread_processor: u8,
	pub worker_thread_priority: u8,
	pub root_hash: [u8; SHA1_DIGEST_SIZE],
	pub flags: u8,
	pub data_block_count: u32,
	pub data_block_offset: u32,
	pub reserved: [u8; 5],
}

impl SvodVolumeDescriptor {
	fn parse(cursor: &mut Cursor<&[u8]>) -> std::io::Result<Self> {
		Ok(SvodVolumeDescriptor {
			size: cursor.read_u8()?,
			block_cache_element_count: cursor.read_u8()?,
			worker_thread_processor: cursor.read_u8()?,
			worker_thread_priority: cursor.read_u8()?,
			root_hash: {
				let mut buf = [0u8; 0x14];
				cursor.read_exact(&mut buf)?;
				buf
			},
			flags: cursor.read_u8()?,
			data_block_count: cursor.read_u24::<BigEndian>()?,
			data_block_offset: cursor.read_u24::<BigEndian>()?,
			reserved: {
				let mut buf = [0u8; 5];
				cursor.read_exact(&mut buf)?;
				buf
			},
		})
	}
}

use num_enum::TryFromPrimitive;

#[derive(Debug, Serialize, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, TryFromPrimitive)]
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
	MarketplaceContent = 2,
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

#[derive(Debug, Serialize, Copy, Clone, TryFromPrimitive)]
#[repr(u32)]
pub enum InstallerType {
	None = 0,
	/// "SUPD" -- system-level dashboard/kernel update.
	SystemUpdate = 0x53555044,
	/// "TUPD" -- per-title update patch.
	TitleUpdate = 0x54555044,
	/// "P$SU" -- partial download state for a system update.
	SystemUpdateProgressCache = 0x50245355,
	/// "P$TU" -- partial download state for a title update.
	TitleUpdateProgressCache = 0x50245455,
	/// "P$TC" -- partial download state for title content.
	TitleContentProgressCache = 0x50245443,
}

#[derive(Debug, Serialize, Copy, Clone, PartialEq, Eq, TryFromPrimitive)]
#[repr(u32)]
pub enum FileSystemKind {
	/// Secure Transacted File System. Used by CON/LIVE/PIRS packages
	/// for savegames, DLC, title updates, and profile data.
	Stfs = 0,
	/// Streamed VOD. Used for large on-demand content like Games on
	/// Demand and video.
	Svod,
	/// FAT-based filesystem. Used on hard drives and USB storage.
	Fatx,
}

#[derive(Debug, Serialize)]
pub enum FileSystem {
	Stfs(StfsVolumeDescriptor),
	Svod(SvodVolumeDescriptor),
	Fatx,
}

impl Default for FileSystem {
	fn default() -> Self {
		FileSystem::Stfs(StfsVolumeDescriptor::default())
	}
}

#[derive(Default, Debug, Serialize, Copy, Clone)]
pub struct LicenseEntry {
	pub ty_raw: u16,
	pub data: [u8; 6],
	pub bits: u32,
	pub flags: u32,
}
