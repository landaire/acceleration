use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use std::io::Cursor;
use std::io::Read;
use thiserror::Error;

pub const KEYVAULT_SIZE_FULL: usize = 0x4000;
pub const KEYVAULT_SIZE_TRUNCATED: usize = 0x3FF0;
const DATA_START: usize = 0x18;

#[derive(Error, Debug)]
pub enum KeyVaultError {
	#[error("invalid keyvault size: expected {expected}, got {got}")]
	InvalidSize { expected: usize, got: usize },
	#[error("I/O error: {0}")]
	Io(#[from] std::io::Error),
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct KeyVault {
	pub header: KeyVaultHeader,
	pub config: KeyVaultConfig,
	pub keys: KeyVaultKeys,
	pub console_certificate: ConsoleCertificate,
	pub xeika_certificate: Vec<u8>,
	pub cardea_certificate: Vec<u8>,
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct KeyVaultHeader {
	#[cfg_attr(feature = "serde", serde(skip))]
	pub hmac_sha_hash: [u8; 0x10],
	#[cfg_attr(feature = "serde", serde(skip))]
	pub confounder: [u8; 0x10],
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct KeyVaultConfig {
	pub manufacturing_mode: u8,
	pub alternate_key_vault: u8,
	pub restricted_privileges_flags: u8,
	pub reserved_byte4: u8,
	pub odd_features: u16,
	pub odd_authtype: u16,
	pub restricted_hvext_loader: u32,
	pub policy_flash_size: u32,
	pub policy_builtin_usbmu_size: u32,
	pub reserved_dword4: u32,
	pub restricted_privileges: u64,
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_qword2: u64,
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_qword3: u64,
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_qword4: u64,
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_key1: [u8; 0x10],
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_key2: [u8; 0x10],
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_key3: [u8; 0x10],
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_key4: [u8; 0x10],
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_random_key1: [u8; 0x10],
	#[cfg_attr(feature = "serde", serde(skip))]
	pub reserved_random_key2: [u8; 0x10],
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct KeyVaultKeys {
	pub console_serial_number: String,
	pub mobo_serial_number: [u8; 0x0C],
	pub game_region: u16,
	pub console_obfuscation_key: [u8; 0x10],
	pub key_obfuscation_key: [u8; 0x10],
	pub roamable_obfuscation_key: [u8; 0x10],
	pub dvd_key: [u8; 0x10],
	pub primary_activation_key: [u8; 0x18],
	pub secondary_activation_key: [u8; 0x10],
	pub global_device_2des_key1: [u8; 0x10],
	pub global_device_2des_key2: [u8; 0x10],
	pub wireless_controller_ms_2des_key1: [u8; 0x10],
	pub wireless_controller_ms_2des_key2: [u8; 0x10],
	pub wired_webcam_ms_2des_key: [u8; 0x10],
	pub wired_controller_ms_2des_key: [u8; 0x10],
	pub memory_unit_ms_2des_key: [u8; 0x10],
	pub other_xsm3_device_ms_2des_key: [u8; 0x10],
	pub wireless_controller_2des_key1: [u8; 0x10],
	pub wireless_controller_2des_key2: [u8; 0x10],
	pub wired_webcam_2des_key: [u8; 0x10],
	pub wired_controller_2des_key: [u8; 0x10],
	pub memory_unit_2des_key: [u8; 0x10],
	pub other_xsm3_device_2des_key: [u8; 0x10],
	pub console_private_key: Vec<u8>,
	pub xeika_private_key: Vec<u8>,
	pub cardea_private_key: Vec<u8>,
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ConsoleCertificate {
	pub cert_size: u16,
	pub console_id: xenon_types::ConsoleId,
	pub console_part_number: String,
	pub privileges: u32,
	pub console_type: ConsoleType,
	pub manufacturing_date: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ConsoleType(pub u32);

impl ConsoleType {
	pub fn is_devkit(&self) -> bool {
		self.0 & 0x3 == 1
	}

	pub fn is_retail(&self) -> bool {
		self.0 & 0x3 == 2
	}

	pub fn is_testkit(&self) -> bool {
		self.0 & 0x40000000 != 0
	}

	pub fn is_recovery_generated(&self) -> bool {
		self.0 & 0x80000000 != 0
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ConsoleRevision {
	Xenon,
	Zephyr,
	Falcon,
	Jasper,
	Trinity,
	Corona,
	Winchester,
	Unknown,
}

impl ConsoleCertificate {
	pub fn revision(&self) -> ConsoleRevision {
		let pn = &self.console_part_number;
		if pn.starts_with("X803") || pn.starts_with("X800") {
			// Xenon/Zephyr era part numbers
			if pn.contains("955") || pn.contains("953") {
				return ConsoleRevision::Xenon;
			}
			if pn.contains("885") || pn.contains("878") {
				return ConsoleRevision::Zephyr;
			}
		}
		if pn.starts_with("X804") || pn.starts_with("X810") {
			return ConsoleRevision::Falcon;
		}
		if pn.starts_with("X811") || pn.starts_with("X812") || pn.starts_with("X815") {
			return ConsoleRevision::Jasper;
		}
		if pn.starts_with("X816") || pn.starts_with("X818") {
			return ConsoleRevision::Trinity;
		}
		if pn.starts_with("X819") || pn.starts_with("X820") || pn.starts_with("X850") {
			return ConsoleRevision::Corona;
		}
		if pn.starts_with("X851") || pn.starts_with("X852") || pn.starts_with("X86") {
			return ConsoleRevision::Winchester;
		}
		// Devkit part numbers use a different scheme
		if pn.starts_with("004") {
			return ConsoleRevision::Xenon;
		}
		ConsoleRevision::Unknown
	}
}

impl KeyVault {
	pub fn parse(data: &[u8]) -> Result<Self, KeyVaultError> {
		if data.len() != KEYVAULT_SIZE_FULL && data.len() != KEYVAULT_SIZE_TRUNCATED {
			return Err(KeyVaultError::InvalidSize { expected: KEYVAULT_SIZE_FULL, got: data.len() });
		}

		let header = KeyVaultHeader::parse(&data[0..0x20])?;
		let kv_data = &data[DATA_START..];
		let config = KeyVaultConfig::parse(kv_data)?;
		let keys = KeyVaultKeys::parse(kv_data)?;

		let cert_offset = 0x09B0;
		let console_certificate = ConsoleCertificate::parse(&kv_data[cert_offset..])?;

		let xeika_offset = 0x0B58;
		let xeika_size = 0x1388;
		let xeika_end = std::cmp::min(xeika_offset + xeika_size, kv_data.len());
		let xeika_certificate = kv_data[xeika_offset..xeika_end].to_vec();

		let cardea_offset = 0x1EE0;
		let cardea_size = 0x2108;
		let cardea_end = std::cmp::min(cardea_offset + cardea_size, kv_data.len());
		let cardea_certificate = kv_data[cardea_offset..cardea_end].to_vec();

		Ok(KeyVault { header, config, keys, console_certificate, xeika_certificate, cardea_certificate })
	}

	pub fn console_id(&self) -> &xenon_types::ConsoleId {
		&self.console_certificate.console_id
	}

	pub fn console_serial(&self) -> &str {
		&self.keys.console_serial_number
	}

	pub fn dvd_key(&self) -> &[u8; 0x10] {
		&self.keys.dvd_key
	}

	pub fn game_region(&self) -> u16 {
		self.keys.game_region
	}

	pub fn console_type(&self) -> &ConsoleType {
		&self.console_certificate.console_type
	}

	pub fn is_devkit(&self) -> bool {
		self.console_certificate.console_type.is_devkit()
	}

	pub fn is_retail(&self) -> bool {
		self.console_certificate.console_type.is_retail()
	}

	pub fn revision(&self) -> ConsoleRevision {
		self.console_certificate.revision()
	}
}

impl KeyVaultHeader {
	fn parse(data: &[u8]) -> Result<Self, KeyVaultError> {
		let mut hmac_sha_hash = [0u8; 0x10];
		hmac_sha_hash.copy_from_slice(&data[0..0x10]);
		let mut confounder = [0u8; 0x10];
		confounder.copy_from_slice(&data[0x10..0x20]);
		Ok(KeyVaultHeader { hmac_sha_hash, confounder })
	}
}

impl KeyVaultConfig {
	fn parse(data: &[u8]) -> Result<Self, KeyVaultError> {
		let mut c = Cursor::new(data);
		let manufacturing_mode = c.read_u8()?;
		let alternate_key_vault = c.read_u8()?;
		let restricted_privileges_flags = c.read_u8()?;
		let reserved_byte4 = c.read_u8()?;
		let odd_features = c.read_u16::<BigEndian>()?;
		let odd_authtype = c.read_u16::<BigEndian>()?;
		let restricted_hvext_loader = c.read_u32::<BigEndian>()?;
		let policy_flash_size = c.read_u32::<BigEndian>()?;
		let policy_builtin_usbmu_size = c.read_u32::<BigEndian>()?;
		let reserved_dword4 = c.read_u32::<BigEndian>()?;
		let restricted_privileges = c.read_u64::<BigEndian>()?;

		fn read_qword(c: &mut Cursor<&[u8]>) -> Result<u64, std::io::Error> {
			c.read_u64::<BigEndian>()
		}
		fn read_key(c: &mut Cursor<&[u8]>) -> Result<[u8; 0x10], std::io::Error> {
			let mut buf = [0u8; 0x10];
			c.read_exact(&mut buf)?;
			Ok(buf)
		}

		Ok(KeyVaultConfig {
			manufacturing_mode,
			alternate_key_vault,
			restricted_privileges_flags,
			reserved_byte4,
			odd_features,
			odd_authtype,
			restricted_hvext_loader,
			policy_flash_size,
			policy_builtin_usbmu_size,
			reserved_dword4,
			restricted_privileges,
			reserved_qword2: read_qword(&mut c)?,
			reserved_qword3: read_qword(&mut c)?,
			reserved_qword4: read_qword(&mut c)?,
			reserved_key1: read_key(&mut c)?,
			reserved_key2: read_key(&mut c)?,
			reserved_key3: read_key(&mut c)?,
			reserved_key4: read_key(&mut c)?,
			reserved_random_key1: read_key(&mut c)?,
			reserved_random_key2: read_key(&mut c)?,
		})
	}
}

impl KeyVaultKeys {
	fn parse(data: &[u8]) -> Result<Self, KeyVaultError> {
		fn read_key<const N: usize>(data: &[u8], offset: usize) -> [u8; N] {
			let mut buf = [0u8; N];
			buf.copy_from_slice(&data[offset..offset + N]);
			buf
		}

		let serial_raw = &data[0x98..0xA4];
		let serial_end = serial_raw.iter().position(|b| *b == 0).unwrap_or(serial_raw.len());
		let console_serial_number = String::from_utf8_lossy(&serial_raw[..serial_end]).into_owned();

		let game_region = u16::from_be_bytes([data[0xB0], data[0xB1]]);

		Ok(KeyVaultKeys {
			console_serial_number,
			mobo_serial_number: read_key(data, 0xA4),
			game_region,
			console_obfuscation_key: read_key(data, 0xB8),
			key_obfuscation_key: read_key(data, 0xC8),
			roamable_obfuscation_key: read_key(data, 0xD8),
			dvd_key: read_key(data, 0xE8),
			primary_activation_key: read_key(data, 0xF8),
			secondary_activation_key: read_key(data, 0x110),
			global_device_2des_key1: read_key(data, 0x120),
			global_device_2des_key2: read_key(data, 0x130),
			wireless_controller_ms_2des_key1: read_key(data, 0x140),
			wireless_controller_ms_2des_key2: read_key(data, 0x150),
			wired_webcam_ms_2des_key: read_key(data, 0x160),
			wired_controller_ms_2des_key: read_key(data, 0x170),
			memory_unit_ms_2des_key: read_key(data, 0x180),
			other_xsm3_device_ms_2des_key: read_key(data, 0x190),
			wireless_controller_2des_key1: read_key(data, 0x1A0),
			wireless_controller_2des_key2: read_key(data, 0x1B0),
			wired_webcam_2des_key: read_key(data, 0x1C0),
			wired_controller_2des_key: read_key(data, 0x1D0),
			memory_unit_2des_key: read_key(data, 0x1E0),
			other_xsm3_device_2des_key: read_key(data, 0x1F0),
			console_private_key: data[0x0200..0x0200 + 0x01D0].to_vec(),
			xeika_private_key: data[0x03D0..0x03D0 + 0x0390].to_vec(),
			cardea_private_key: data[0x0760..0x0760 + 0x01D0].to_vec(),
		})
	}
}

impl ConsoleCertificate {
	fn parse(data: &[u8]) -> Result<Self, KeyVaultError> {
		let mut c = Cursor::new(data);
		let cert_size = c.read_u16::<BigEndian>()?;
		let mut console_id_raw = [0u8; 5];
		c.read_exact(&mut console_id_raw)?;
		let console_id = xenon_types::ConsoleId(console_id_raw);

		let mut part_number = [0u8; 0x0B];
		c.read_exact(&mut part_number)?;
		let pn_end = part_number.iter().position(|b| *b == 0).unwrap_or(part_number.len());
		let console_part_number = String::from_utf8_lossy(&part_number[..pn_end]).into_owned();

		let privileges = c.read_u32::<BigEndian>()?;
		let console_type = ConsoleType(c.read_u32::<BigEndian>()?);

		let mut date_raw = [0u8; 0x08];
		c.read_exact(&mut date_raw)?;
		let date_end = date_raw.iter().position(|b| *b == 0).unwrap_or(date_raw.len());
		let manufacturing_date = String::from_utf8_lossy(&date_raw[..date_end]).into_owned();

		Ok(ConsoleCertificate {
			cert_size,
			console_id,
			console_part_number,
			privileges,
			console_type,
			manufacturing_date,
		})
	}
}
