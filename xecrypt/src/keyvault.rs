use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use std::io::Cursor;
use std::io::Read;
use std::ops::Range;
use thiserror::Error;

pub const KEYVAULT_SIZE_FULL: usize = 0x4000;
pub const KEYVAULT_SIZE_TRUNCATED: usize = 0x3FF0;
const DATA_START: usize = 0x18;

mod offsets {
	use std::ops::Range;

	pub const HMAC_SHA_HASH: Range<usize> = 0x00..0x10;
	pub const CONFOUNDER: Range<usize> = 0x10..0x20;

	pub const CONSOLE_SERIAL: Range<usize> = 0x98..0xA4;
	pub const MOBO_SERIAL: Range<usize> = 0xA4..0xB0;
	pub const GAME_REGION: Range<usize> = 0xB0..0xB2;

	pub const CONSOLE_OBFUSCATION_KEY: Range<usize> = 0xB8..0xC8;
	pub const KEY_OBFUSCATION_KEY: Range<usize> = 0xC8..0xD8;
	pub const ROAMABLE_OBFUSCATION_KEY: Range<usize> = 0xD8..0xE8;
	pub const DVD_KEY: Range<usize> = 0xE8..0xF8;
	pub const PRIMARY_ACTIVATION_KEY: Range<usize> = 0xF8..0x110;
	pub const SECONDARY_ACTIVATION_KEY: Range<usize> = 0x110..0x120;

	pub const GLOBAL_DEVICE_2DES_KEY1: Range<usize> = 0x120..0x130;
	pub const GLOBAL_DEVICE_2DES_KEY2: Range<usize> = 0x130..0x140;
	pub const WIRELESS_CONTROLLER_MS_2DES_KEY1: Range<usize> = 0x140..0x150;
	pub const WIRELESS_CONTROLLER_MS_2DES_KEY2: Range<usize> = 0x150..0x160;
	pub const WIRED_WEBCAM_MS_2DES_KEY: Range<usize> = 0x160..0x170;
	pub const WIRED_CONTROLLER_MS_2DES_KEY: Range<usize> = 0x170..0x180;
	pub const MEMORY_UNIT_MS_2DES_KEY: Range<usize> = 0x180..0x190;
	pub const OTHER_XSM3_DEVICE_MS_2DES_KEY: Range<usize> = 0x190..0x1A0;
	pub const WIRELESS_CONTROLLER_2DES_KEY1: Range<usize> = 0x1A0..0x1B0;
	pub const WIRELESS_CONTROLLER_2DES_KEY2: Range<usize> = 0x1B0..0x1C0;
	pub const WIRED_WEBCAM_2DES_KEY: Range<usize> = 0x1C0..0x1D0;
	pub const WIRED_CONTROLLER_2DES_KEY: Range<usize> = 0x1D0..0x1E0;
	pub const MEMORY_UNIT_2DES_KEY: Range<usize> = 0x1E0..0x1F0;
	pub const OTHER_XSM3_DEVICE_2DES_KEY: Range<usize> = 0x1F0..0x200;

	pub const CONSOLE_PRIVATE_KEY: Range<usize> = 0x0200..0x03D0;
	pub const XEIKA_PRIVATE_KEY: Range<usize> = 0x03D0..0x0760;
	pub const CARDEA_PRIVATE_KEY: Range<usize> = 0x0760..0x0930;

	pub const CONSOLE_CERTIFICATE: usize = 0x09B0;
	pub const CERT_CONSOLE_ID: Range<usize> = 0x02..0x07;
	pub const CERT_PART_NUMBER: Range<usize> = 0x07..0x12;
	pub const CERT_PRIVILEGES: Range<usize> = 0x12..0x16;
	pub const CERT_CONSOLE_TYPE: Range<usize> = 0x16..0x1A;
	pub const CERT_MFG_DATE: Range<usize> = 0x1E..0x26;

	pub const XEIKA_CERTIFICATE: Range<usize> = 0x0B58..0x1EE0;
	pub const CARDEA_CERTIFICATE: Range<usize> = 0x1EE0..0x3FE8;
}

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
		revision_from_part_number(&self.console_part_number)
	}
}

