use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use num_enum::TryFromPrimitive;
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Cursor;
use std::io::Read;

use crate::error::IoResultExt;
use crate::error::Result;
use crate::error::Xex2Error;
use rootcause::IntoReport;

pub const XEX2_MAGIC: [u8; 4] = *b"XEX2";

#[derive(Debug, Serialize)]
pub struct Xex2Header {
	pub module_flags: ModuleFlags,
	pub data_offset: u32,
	pub security_offset: u32,
	pub optional_header_count: u32,
	pub optional_headers: BTreeMap<u32, OptionalHeaderValue>,
}

#[derive(Debug, Clone, Serialize)]
pub enum OptionalHeaderValue {
	Inline(u32),
	Data(Vec<u8>),
}

#[derive(Debug, Serialize)]
pub struct ModuleFlags(pub u32);

impl ModuleFlags {
	pub fn is_title(&self) -> bool {
		self.0 & 0x01 != 0
	}

	pub fn is_exports_to_title(&self) -> bool {
		self.0 & 0x02 != 0
	}

	pub fn is_system_debugger(&self) -> bool {
		self.0 & 0x04 != 0
	}

	pub fn is_dll(&self) -> bool {
		self.0 & 0x08 != 0
	}

	pub fn is_patch(&self) -> bool {
		self.0 & 0x10 != 0
	}

	pub fn is_patch_full(&self) -> bool {
		self.0 & 0x20 != 0
	}

	pub fn is_patch_delta(&self) -> bool {
		self.0 & 0x40 != 0
	}

	pub fn is_user_mode(&self) -> bool {
		self.0 & 0x80 != 0
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive, Serialize)]
#[repr(u16)]
pub enum CompressionType {
	None = 0,
	Basic = 1,
	Normal = 2,
	Delta = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive, Serialize)]
#[repr(u16)]
pub enum EncryptionType {
	None = 0,
	Normal = 1,
}

