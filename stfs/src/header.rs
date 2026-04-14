use byteorder::BigEndian;
use byteorder::LittleEndian;
use byteorder::ReadBytesExt;
use serde::Serialize;
use std::io::Cursor;
use std::io::Read;

use crate::error::StfsError;
use crate::serde_bytes;
use crate::types::*;

const INVALID_STR: &str = "<INVALID>";

fn read_array<const N: usize>(cursor: &mut Cursor<&[u8]>) -> Result<[u8; N], StfsError> {
	let mut buf = [0u8; N];
	cursor.read_exact(&mut buf)?;
	Ok(buf)
}

fn read_bytes(cursor: &mut Cursor<&[u8]>, n: usize) -> Result<Vec<u8>, StfsError> {
	let mut buf = vec![0u8; n];
	cursor.read_exact(&mut buf)?;
	Ok(buf)
}

fn read_utf16_cstr(cursor: &mut Cursor<&[u8]>, input: &[u8]) -> String {
	let position = cursor.position() as usize;

	let mut end_of_str_position = None;
	for i in (0..input.len() - position).step_by(2) {
		if input[position + i] == 0 && input[position + i + 1] == 0 {
			end_of_str_position = Some(position + i);
			break;
		}
	}

	let end_of_str_position = end_of_str_position.expect("failed to find null terminator");
	cursor.set_position((end_of_str_position + 2) as u64);

	let byte_range = &input[position..end_of_str_position];
	let mut utf16_str = Vec::with_capacity(byte_range.len() / 2);
	for chunk in byte_range.chunks(2) {
		utf16_str.push(((chunk[0] as u16) << 8) | chunk[1] as u16);
	}

	String::from_utf16(utf16_str.as_slice()).expect("failed to convert data to utf16")
}

#[derive(Debug, Serialize)]
pub struct Certificate {
	pub pubkey_cert_size: u16,
	pub owner_console_id: [u8; 5],
	pub owner_console_part_number: String,
	pub owner_console_type: Option<ConsoleType>,
	pub console_type_flags: Option<ConsoleTypeFlags>,
	pub date_generation: String,
	pub public_exponent: u32,
	#[serde(with = "serde_bytes::fixed")]
	pub public_modulus: [u8; 0x80],
	#[serde(with = "serde_bytes::fixed")]
	pub certificate_signature: [u8; 0x100],
	#[serde(with = "serde_bytes::fixed")]
	pub signature: [u8; 0x80],
}

#[derive(Debug, Serialize)]
pub struct AvatarAssetInformation {
	pub subcategory: AssetSubcategory,
	pub colorizable: u32,
	pub guid: [u8; 0x10],
	pub skeleton_version: SkeletonVersion,
}

#[derive(Debug, Serialize)]
pub struct MediaInformation {
	pub series_id: [u8; 0x10],
	pub season_id: [u8; 0x10],
	pub season_number: u16,
	pub episode_number: u16,
}

#[derive(Debug, Serialize)]
pub struct InstallerProgressCache {
	pub resume_state: OnlineContentResumeState,
	pub current_file_index: u32,
	pub current_file_offset: u64,
	pub bytes_processed: u64,
	pub last_modified_high: u32,
	pub last_modified_low: u32,
}

#[derive(Debug, Serialize)]
pub struct FullInstallerMeta {
	pub installer_base_version: Version,
	pub installer_version: Version,
}

#[derive(Debug, Serialize)]
pub enum InstallerMeta {
	FullInstaller(FullInstallerMeta),
	InstallerProgressCache(InstallerProgressCache),
}

#[derive(Debug, Serialize)]
pub enum ContentMetadata {
	AvatarItem(AvatarAssetInformation),
	Video(MediaInformation),
}

#[derive(Debug, Serialize)]
pub enum FileSystem {
	STFS(StfsVolumeDescriptor),
	SVOD(SvodVolumeDescriptor),
}

impl FileSystem {
	pub fn stfs_ref(&self) -> &StfsVolumeDescriptor {
		if let Self::STFS(vd) = self {
			vd
		} else {
			panic!("FileSystem is not an StfsVolumeDescriptor")
		}
	}

	pub fn svod_ref(&self) -> &SvodVolumeDescriptor {
		if let Self::SVOD(vd) = self {
			vd
		} else {
			panic!("FileSystem is not an SvodVolumeDescriptor")
		}
	}
}