impl KeyVault {
	pub fn parse(data: &[u8]) -> Result<Self, KeyVaultError> {
		if data.len() != KEYVAULT_SIZE_FULL && data.len() != KEYVAULT_SIZE_TRUNCATED {
			return Err(KeyVaultError::InvalidSize { expected: KEYVAULT_SIZE_FULL, got: data.len() });
		}

		let header = KeyVaultHeader::parse(data)?;
		let kv = &data[DATA_START..];
		let config = KeyVaultConfig::parse(kv)?;
		let keys = KeyVaultKeys::parse(kv)?;
		let console_certificate = ConsoleCertificate::parse(&kv[offsets::CONSOLE_CERTIFICATE..])?;

		let xeika_end = std::cmp::min(offsets::XEIKA_CERTIFICATE.end, kv.len());
		let xeika_certificate = kv[offsets::XEIKA_CERTIFICATE.start..xeika_end].to_vec();

		let cardea_end = std::cmp::min(offsets::CARDEA_CERTIFICATE.end, kv.len());
		let cardea_certificate = kv[offsets::CARDEA_CERTIFICATE.start..cardea_end].to_vec();

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

/// Zerocopy keyvault view that borrows the underlying data.
///
/// Validates and parses the structure upfront, storing parsed scalars
/// and validated sub-structure references. Variable-length and large
/// fields (keys, certificates) are borrowed from the original buffer.
#[derive(Debug)]
pub struct KeyVaultRef<'a> {
	pub hmac_sha_hash: &'a [u8; 0x10],
	pub confounder: &'a [u8; 0x10],
	pub config: KeyVaultConfig,
	pub keys: KeyVaultKeysRef<'a>,
	pub console_certificate: ConsoleCertificateRef<'a>,
	pub xeika_certificate: &'a [u8],
	pub cardea_certificate: &'a [u8],
}

#[derive(Debug)]
pub struct KeyVaultKeysRef<'a> {
	pub console_serial_number: &'a str,
	pub mobo_serial_number: &'a [u8; 0x0C],
	pub game_region: u16,
	pub console_obfuscation_key: &'a [u8; 0x10],
	pub key_obfuscation_key: &'a [u8; 0x10],
	pub roamable_obfuscation_key: &'a [u8; 0x10],
	pub dvd_key: &'a [u8; 0x10],
	pub primary_activation_key: &'a [u8; 0x18],
	pub secondary_activation_key: &'a [u8; 0x10],
	pub global_device_2des_key1: &'a [u8; 0x10],
	pub global_device_2des_key2: &'a [u8; 0x10],
	pub wireless_controller_ms_2des_key1: &'a [u8; 0x10],
	pub wireless_controller_ms_2des_key2: &'a [u8; 0x10],
	pub wired_webcam_ms_2des_key: &'a [u8; 0x10],
	pub wired_controller_ms_2des_key: &'a [u8; 0x10],
	pub memory_unit_ms_2des_key: &'a [u8; 0x10],
	pub other_xsm3_device_ms_2des_key: &'a [u8; 0x10],
	pub wireless_controller_2des_key1: &'a [u8; 0x10],
	pub wireless_controller_2des_key2: &'a [u8; 0x10],
	pub wired_webcam_2des_key: &'a [u8; 0x10],
	pub wired_controller_2des_key: &'a [u8; 0x10],
	pub memory_unit_2des_key: &'a [u8; 0x10],
	pub other_xsm3_device_2des_key: &'a [u8; 0x10],
	pub console_private_key: &'a [u8],
	pub xeika_private_key: &'a [u8],
	pub cardea_private_key: &'a [u8],
}

#[derive(Debug)]
pub struct ConsoleCertificateRef<'a> {
	pub cert_size: u16,
	pub console_id: xenon_types::ConsoleId,
	pub console_part_number: &'a str,
	pub privileges: u32,
	pub console_type: ConsoleType,
	pub manufacturing_date: &'a str,
}

