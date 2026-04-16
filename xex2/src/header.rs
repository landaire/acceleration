//! XEX2 file header and security info.
//!
//! The on-disk layout of an XEX file is:
//!
//! ```text
//! +0x00  "XEX2" magic
//! +0x04  ModuleFlags
//! +0x08  data_offset        (start of the encrypted/compressed payload)
//! +0x0C  reserved
//! +0x10  security_offset    (offset of SecurityInfo)
//! +0x14  optional_header_count
//! +0x18  optional_header_table[optional_header_count]
//! ...
//! ```
//!
//! Optional headers are `(key, value)` pairs where the low byte of the key
//! encodes the size class: `0x00` = scalar value, `0x01` = inline pointer,
//! `0xFF` = variable-length data (offset with size prefix), `N` = fixed
//! `N*4`-byte struct. See [`OptionalHeaderKey`] for recognized keys.
//!
//! The [`SecurityInfo`] at `security_offset` contains the RSA signature,
//! image hash, AES file key, and [`ImageInfo`] (media restrictions, flags,
//! hashes). The kernel verifies this region using `XeCryptRotSumSha` and
//! the PIRS RSA public key before loading.

use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use num_enum::TryFromPrimitive;
#[cfg(feature = "serde")]
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Cursor;
use std::io::Read;

use crate::error::IoResultExt;
use crate::error::Result;
use crate::error::Xex2Error;
use rootcause::IntoReport;

pub use xenon_types::AesKey;
pub use xenon_types::MediaId;
pub use xenon_types::TitleId;
pub use xenon_types::VirtualAddress;