#[derive(Debug, Serialize)]
pub struct FileFormatInfo {
	pub encryption_type: EncryptionType,
	pub compression_type: CompressionType,
	pub blocks: Vec<BasicCompressionBlock>,
	pub window_size: Option<u32>,
	pub first_block_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BasicCompressionBlock {
	pub data_size: u32,
	pub zero_size: u32,
}

#[derive(Debug, Serialize)]
pub struct NormalCompressionInfo {
	pub window_size: u32,
	pub first_block: NormalCompressionBlock,
}

#[derive(Debug, Clone, Serialize)]
pub struct NormalCompressionBlock {
	pub block_size: u32,
	pub hash: [u8; 20],
}

#[derive(Debug, Serialize)]
pub struct SecurityInfo {
	pub header_size: u32,
	pub image_size: u32,
	#[serde(skip)]
	pub rsa_signature: [u8; 256],
	pub image_info: ImageInfo,
	pub page_descriptor_count: u32,
}

#[derive(Debug, Serialize)]
pub struct ImageInfo {
	pub info_size: u32,
	pub image_flags: u32,
	pub load_address: u32,
	pub image_hash: [u8; 20],
	pub import_table_count: u32,
	pub import_table_hash: [u8; 20],
	pub media_id: [u8; 16],
	pub file_key: [u8; 16],
	pub header_hash: [u8; 20],
	pub game_regions: u32,
	pub allowed_media_types: u32,
}

pub mod optional_header_keys {
	pub const RESOURCE_INFO: u32 = 0x000002FF;
	pub const FILE_FORMAT_INFO: u32 = 0x000003FF;
	pub const BASE_REFERENCE: u32 = 0x00000405;
	pub const DELTA_PATCH_DESCRIPTOR: u32 = 0x000005FF;
	pub const BOUNDING_PATH: u32 = 0x000080FF;
	pub const DEVICE_ID: u32 = 0x00008105;
	pub const ORIGINAL_BASE_ADDRESS: u32 = 0x00010001;
	pub const ENTRY_POINT: u32 = 0x00010100;
	pub const TLS_INFO: u32 = 0x000100FF;
	pub const DEFAULT_STACK_SIZE: u32 = 0x00010200;
	pub const DEFAULT_FS_CACHE_SIZE: u32 = 0x000102FF;
	pub const DEFAULT_HEAP_SIZE: u32 = 0x00010301;
	pub const PAGE_HEAP_SIZE_AND_FLAGS: u32 = 0x00010302;
	pub const IMPORT_LIBRARIES: u32 = 0x000103FF;
	pub const EXECUTION_INFO: u32 = 0x00040006;
	pub const SERVICE_ID_LIST: u32 = 0x000401FF;
	pub const TITLE_WORKSPACE_SIZE: u32 = 0x00040201;
	pub const GAME_RATINGS: u32 = 0x00040310;
	pub const LAN_KEY: u32 = 0x00040404;
	pub const XBOX_360_LOGO: u32 = 0x000405FF;
	pub const MULTIDISC_MEDIA_IDS: u32 = 0x000406FF;
	pub const ALTERNATE_TITLE_IDS: u32 = 0x000407FF;
	pub const ADDITIONAL_TITLE_MEMORY: u32 = 0x00040801;
	pub const EXPORTS_BY_NAME: u32 = 0x00E10402;
}

#[derive(Debug, Serialize)]
pub struct ExecutionInfo {
	pub media_id: u32,
	pub version: u32,
	pub base_version: u32,
	pub title_id: u32,
	pub platform: u8,
	pub executable_table: u8,
	pub disc_number: u8,
	pub disc_count: u8,
	pub savegame_id: u32,
}

impl Xex2Header {
	pub fn parse(data: &[u8]) -> Result<Self> {
		let mut cursor = Cursor::new(data);

		let mut magic = [0u8; 4];
		cursor.read_exact(&mut magic).io()?;
		if magic != XEX2_MAGIC {
			return Err(Xex2Error::InvalidMagic { found: magic }.into_report());
		}

		let module_flags = ModuleFlags(cursor.read_u32::<BigEndian>().io()?);
		let data_offset = cursor.read_u32::<BigEndian>().io()?;
		let _reserved = cursor.read_u32::<BigEndian>().io()?;
		let security_offset = cursor.read_u32::<BigEndian>().io()?;
		let optional_header_count = cursor.read_u32::<BigEndian>().io()?;

		if data_offset as usize > data.len() {
			return Err(Xex2Error::InvalidHeaderOffset { offset: data_offset, file_size: data.len() }.into_report());
		}

		if security_offset as usize > data.len() {
			return Err(
				Xex2Error::InvalidSecurityOffset { offset: security_offset, file_size: data.len() }.into_report()
			);
		}

		let mut optional_headers = BTreeMap::new();
		for _ in 0..optional_header_count {
			let key = cursor.read_u32::<BigEndian>().io()?;
			let value = cursor.read_u32::<BigEndian>().io()?;

			let size_class = key & 0xFF;
			let header_value = match size_class {
				0x00 | 0x01 => OptionalHeaderValue::Inline(value),
				0xFF => {
					let offset = value as usize;
					let struct_size = cursor_read_u32_at(data, offset)? as usize;
					if offset + struct_size > data.len() {
						return Err(Xex2Error::InvalidOptionalHeaderSize { key, size: struct_size }.into_report());
					}
					OptionalHeaderValue::Data(data[offset..offset + struct_size].to_vec())
				}
				n => {
					let byte_count = (n as usize) * 4;
					let offset = value as usize;
					if offset + byte_count > data.len() {
						return Err(Xex2Error::InvalidOptionalHeaderSize { key, size: byte_count }.into_report());
					}
					OptionalHeaderValue::Data(data[offset..offset + byte_count].to_vec())
				}
			};

			optional_headers.insert(key, header_value);
		}

		Ok(Xex2Header { module_flags, data_offset, security_offset, optional_header_count, optional_headers })
	}

	pub fn get_optional_header(&self, key: u32) -> Option<&OptionalHeaderValue> {
		self.optional_headers.get(&key)
	}

	pub fn get_optional_data(&self, key: u32) -> Option<&[u8]> {
		match self.optional_headers.get(&key)? {
			OptionalHeaderValue::Data(data) => Some(data),
			OptionalHeaderValue::Inline(_) => None,
		}
	}

	pub fn get_optional_inline(&self, key: u32) -> Option<u32> {
		match self.optional_headers.get(&key)? {
			OptionalHeaderValue::Inline(v) => Some(*v),
			OptionalHeaderValue::Data(_) => None,
		}
	}

	pub fn entry_point(&self) -> Option<u32> {
		self.get_optional_inline(optional_header_keys::ENTRY_POINT)
	}

	pub fn original_base_address(&self) -> Option<u32> {
		self.get_optional_inline(optional_header_keys::ORIGINAL_BASE_ADDRESS)
	}

	pub fn default_stack_size(&self) -> Option<u32> {
		self.get_optional_inline(optional_header_keys::DEFAULT_STACK_SIZE)
	}