impl<'a> KeyVaultRef<'a> {
	pub fn parse(data: &'a [u8]) -> Result<Self, KeyVaultError> {
		if data.len() != KEYVAULT_SIZE_FULL && data.len() != KEYVAULT_SIZE_TRUNCATED {
			return Err(KeyVaultError::InvalidSize { expected: KEYVAULT_SIZE_FULL, got: data.len() });
		}

		let kv = &data[DATA_START..];

		let config = KeyVaultConfig::parse(kv)?;

		let serial_raw = &kv[offsets::CONSOLE_SERIAL];
		let serial_end = serial_raw.iter().position(|b| *b == 0).unwrap_or(serial_raw.len());
		let console_serial_number = std::str::from_utf8(&serial_raw[..serial_end])
			.map_err(|_| KeyVaultError::InvalidSize { expected: KEYVAULT_SIZE_FULL, got: data.len() })?;

		let keys = KeyVaultKeysRef {
			console_serial_number,
			mobo_serial_number: kv[offsets::MOBO_SERIAL].try_into().unwrap(),
			game_region: u16::from_be_bytes(kv[offsets::GAME_REGION].try_into().unwrap()),
			console_obfuscation_key: kv[offsets::CONSOLE_OBFUSCATION_KEY].try_into().unwrap(),
			key_obfuscation_key: kv[offsets::KEY_OBFUSCATION_KEY].try_into().unwrap(),
			roamable_obfuscation_key: kv[offsets::ROAMABLE_OBFUSCATION_KEY].try_into().unwrap(),
			dvd_key: kv[offsets::DVD_KEY].try_into().unwrap(),
			primary_activation_key: kv[offsets::PRIMARY_ACTIVATION_KEY].try_into().unwrap(),
			secondary_activation_key: kv[offsets::SECONDARY_ACTIVATION_KEY].try_into().unwrap(),
			global_device_2des_key1: kv[offsets::GLOBAL_DEVICE_2DES_KEY1].try_into().unwrap(),
			global_device_2des_key2: kv[offsets::GLOBAL_DEVICE_2DES_KEY2].try_into().unwrap(),
			wireless_controller_ms_2des_key1: kv[offsets::WIRELESS_CONTROLLER_MS_2DES_KEY1].try_into().unwrap(),
			wireless_controller_ms_2des_key2: kv[offsets::WIRELESS_CONTROLLER_MS_2DES_KEY2].try_into().unwrap(),
			wired_webcam_ms_2des_key: kv[offsets::WIRED_WEBCAM_MS_2DES_KEY].try_into().unwrap(),
			wired_controller_ms_2des_key: kv[offsets::WIRED_CONTROLLER_MS_2DES_KEY].try_into().unwrap(),
			memory_unit_ms_2des_key: kv[offsets::MEMORY_UNIT_MS_2DES_KEY].try_into().unwrap(),
			other_xsm3_device_ms_2des_key: kv[offsets::OTHER_XSM3_DEVICE_MS_2DES_KEY].try_into().unwrap(),
			wireless_controller_2des_key1: kv[offsets::WIRELESS_CONTROLLER_2DES_KEY1].try_into().unwrap(),
			wireless_controller_2des_key2: kv[offsets::WIRELESS_CONTROLLER_2DES_KEY2].try_into().unwrap(),
			wired_webcam_2des_key: kv[offsets::WIRED_WEBCAM_2DES_KEY].try_into().unwrap(),
			wired_controller_2des_key: kv[offsets::WIRED_CONTROLLER_2DES_KEY].try_into().unwrap(),
			memory_unit_2des_key: kv[offsets::MEMORY_UNIT_2DES_KEY].try_into().unwrap(),
			other_xsm3_device_2des_key: kv[offsets::OTHER_XSM3_DEVICE_2DES_KEY].try_into().unwrap(),
			console_private_key: &kv[offsets::CONSOLE_PRIVATE_KEY],
			xeika_private_key: &kv[offsets::XEIKA_PRIVATE_KEY],
			cardea_private_key: &kv[offsets::CARDEA_PRIVATE_KEY],
		};

		let cert = &kv[offsets::CONSOLE_CERTIFICATE..];
		let cert_size = u16::from_be_bytes([cert[0], cert[1]]);
		let pn_raw = &cert[offsets::CERT_PART_NUMBER];
		let pn_end = pn_raw.iter().position(|b| *b == 0).unwrap_or(pn_raw.len());
		let date_raw = &cert[offsets::CERT_MFG_DATE];
		let date_end = date_raw.iter().position(|b| *b == 0).unwrap_or(date_raw.len());

		let console_certificate = ConsoleCertificateRef {
			cert_size,
			console_id: xenon_types::ConsoleId(cert[offsets::CERT_CONSOLE_ID].try_into().unwrap()),
			console_part_number: std::str::from_utf8(&pn_raw[..pn_end]).unwrap_or(""),
			privileges: u32::from_be_bytes(cert[offsets::CERT_PRIVILEGES].try_into().unwrap()),
			console_type: ConsoleType(u32::from_be_bytes(cert[offsets::CERT_CONSOLE_TYPE].try_into().unwrap())),
			manufacturing_date: std::str::from_utf8(&date_raw[..date_end]).unwrap_or(""),
		};

		let xeika_end = std::cmp::min(offsets::XEIKA_CERTIFICATE.end, kv.len());
		let cardea_end = std::cmp::min(offsets::CARDEA_CERTIFICATE.end, kv.len());

		Ok(KeyVaultRef {
			hmac_sha_hash: data[offsets::HMAC_SHA_HASH].try_into().unwrap(),
			confounder: data[offsets::CONFOUNDER].try_into().unwrap(),
			config,
			keys,
			console_certificate,
			xeika_certificate: &kv[offsets::XEIKA_CERTIFICATE.start..xeika_end],
			cardea_certificate: &kv[offsets::CARDEA_CERTIFICATE.start..cardea_end],
		})
	}

