use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use serde::Serialize;
use std::io::Cursor;
use std::io::Read;

use crate::header::OptionalHeaderKey;
use crate::header::Xex2Header;

#[derive(Debug, Serialize)]
pub struct TlsInfo {
	pub slot_count: u32,
	pub raw_data_address: u32,
	pub data_size: u32,
	pub raw_data_size: u32,
}

#[derive(Debug, Serialize)]
pub struct ResourceInfo {
	pub resources: Vec<ResourceEntry>,
}

#[derive(Debug, Serialize)]
pub struct ResourceEntry {
	pub name: String,
	pub address: u32,
	pub size: u32,
}

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct SystemFlags(pub u32);

impl SystemFlags {
	pub fn no_forced_reboot(&self) -> bool {
		self.0 & 0x01 != 0
	}
	pub fn foreground_tasks(&self) -> bool {
		self.0 & 0x02 != 0
	}
	pub fn no_odd_mapping(&self) -> bool {
		self.0 & 0x04 != 0
	}
	pub fn handles_gamepad_disconnect(&self) -> bool {
		self.0 & 0x08 != 0
	}
	pub fn insecure_sockets(&self) -> bool {
		self.0 & 0x40 != 0
	}
	pub fn xbox1_interoperability(&self) -> bool {
		self.0 & 0x80 != 0
	}
	pub fn dash_context(&self) -> bool {
		self.0 & 0x100 != 0
	}
	pub fn uses_game_voice_channel(&self) -> bool {
		self.0 & 0x200 != 0
	}
	pub fn pal50_incompatible(&self) -> bool {
		self.0 & 0x1000 != 0
	}
	pub fn insecure_utility_drive(&self) -> bool {
		self.0 & 0x2000 != 0
	}
	pub fn xam_hooks(&self) -> bool {
		self.0 & 0x4000 != 0
	}
	pub fn access_pip(&self) -> bool {
		self.0 & 0x8000 != 0
	}
	pub fn prefer_big_button_input(&self) -> bool {
		self.0 & 0x100000 != 0
	}
	pub fn allow_controller_swapping(&self) -> bool {
		self.0 & 0x2000000 != 0
	}
	pub fn allow_kinect(&self) -> bool {
		self.0 & 0x4000000 != 0
	}
	pub fn allow_kinect_unless_bound(&self) -> bool {
		self.0 & 0x8000000 != 0
	}
}

#[derive(Debug, Serialize)]
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