	pub fn execution_info(&self) -> Option<ExecutionInfo> {
		let data = self.get_optional_data(optional_header_keys::EXECUTION_INFO)?;
		if data.len() < 24 {
			return None;
		}
		let mut c = Cursor::new(data);
		Some(ExecutionInfo {
			media_id: c.read_u32::<BigEndian>().ok()?,
			version: c.read_u32::<BigEndian>().ok()?,
			base_version: c.read_u32::<BigEndian>().ok()?,
			title_id: c.read_u32::<BigEndian>().ok()?,
			platform: c.read_u8().ok()?,
			executable_table: c.read_u8().ok()?,
			disc_number: c.read_u8().ok()?,
			disc_count: c.read_u8().ok()?,
			savegame_id: c.read_u32::<BigEndian>().ok()?,
		})
	}

	pub fn file_format_info(&self, _data: &[u8]) -> Result<FileFormatInfo> {
		let header_data = match self.get_optional_header(optional_header_keys::FILE_FORMAT_INFO) {
			Some(OptionalHeaderValue::Data(d)) => d.as_slice(),
			Some(OptionalHeaderValue::Inline(_)) => {
				return Err(Xex2Error::InvalidOptionalHeaderSize {
					key: optional_header_keys::FILE_FORMAT_INFO,
					size: 4,
				}
				.into_report())
			}
			None => return Err(Xex2Error::MissingOptionalHeader(optional_header_keys::FILE_FORMAT_INFO).into_report()),
		};

		let mut c = Cursor::new(header_data);
		let _info_size = c.read_u32::<BigEndian>().io()?;
		let encryption_type = EncryptionType::try_from(c.read_u16::<BigEndian>().io()?)
			.map_err(|e| Xex2Error::InvalidEncryptionType(e.number).into_report())?;
		let compression_type = CompressionType::try_from(c.read_u16::<BigEndian>().io()?)
			.map_err(|e| Xex2Error::InvalidCompressionFormat(e.number).into_report())?;

		let mut blocks = Vec::new();
		let mut window_size = None;
		let mut first_block_size = None;

		match compression_type {
			CompressionType::Basic => {
				while (c.position() as usize) < header_data.len() {
					blocks.push(BasicCompressionBlock {
						data_size: c.read_u32::<BigEndian>().io()?,
						zero_size: c.read_u32::<BigEndian>().io()?,
					});
				}
			}
			CompressionType::Normal => {
				window_size = Some(c.read_u32::<BigEndian>().io()?);
				first_block_size = Some(c.read_u32::<BigEndian>().io()?);
				// Skip the 20-byte hash of the first block
			}
			_ => {}
		}

		Ok(FileFormatInfo { encryption_type, compression_type, blocks, window_size, first_block_size })
	}
}

impl SecurityInfo {
	pub fn parse(data: &[u8], offset: usize) -> Result<Self> {
		let mut c = Cursor::new(&data[offset..]);

		let header_size = c.read_u32::<BigEndian>().io()?;
		let image_size = c.read_u32::<BigEndian>().io()?;

		let mut rsa_signature = [0u8; 256];
		c.read_exact(&mut rsa_signature).io()?;

		let _info_size = c.read_u32::<BigEndian>().io()?;

		let image_info = ImageInfo {
			info_size: _info_size,
			image_flags: c.read_u32::<BigEndian>().io()?,
			load_address: c.read_u32::<BigEndian>().io()?,
			image_hash: {
				let mut h = [0u8; 20];
				c.read_exact(&mut h).io()?;
				h
			},
			import_table_count: c.read_u32::<BigEndian>().io()?,
			import_table_hash: {
				let mut h = [0u8; 20];
				c.read_exact(&mut h).io()?;
				h
			},
			media_id: {
				let mut m = [0u8; 16];
				c.read_exact(&mut m).io()?;
				m
			},
			file_key: {
				let mut k = [0u8; 16];
				c.read_exact(&mut k).io()?;
				k
			},
			header_hash: {
				let mut h = [0u8; 20];
				c.read_exact(&mut h).io()?;
				h
			},
			game_regions: c.read_u32::<BigEndian>().io()?,
			allowed_media_types: c.read_u32::<BigEndian>().io()?,
		};

		let page_descriptor_count = c.read_u32::<BigEndian>().io()?;

		Ok(SecurityInfo { header_size, image_size, rsa_signature, image_info, page_descriptor_count })
	}
}

fn cursor_read_u32_at(data: &[u8], offset: usize) -> Result<u32> {
	let mut c = Cursor::new(&data[offset..]);
	c.read_u32::<BigEndian>().io()
}