#[derive(Debug, Serialize)]
pub struct StfsVolumeDescriptor {
	pub size: u8,
	pub reserved: u8,
	pub block_separation: u8,
	pub file_table_block_count: u16,
	/// Encoded as a 24-bit integer
	pub file_table_block_num: BlockNumber,
	#[serde(with = "serde_bytes::fixed")]
	pub top_hash_table_hash: [u8; 0x14],
	pub allocated_block_count: u32,
	pub unallocated_block_count: u32,
}

#[derive(Debug, Serialize)]
pub struct SvodVolumeDescriptor {
	pub size: u8,
	pub block_cache_element_count: u8,
	pub worker_thread_processor: u8,
	pub worker_thread_priority: u8,
	#[serde(with = "serde_bytes::fixed")]
	pub root_hash: [u8; 0x14],
	pub flags: u8,
	/// Encoded as an int24
	pub data_block_count: u32,
	/// Encoded as an int24
	pub data_block_offset: u32,
	pub reserved: [u8; 5],
}

#[derive(Debug, Serialize)]
pub struct XContentHeader {
	pub package_type: PackageType,
	/// Only present in console-signed packages
	pub certificate: Option<Certificate>,
	/// Only present in strong-signed packages
	#[serde(with = "serde_bytes::fixed_opt")]
	pub package_signature: Option<[u8; 0x100]>,

	pub license_data: [LicenseEntry; 0x10],
	#[serde(with = "serde_bytes::fixed")]
	pub header_hash: [u8; 0x14],
	pub header_size: u32,
	pub content_type: ContentType,
	pub metadata_version: u32,
	pub content_size: u64,
	pub media_id: u32,
	pub version: u32,
	pub base_version: u32,
	pub title_id: u32,
	pub platform: u8,
	pub executable_type: u8,
	pub disc_number: u8,
	pub disc_in_set: u8,
	pub savegame_id: u32,
	pub console_id: [u8; 5],
	pub profile_id: [u8; 8],
	pub volume_descriptor: FileSystem,
	pub filesystem_type: FileSystemType,
	/// Only in PEC
	pub enabled: bool,

	// Metadata v1
	pub data_file_count: u32,
	pub data_file_combined_size: u64,
	#[serde(with = "serde_bytes::fixed")]
	pub device_id: [u8; 0x14],
	pub display_name: String,
	pub display_description: String,
	pub publisher_name: String,
	pub title_name: String,
	pub transfer_flags: u8,
	pub thumbnail_image_size: usize,
	#[serde(with = "serde_bytes::vec")]
	pub thumbnail_image: Vec<u8>,
	pub title_thumbnail_image_size: usize,
	#[serde(with = "serde_bytes::vec")]
	pub title_image: Vec<u8>,
	pub installer_type: Option<InstallerType>,
	pub installer_meta: Option<InstallerMeta>,
	pub content_metadata: Option<ContentMetadata>,
}

impl XContentHeader {
	/// Returns which hash table level the root hash is in
	pub fn root_hash_table_level(&self) -> Result<HashTableLevel, StfsError> {
		if let FileSystem::STFS(vd) = &self.volume_descriptor {
			let level = if vd.allocated_block_count as usize <= HASHES_PER_HASH_TABLE {
				HashTableLevel::First
			} else if vd.allocated_block_count as usize <= HASHES_PER_HASH_TABLE_LEVEL[1] {
				HashTableLevel::Second
			} else if vd.allocated_block_count as usize <= HASHES_PER_HASH_TABLE_LEVEL[2] {
				HashTableLevel::Third
			} else {
				return Err(StfsError::InvalidHeader);
			};
			Ok(level)
		} else {
			Err(StfsError::InvalidPackageType)
		}
	}

	pub fn parse(input: &[u8]) -> Result<XContentHeader, StfsError> {
		parse_header_inner(input)
	}
}

