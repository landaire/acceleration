mod serde_hex;

use bitflags::bitflags;
use serde::Deserialize;
use serde::Serialize;
use serde::Serializer;

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TitleId(pub u32);

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MediaId(pub u32);

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SavegameId(pub u32);

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConsoleId(#[serde(with = "serde_hex::fixed5")] pub [u8; 5]);

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileId(#[serde(with = "serde_hex::fixed8")] pub [u8; 8]);

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId(#[serde(with = "serde_hex::fixed20")] pub [u8; 0x14]);

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VirtualAddress(pub u32);

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AesKey(pub [u8; 16]);

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
