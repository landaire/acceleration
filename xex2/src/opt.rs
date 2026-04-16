use bitflags::bitflags;
use byteorder::BigEndian;
use byteorder::ReadBytesExt;
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

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct GameRatings {
	pub esrb: u8,
	pub pegi: u8,
	pub pegifi: u8,
	pub pegipt: u8,
	pub bbfc: u8,
	pub cero: u8,
	pub usk: u8,
	pub oflcau: u8,
	pub oflcnz: u8,
	pub kmrb: u8,
	pub brazil: u8,
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
		const NO_FORCED_REBOOT         = 0x0000_0001;
		const FOREGROUND_TASKS         = 0x0000_0002;
		const NO_ODD_MAPPING           = 0x0000_0004;
		const HANDLES_GAMEPAD_DISCONNECT = 0x0000_0008;
		const INSECURE_SOCKETS         = 0x0000_0040;
		const XBOX1_INTEROPERABILITY   = 0x0000_0080;
		const DASH_CONTEXT             = 0x0000_0100;
		const USES_GAME_VOICE_CHANNEL  = 0x0000_0200;
		const PAL50_INCOMPATIBLE       = 0x0000_1000;
		const INSECURE_UTILITY_DRIVE   = 0x0000_2000;
		const XAM_HOOKS                = 0x0000_4000;
		const ACCESS_PIP               = 0x0000_8000;
		const PREFER_BIG_BUTTON_INPUT  = 0x0010_0000;
		const ALLOW_CONTROLLER_SWAPPING = 0x0200_0000;
		const ALLOW_KINECT             = 0x0400_0000;
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
		const SMALL_PAGES              = 0x1000_0000;
		const CONSOLE_ID_REQUIRED      = 0x0400_0000;
		/// Cleared by kernel if the HV reports the keyvault is unsigned
		/// (shared page 0x8E038614 bit 8 clear).
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
		const DVD_X2                   = 0x0000_0002;
		const DVD_CD                   = 0x0000_0004;
		const DVD_5                    = 0x0000_0008;
		const DVD_9                    = 0x0000_0010;
		const SYSTEM_FLASH             = 0x0000_0020;
		const MEMORY_UNIT              = 0x0000_0080;
		const USB_MASS_STORAGE         = 0x0000_0100;
		const NETWORK                  = 0x0000_0200;
		const DIRECT_FROM_MEMORY       = 0x0000_0400;
		const RAM_DRIVE                = 0x0000_1000;
		const SVOD                     = 0x0000_2000;
		const INSECURE_PACKAGE         = 0x0000_4000;
		const SAVEGAME_PACKAGE         = 0x0000_8000;
		const LOCALLY_SIGNED_PACKAGE   = 0x0001_0000;
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
		const TITLE            = 0x0000_0001;
		const EXPORTS_TO_TITLE = 0x0000_0002;
		const SYSTEM_DEBUGGER  = 0x0000_0004;
		const DLL              = 0x0000_0008;
		const PATCH            = 0x0000_0010;
		const PATCH_DELTA      = 0x0000_0020;
		const PATCH_FULL       = 0x0000_0040;
		const BOUND_PATH       = 0x4000_0000;
		const DEVICE_ID        = 0x2000_0000;
	}
}
#[cfg(feature = "serde")]
impl_bitflags_serialize!(ModuleFlags);

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
			esrb: data[0],
			pegi: data[1],
			pegifi: data[2],
			pegipt: data[3],
			bbfc: data[4],
			cero: data[5],
			usk: data[6],
			oflcau: data[7],
			oflcnz: data[8],
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