pub const XEX2_MAGIC: [u8; 4] = *b"XEX2";

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Xex2Header {
	pub module_flags: ModuleFlags,
	pub data_offset: u32,
	pub security_offset: u32,
	pub optional_header_count: u32,
	pub optional_headers: BTreeMap<u32, OptionalHeaderValue>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub enum OptionalHeaderValue {
	Inline(u32),
	Data(Vec<u8>),
}

pub use crate::opt::AllowedMediaTypes;
pub use crate::opt::ImageFlags;
pub use crate::opt::ModuleFlags;

#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[repr(u16)]
pub enum CompressionType {
	None = 0,
	/// Data blocks followed by zero-fill blocks. Each block is described
	/// by a (data_size, zero_size) pair in the FileFormatInfo header.
	Basic = 1,
	/// LZX-compressed blocks with u16 BE chunk-size prefixes. Each block
	/// has a 24-byte header containing the next block's size and a SHA-1
	/// hash for integrity verification. LZX state persists across blocks.
	Normal = 2,
	/// Binary diff against a base XEX. Used by XEXP patch files.
	Delta = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[repr(u16)]
pub enum EncryptionType {
	/// Data is stored in plaintext. Devkit XEXs and extracted basefiles.
	None = 0,
	/// AES-128-CBC with a per-file session key derived from the image
	/// key in SecurityInfo. The session key is decrypted using either
	/// the retail or devkit master key.
	Normal = 1,
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct FileFormatInfo {
	pub encryption_type: EncryptionType,
	pub compression_type: CompressionType,
	pub blocks: Vec<BasicCompressionBlock>,
	pub window_size: Option<u32>,
	pub first_block_size: Option<u32>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct BasicCompressionBlock {
	pub data_size: u32,
	pub zero_size: u32,
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct NormalCompressionInfo {
	pub window_size: u32,
	pub first_block: NormalCompressionBlock,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct NormalCompressionBlock {
	pub block_size: u32,
	pub hash: [u8; 20],
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct SecurityInfo {
	pub header_size: u32,
	pub image_size: u32,
	#[cfg_attr(feature = "serde", serde(skip))]
	pub rsa_signature: [u8; 256],
	pub image_info: ImageInfo,
	pub page_descriptor_count: u32,
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct ImageInfo {
	pub info_size: u32,
	pub image_flags: ImageFlags,
	pub load_address: VirtualAddress,
	pub image_hash: [u8; 20],
	pub import_table_count: u32,
	pub import_table_hash: [u8; 20],
	pub media_id: [u8; 16],
	pub file_key: AesKey,
	pub header_hash: [u8; 20],
	pub game_regions: u32,
	pub allowed_media_types: AllowedMediaTypes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize))]
#[repr(u32)]
pub enum OptionalHeaderKey {
	ResourceInfo = 0x000002FF,
	FileFormatInfo = 0x000003FF,
	BaseReference = 0x00000405,
	DeltaPatchDescriptor = 0x000005FF,
	BoundingPath = 0x000080FF,
	DeviceId = 0x00008105,
	OriginalBaseAddress = 0x00010001,
	TlsInfo = 0x000100FF,
	EntryPoint = 0x00010100,
	DefaultStackSize = 0x00010200,
	DefaultFsCacheSize = 0x000102FF,
	DefaultHeapSize = 0x00010301,
	PageHeapSizeAndFlags = 0x00010302,
	ImportLibraries = 0x000103FF,
	ChecksumTimestamp = 0x00018002,
	OriginalPeName = 0x000183FF,
	StaticLibraries = 0x000200FF,
	BuildVersions = 0x00020104,
	TlsData = 0x00020200,
	SystemFlags = 0x00030000,
	Privileges = 0x00030100,
	KvPrivilegeRequirement = 0x00004004,
	DateRange = 0x00004104,
	ConsoleSerialList = 0x000042FF,
	ExecutionInfo = 0x00040006,
	ServiceIdList = 0x000401FF,
	TitleWorkspaceSize = 0x00040201,
	GameRatings = 0x00040310,
	LanKey = 0x00040404,
	Xbox360Logo = 0x000405FF,
	MultidiscMediaIds = 0x000406FF,
	AlternateTitleIds = 0x000407FF,
	AdditionalTitleMemory = 0x00040801,
	ExportsByName = 0x00E10402,
}

impl OptionalHeaderKey {
	pub fn from_u32(value: u32) -> Option<Self> {
		match value {
			0x000002FF => Some(Self::ResourceInfo),
			0x000003FF => Some(Self::FileFormatInfo),
			0x00000405 => Some(Self::BaseReference),
			0x000005FF => Some(Self::DeltaPatchDescriptor),
			0x000080FF => Some(Self::BoundingPath),
			0x00008105 => Some(Self::DeviceId),
			0x00010001 => Some(Self::OriginalBaseAddress),
			0x000100FF => Some(Self::TlsInfo),
			0x00010100 => Some(Self::EntryPoint),
			0x00010200 | 0x00010201 => Some(Self::DefaultStackSize),
			0x000102FF => Some(Self::DefaultFsCacheSize),
			0x00010301 => Some(Self::DefaultHeapSize),
			0x00010302 => Some(Self::PageHeapSizeAndFlags),
			0x000103FF => Some(Self::ImportLibraries),
			0x00018002 => Some(Self::ChecksumTimestamp),
			0x000183FF => Some(Self::OriginalPeName),
			0x000200FF => Some(Self::StaticLibraries),
			0x00020104 => Some(Self::BuildVersions),
			0x00020200 => Some(Self::TlsData),
			0x00030000 => Some(Self::SystemFlags),
			0x00030100 => Some(Self::Privileges),
			0x00004004 => Some(Self::KvPrivilegeRequirement),
			0x00004104 => Some(Self::DateRange),
			0x000042FF => Some(Self::ConsoleSerialList),
			0x00040006 => Some(Self::ExecutionInfo),
			0x000401FF => Some(Self::ServiceIdList),
			0x00040201 => Some(Self::TitleWorkspaceSize),
			0x00040310 => Some(Self::GameRatings),
			0x00040404 => Some(Self::LanKey),
			0x000405FF => Some(Self::Xbox360Logo),
			0x000406FF => Some(Self::MultidiscMediaIds),
			0x000407FF => Some(Self::AlternateTitleIds),
			0x00040801 => Some(Self::AdditionalTitleMemory),
			0x00E10402 => Some(Self::ExportsByName),
			_ => None,
		}
	}
}

impl std::fmt::Display for OptionalHeaderKey {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{:?}", self)
	}
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct ExecutionInfo {
	pub media_id: MediaId,
	pub version: u32,
	pub base_version: u32,
	pub title_id: TitleId,
	pub platform: u8,
	pub executable_table: u8,
	pub disc_number: u8,
	pub disc_count: u8,
	pub savegame_id: u32,
}

impl Xex2Header {
	pub fn parse(data: impl AsRef<[u8]>) -> Result<Self> {
		Self::parse_inner(data.as_ref())
	}

	fn parse_inner(data: &[u8]) -> Result<Self> {
		let mut cursor = Cursor::new(data);

		let mut magic = [0u8; 4];
		cursor.read_exact(&mut magic).io()?;
		if magic != XEX2_MAGIC {
			return Err(Xex2Error::InvalidMagic { found: magic }.into_report());
		}

		let module_flags = ModuleFlags::from_bits_retain(cursor.read_u32::<BigEndian>().io()?);
		let data_offset = cursor.read_u32::<BigEndian>().io()?;
		// Reserved field -- always zero in practice.
		cursor.read_u32::<BigEndian>().io()?;
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

			const SIZE_CLASS_MASK: u32 = 0xFF;
			const SIZE_CLASS_SCALAR: u32 = 0x00;
			const SIZE_CLASS_INLINE_PTR: u32 = 0x01;
			const SIZE_CLASS_VARIABLE: u32 = 0xFF;

			let size_class = key & SIZE_CLASS_MASK;
			let header_value = match size_class {
				SIZE_CLASS_SCALAR | SIZE_CLASS_INLINE_PTR => OptionalHeaderValue::Inline(value),
				SIZE_CLASS_VARIABLE => {
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

	pub fn get_optional_header(&self, key: OptionalHeaderKey) -> Option<&OptionalHeaderValue> {
		self.optional_headers.get(&(key as u32))
	}

	pub fn get_optional_data(&self, key: OptionalHeaderKey) -> Option<&[u8]> {
		match self.get_optional_header(key)? {
			OptionalHeaderValue::Data(data) => Some(data),
			OptionalHeaderValue::Inline(_) => None,
		}
	}

	pub fn get_optional_inline(&self, key: OptionalHeaderKey) -> Option<u32> {
		match self.get_optional_header(key)? {
			OptionalHeaderValue::Inline(v) => Some(*v),
			OptionalHeaderValue::Data(_) => None,
		}
	}

	pub fn entry_point(&self) -> Option<u32> {
		self.get_optional_inline(OptionalHeaderKey::EntryPoint)
	}

	pub fn original_base_address(&self) -> Option<u32> {
		self.get_optional_inline(OptionalHeaderKey::OriginalBaseAddress)
	}

	pub fn default_stack_size(&self) -> Option<u32> {
		self.get_optional_inline(OptionalHeaderKey::DefaultStackSize)
	}

	pub fn date_range(&self) -> Option<crate::opt::DateRange> {
		let data = self.get_optional_data(OptionalHeaderKey::DateRange)?;
		if data.len() < 16 {
			return None;
		}
		let not_before = xenon_types::filetime_from_xe_bytes(data[0..8].try_into().unwrap());
		let not_after = xenon_types::filetime_from_xe_bytes(data[8..16].try_into().unwrap());
		Some(crate::opt::DateRange {
			not_before: if not_before == 0 { None } else { Some(not_before) },
			not_after: if not_after == 0 { None } else { Some(not_after) },
		})
	}

	pub fn execution_info(&self) -> Option<ExecutionInfo> {
		let data = self.get_optional_data(OptionalHeaderKey::ExecutionInfo)?;
		if data.len() < 24 {
			return None;
		}
		let mut c = Cursor::new(data);
		Some(ExecutionInfo {
			media_id: MediaId(c.read_u32::<BigEndian>().ok()?),
			version: c.read_u32::<BigEndian>().ok()?,
			base_version: c.read_u32::<BigEndian>().ok()?,
			title_id: TitleId(c.read_u32::<BigEndian>().ok()?),
			platform: c.read_u8().ok()?,
			executable_table: c.read_u8().ok()?,
			disc_number: c.read_u8().ok()?,
			disc_count: c.read_u8().ok()?,
			savegame_id: c.read_u32::<BigEndian>().ok()?,
		})
	}

	pub fn file_format_info(&self) -> Result<FileFormatInfo> {
		let header_data = match self.get_optional_header(OptionalHeaderKey::FileFormatInfo) {
			Some(OptionalHeaderValue::Data(d)) => d.as_slice(),
			Some(OptionalHeaderValue::Inline(_)) => {
				return Err(Xex2Error::InvalidOptionalHeaderSize {
					key: OptionalHeaderKey::FileFormatInfo as u32,
					size: 4,
				}
				.into_report());
			}
			None => {
				return Err(Xex2Error::MissingOptionalHeader(OptionalHeaderKey::FileFormatInfo as u32).into_report());
			}
		};

		let mut c = Cursor::new(header_data);
		// info_size -- redundant with the length of `header_data` itself.
		c.read_u32::<BigEndian>().io()?;
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
	pub fn parse(data: impl AsRef<[u8]>, offset: usize) -> Result<Self> {
		Self::parse_inner(data.as_ref(), offset)
	}

	fn parse_inner(data: &[u8], offset: usize) -> Result<Self> {
		let mut c = Cursor::new(&data[offset..]);

		let header_size = c.read_u32::<BigEndian>().io()?;
		let image_size = c.read_u32::<BigEndian>().io()?;

		let mut rsa_signature = [0u8; 256];
		c.read_exact(&mut rsa_signature).io()?;

		let info_size = c.read_u32::<BigEndian>().io()?;

		let image_info = ImageInfo {
			info_size,
			image_flags: ImageFlags::from_bits_retain(c.read_u32::<BigEndian>().io()?),
			load_address: VirtualAddress(c.read_u32::<BigEndian>().io()?),
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
			file_key: AesKey({
				let mut k = [0u8; 16];
				c.read_exact(&mut k).io()?;
				k
			}),
			header_hash: {
				let mut h = [0u8; 20];
				c.read_exact(&mut h).io()?;
				h
			},
			game_regions: c.read_u32::<BigEndian>().io()?,
			allowed_media_types: AllowedMediaTypes::from_bits_retain(c.read_u32::<BigEndian>().io()?),
		};

		let page_descriptor_count = c.read_u32::<BigEndian>().io()?;

		Ok(SecurityInfo { header_size, image_size, rsa_signature, image_info, page_descriptor_count })
	}
}

fn cursor_read_u32_at(data: &[u8], offset: usize) -> Result<u32> {
	let mut c = Cursor::new(&data[offset..]);
	c.read_u32::<BigEndian>().io()
}