	pub fn into_owned(self) -> KeyVault {
		KeyVault {
			header: KeyVaultHeader { hmac_sha_hash: *self.hmac_sha_hash, confounder: *self.confounder },
			config: self.config,
			keys: KeyVaultKeys {
				console_serial_number: self.keys.console_serial_number.to_owned(),
				mobo_serial_number: *self.keys.mobo_serial_number,
				game_region: self.keys.game_region,
				console_obfuscation_key: *self.keys.console_obfuscation_key,
				key_obfuscation_key: *self.keys.key_obfuscation_key,
				roamable_obfuscation_key: *self.keys.roamable_obfuscation_key,
				dvd_key: *self.keys.dvd_key,
				primary_activation_key: *self.keys.primary_activation_key,
				secondary_activation_key: *self.keys.secondary_activation_key,
				global_device_2des_key1: *self.keys.global_device_2des_key1,
				global_device_2des_key2: *self.keys.global_device_2des_key2,
				wireless_controller_ms_2des_key1: *self.keys.wireless_controller_ms_2des_key1,
				wireless_controller_ms_2des_key2: *self.keys.wireless_controller_ms_2des_key2,
				wired_webcam_ms_2des_key: *self.keys.wired_webcam_ms_2des_key,
				wired_controller_ms_2des_key: *self.keys.wired_controller_ms_2des_key,
				memory_unit_ms_2des_key: *self.keys.memory_unit_ms_2des_key,
				other_xsm3_device_ms_2des_key: *self.keys.other_xsm3_device_ms_2des_key,
				wireless_controller_2des_key1: *self.keys.wireless_controller_2des_key1,
				wireless_controller_2des_key2: *self.keys.wireless_controller_2des_key2,
				wired_webcam_2des_key: *self.keys.wired_webcam_2des_key,
				wired_controller_2des_key: *self.keys.wired_controller_2des_key,
				memory_unit_2des_key: *self.keys.memory_unit_2des_key,
				other_xsm3_device_2des_key: *self.keys.other_xsm3_device_2des_key,
				console_private_key: self.keys.console_private_key.to_vec(),
				xeika_private_key: self.keys.xeika_private_key.to_vec(),
				cardea_private_key: self.keys.cardea_private_key.to_vec(),
			},
			console_certificate: ConsoleCertificate {
				cert_size: self.console_certificate.cert_size,
				console_id: self.console_certificate.console_id,
				console_part_number: self.console_certificate.console_part_number.to_owned(),
				privileges: self.console_certificate.privileges,
				console_type: self.console_certificate.console_type,
				manufacturing_date: self.console_certificate.manufacturing_date.to_owned(),
			},
			xeika_certificate: self.xeika_certificate.to_vec(),
			cardea_certificate: self.cardea_certificate.to_vec(),
		}
	}

	pub fn console_id(&self) -> &xenon_types::ConsoleId {
		&self.console_certificate.console_id
	}

	pub fn console_serial(&self) -> &str {
		self.keys.console_serial_number
	}

	pub fn dvd_key(&self) -> &[u8; 0x10] {
		self.keys.dvd_key
	}

	pub fn game_region(&self) -> u16 {
		self.keys.game_region
	}

	pub fn console_type(&self) -> ConsoleType {
		self.console_certificate.console_type
	}

	pub fn is_devkit(&self) -> bool {
		self.console_certificate.console_type.is_devkit()
	}

	pub fn is_retail(&self) -> bool {
		self.console_certificate.console_type.is_retail()
	}