fn parse_certificate(cursor: &mut Cursor<&[u8]>) -> Result<Certificate, StfsError> {
	let pubkey_cert_size = cursor.read_u16::<BigEndian>()?;
	let owner_console_id: [u8; 5] = read_array(cursor)?;

	let part_number_bytes: [u8; 0x11] = read_array(cursor)?;
	let end = part_number_bytes.iter().position(|b| *b == 0x0).unwrap_or(part_number_bytes.len());
	let owner_console_part_number =
		String::from_utf8(part_number_bytes[..end].to_vec()).unwrap_or_else(|_| INVALID_STR.into());

	let owner_console_type_raw = cursor.read_u32::<BigEndian>()?;
	let console_type_flags = ConsoleTypeFlags::from_bits(owner_console_type_raw & 0xFFFFFFFC);
	let owner_console_type = ConsoleType::try_from((owner_console_type_raw & 0x3) as u8).ok();

	let date_generation_bytes: [u8; 0x8] = read_array(cursor)?;
	let date_generation = String::from_utf8(date_generation_bytes.to_vec()).unwrap_or_else(|_| INVALID_STR.into());

	let public_exponent = cursor.read_u32::<BigEndian>()?;
	let public_modulus: [u8; 0x80] = read_array(cursor)?;
	let certificate_signature: [u8; 0x100] = read_array(cursor)?;
	let signature: [u8; 0x80] = read_array(cursor)?;

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

fn parse_avatar_asset_info(cursor: &mut Cursor<&[u8]>) -> Result<AvatarAssetInformation, StfsError> {
	let subcategory =
		AssetSubcategory::try_from(cursor.read_u32::<LittleEndian>()?).expect("invalid avatar asset subcategory");
	let colorizable = cursor.read_u32::<LittleEndian>()?;
	let guid: [u8; 0x10] = read_array(cursor)?;
	let skeleton_version = SkeletonVersion::try_from(cursor.read_u8()?).expect("invalid skeleton version");

	Ok(AvatarAssetInformation { subcategory, colorizable, guid, skeleton_version })
}

fn parse_media_info(cursor: &mut Cursor<&[u8]>) -> Result<MediaInformation, StfsError> {
	let series_id: [u8; 0x10] = read_array(cursor)?;
	let season_id: [u8; 0x10] = read_array(cursor)?;
	let season_number = cursor.read_u16::<BigEndian>()?;
	let episode_number = cursor.read_u16::<BigEndian>()?;

	Ok(MediaInformation { series_id, season_id, season_number, episode_number })
}

fn parse_stfs_volume_descriptor(cursor: &mut Cursor<&[u8]>) -> Result<StfsVolumeDescriptor, StfsError> {
	Ok(StfsVolumeDescriptor {
		size: cursor.read_u8()?,
		reserved: cursor.read_u8()?,
		block_separation: cursor.read_u8()?,
		file_table_block_count: cursor.read_u16::<LittleEndian>()?,
		file_table_block_num: BlockNumber(cursor.read_u24::<LittleEndian>()? as usize),
		top_hash_table_hash: read_array(cursor)?,
		allocated_block_count: cursor.read_u32::<BigEndian>()?,
		unallocated_block_count: cursor.read_u32::<BigEndian>()?,
	})
}

fn parse_svod_volume_descriptor(cursor: &mut Cursor<&[u8]>) -> Result<SvodVolumeDescriptor, StfsError> {
	Ok(SvodVolumeDescriptor {
		size: cursor.read_u8()?,
		block_cache_element_count: cursor.read_u8()?,
		worker_thread_processor: cursor.read_u8()?,
		worker_thread_priority: cursor.read_u8()?,
		root_hash: read_array(cursor)?,
		flags: cursor.read_u8()?,
		data_block_count: cursor.read_u24::<BigEndian>()?,
		data_block_offset: cursor.read_u24::<BigEndian>()?,
		reserved: read_array(cursor)?,
	})
}

fn parse_header_inner(input: &[u8]) -> Result<XContentHeader, StfsError> {
	let mut cursor = Cursor::new(input);

	let package_type = {
		let mut buf = [0u8; 4];
		cursor.read_exact(&mut buf)?;
		PackageType::try_from(buf)?
	};

	let certificate = if let PackageType::Con = &package_type { Some(parse_certificate(&mut cursor)?) } else { None };

	let package_signature = if matches!(package_type, PackageType::Live | PackageType::Pirs) {
		Some(read_array(&mut cursor)?)
	} else {
		None
	};

	cursor.set_position(0x22c);

	let mut license_data = [LicenseEntry::default(); 16];
	for entry in &mut license_data {
		let license = cursor.read_u64::<BigEndian>()?;
		entry.ty = LicenseType::try_from(u16::try_from(license >> 48).expect("failed to convert license type to u16"))
			.expect("invalid LicenseType");
		entry.data = license & 0xFFFFFFFFFFFF;
		entry.bits = cursor.read_u32::<BigEndian>()?;
		entry.flags = cursor.read_u32::<BigEndian>()?;
	}

	let header_hash: [u8; 0x14] = read_array(&mut cursor)?;
	let header_size = cursor.read_u32::<BigEndian>()?;
	let content_type = ContentType::try_from(cursor.read_u32::<BigEndian>()?).expect("invalid content type");
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

	let console_id: [u8; 5] = read_array(&mut cursor)?;
	let profile_id: [u8; 8] = read_array(&mut cursor)?;

	// Read filesystem type
	cursor.set_position(0x3a9);
	let filesystem_type = FileSystemType::try_from(cursor.read_u32::<BigEndian>()?).expect("invalid filesystem type");

	let volume_descriptor = match filesystem_type {
		FileSystemType::STFS => {
			cursor.set_position(0x379);
			FileSystem::STFS(parse_stfs_volume_descriptor(&mut cursor)?)
		}
		FileSystemType::SVOD => FileSystem::SVOD(parse_svod_volume_descriptor(&mut cursor)?),
		_ => panic!("Invalid filesystem type"),
	};

	let data_file_count = cursor.read_u32::<BigEndian>()?;
	let data_file_combined_size = cursor.read_u64::<BigEndian>()?;

	let content_metadata = match content_type {
		ContentType::AvatarItem => {
			cursor.set_position(0x3d9);
			Some(ContentMetadata::AvatarItem(parse_avatar_asset_info(&mut cursor)?))
		}
		ContentType::Video => {
			cursor.set_position(0x3d9);
			Some(ContentMetadata::Video(parse_media_info(&mut cursor)?))
		}
		_ => None,
	};

	cursor.set_position(0x3fd);
	let device_id: [u8; 0x14] = read_array(&mut cursor)?;

	let display_name = read_utf16_cstr(&mut cursor, input);

	cursor.set_position(0xD11);
	let display_description = read_utf16_cstr(&mut cursor, input);

	cursor.set_position(0x1611);
	let publisher_name = read_utf16_cstr(&mut cursor, input);

	cursor.set_position(0x1691);
	let title_name = read_utf16_cstr(&mut cursor, input);

	cursor.set_position(0x1711);
	let transfer_flags = cursor.read_u8()?;

	let thumbnail_image_size = cursor.read_u32::<BigEndian>()? as usize;
	let title_thumbnail_image_size = cursor.read_u32::<BigEndian>()? as usize;

	let thumbnail_image = read_bytes(&mut cursor, thumbnail_image_size)?;
	cursor.set_position(0x571a);

	let title_image = read_bytes(&mut cursor, title_thumbnail_image_size)?;
	cursor.set_position(0x971a);

	let mut installer_type = None;
	let mut installer_meta = None;
	if ((header_size + 0xFFF) & 0xFFFFF000) - 0x971A > 0x15F4 {
		installer_type = Some(InstallerType::try_from(cursor.read_u32::<BigEndian>()?).expect("invalid InstallerType"));
		installer_meta = match *installer_type.as_ref().unwrap() {
			InstallerType::SystemUpdate | InstallerType::TitleUpdate => {
				let installer_base_version = Version::from(cursor.read_u32::<BigEndian>()?);
				let installer_version = Version::from(cursor.read_u32::<BigEndian>()?);
				Some(InstallerMeta::FullInstaller(FullInstallerMeta { installer_base_version, installer_version }))
			}
			InstallerType::SystemUpdateProgressCache
			| InstallerType::TitleUpdateProgressCache
			| InstallerType::TitleContentProgressCache => {
				let resume_state =
					OnlineContentResumeState::try_from(cursor.read_u32::<BigEndian>()?).expect("invalid resume state");
				let current_file_index = cursor.read_u32::<BigEndian>()?;
				let current_file_offset = cursor.read_u64::<BigEndian>()?;
				let bytes_processed = cursor.read_u64::<BigEndian>()?;

				let last_modified_high = cursor.read_u32::<BigEndian>()?;
				let last_modified_low = cursor.read_u32::<BigEndian>()?;

				Some(InstallerMeta::InstallerProgressCache(InstallerProgressCache {
					resume_state,
					current_file_index,
					current_file_offset,
					bytes_processed,
					last_modified_high,
					last_modified_low,
				}))
			}
			_ => None,
		}
	}

	let enabled = false;
	Ok(XContentHeader {
		package_type,
		certificate,
		package_signature,
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