	pub fn revision(&self) -> ConsoleRevision {
		revision_from_part_number(self.console_certificate.console_part_number)
	}
}

impl<'a> ConsoleCertificateRef<'a> {
	pub fn revision(&self) -> ConsoleRevision {
		revision_from_part_number(self.console_part_number)
	}
}

fn revision_from_part_number(pn: &str) -> ConsoleRevision {
	if pn.starts_with("X803") || pn.starts_with("X800") {
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
	if pn.starts_with("004") {
		return ConsoleRevision::Xenon;
	}
	ConsoleRevision::Unknown
}

impl KeyVaultHeader {
	fn parse(data: &[u8]) -> Result<Self, KeyVaultError> {
		let mut hmac_sha_hash = [0u8; 0x10];
		hmac_sha_hash.copy_from_slice(&data[offsets::HMAC_SHA_HASH]);
		let mut confounder = [0u8; 0x10];
		confounder.copy_from_slice(&data[offsets::CONFOUNDER]);
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
		fn read_range<const N: usize>(data: &[u8], range: Range<usize>) -> [u8; N] {
			let mut buf = [0u8; N];
			buf.copy_from_slice(&data[range]);
			buf
		}

		let serial_raw = &data[offsets::CONSOLE_SERIAL];
		let serial_end = serial_raw.iter().position(|b| *b == 0).unwrap_or(serial_raw.len());
		let console_serial_number = String::from_utf8_lossy(&serial_raw[..serial_end]).into_owned();

		let game_region = u16::from_be_bytes(data[offsets::GAME_REGION].try_into().unwrap());

		Ok(KeyVaultKeys {
			console_serial_number,
			mobo_serial_number: read_range(data, offsets::MOBO_SERIAL),
			game_region,
			console_obfuscation_key: read_range(data, offsets::CONSOLE_OBFUSCATION_KEY),
			key_obfuscation_key: read_range(data, offsets::KEY_OBFUSCATION_KEY),
			roamable_obfuscation_key: read_range(data, offsets::ROAMABLE_OBFUSCATION_KEY),
			dvd_key: read_range(data, offsets::DVD_KEY),
			primary_activation_key: read_range(data, offsets::PRIMARY_ACTIVATION_KEY),
			secondary_activation_key: read_range(data, offsets::SECONDARY_ACTIVATION_KEY),
			global_device_2des_key1: read_range(data, offsets::GLOBAL_DEVICE_2DES_KEY1),
			global_device_2des_key2: read_range(data, offsets::GLOBAL_DEVICE_2DES_KEY2),
			wireless_controller_ms_2des_key1: read_range(data, offsets::WIRELESS_CONTROLLER_MS_2DES_KEY1),
			wireless_controller_ms_2des_key2: read_range(data, offsets::WIRELESS_CONTROLLER_MS_2DES_KEY2),
			wired_webcam_ms_2des_key: read_range(data, offsets::WIRED_WEBCAM_MS_2DES_KEY),
			wired_controller_ms_2des_key: read_range(data, offsets::WIRED_CONTROLLER_MS_2DES_KEY),
			memory_unit_ms_2des_key: read_range(data, offsets::MEMORY_UNIT_MS_2DES_KEY),
			other_xsm3_device_ms_2des_key: read_range(data, offsets::OTHER_XSM3_DEVICE_MS_2DES_KEY),
			wireless_controller_2des_key1: read_range(data, offsets::WIRELESS_CONTROLLER_2DES_KEY1),
			wireless_controller_2des_key2: read_range(data, offsets::WIRELESS_CONTROLLER_2DES_KEY2),
			wired_webcam_2des_key: read_range(data, offsets::WIRED_WEBCAM_2DES_KEY),
			wired_controller_2des_key: read_range(data, offsets::WIRED_CONTROLLER_2DES_KEY),
			memory_unit_2des_key: read_range(data, offsets::MEMORY_UNIT_2DES_KEY),
			other_xsm3_device_2des_key: read_range(data, offsets::OTHER_XSM3_DEVICE_2DES_KEY),
			console_private_key: data[offsets::CONSOLE_PRIVATE_KEY].to_vec(),
			xeika_private_key: data[offsets::XEIKA_PRIVATE_KEY].to_vec(),
			cardea_private_key: data[offsets::CARDEA_PRIVATE_KEY].to_vec(),
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
