use xex2::header::CompressionType;
use xex2::Xex2;

fn load_xex(name: &str) -> Xex2 {
	let path = format!("../xex_files/{}", name);
	let data = std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {}", path, e));
	Xex2::parse(data).unwrap_or_else(|e| panic!("failed to parse {}: {}", name, e))
}

#[test]
fn parse_devkit_basic() {
	let xex = load_xex("afplayer.xex");
	assert_eq!(xex.header.module_flags.0, 0x09);
	assert_eq!(xex.security_info.image_info.load_address, 0x9ef30000);
	assert!(xex.header.entry_point().is_some());

	let fmt = xex.header.file_format_info(xex.raw()).unwrap();
	assert_eq!(fmt.encryption_type, xex2::header::EncryptionType::None);
	assert_eq!(fmt.compression_type, CompressionType::Basic);
}

#[test]
fn parse_encrypted_basic() {
	let xex = load_xex("AntiPiracyUI.xex");
	let fmt = xex.header.file_format_info(xex.raw()).unwrap();
	assert_eq!(fmt.encryption_type, xex2::header::EncryptionType::Normal);
	assert_eq!(fmt.compression_type, CompressionType::Basic);
}

#[test]
fn extract_devkit_basic_produces_pe() {
	let xex = load_xex("afplayer.xex");
	let basefile = xex.extract_basefile().unwrap();
	assert_eq!(&basefile[0..2], b"MZ");
	assert_eq!(basefile.len(), xex.security_info.image_size as usize);
}

#[test]
fn extract_encrypted_basic_produces_pe() {
	let xex = load_xex("AntiPiracyUI.xex");
	let basefile = xex.extract_basefile().unwrap();
	assert_eq!(&basefile[0..2], b"MZ");
}

#[test]
fn extract_multiple_encrypted_basic() {
	for name in &["Portal 2.xex", "xlaunch.xex", "HvxDump.xex"] {
		let xex = load_xex(name);
		let fmt = xex.header.file_format_info(xex.raw()).unwrap();
		if fmt.compression_type == CompressionType::Basic {
			let basefile = xex.extract_basefile().unwrap();
			assert_eq!(&basefile[0..2], b"MZ", "failed for {}", name);
		}
	}
}

#[test]
fn execution_info_parsed() {
	let xex = load_xex("afplayer.xex");
	let exec = xex.header.execution_info();
	assert!(exec.is_some());
}

#[test]
fn security_info_file_key_not_all_zeros_for_encrypted() {
	let xex = load_xex("AntiPiracyUI.xex");
	assert_ne!(xex.security_info.image_info.file_key, [0u8; 16]);
}

#[test]
fn extract_unencrypted_normal_produces_pe() {
	let xex = load_xex("xshell twi.xex");
	let fmt = xex.header.file_format_info(xex.raw()).unwrap();
	assert_eq!(fmt.encryption_type, xex2::header::EncryptionType::None);
	assert_eq!(fmt.compression_type, CompressionType::Normal);
	let basefile = xex.extract_basefile().unwrap();
	assert_eq!(&basefile[0..2], b"MZ");
}

#[test]
fn extract_encrypted_normal_produces_pe() {
	let xex = load_xex("ArchEngine.xex");
	let fmt = xex.header.file_format_info(xex.raw()).unwrap();
	assert_eq!(fmt.encryption_type, xex2::header::EncryptionType::Normal);
	assert_eq!(fmt.compression_type, CompressionType::Normal);
	let basefile = xex.extract_basefile().unwrap();
	assert_eq!(&basefile[0..2], b"MZ");
}

#[test]
fn extract_large_window_normal() {
	let xex = load_xex("xshell - Copy.xex");
	let fmt = xex.header.file_format_info(xex.raw()).unwrap();
	assert_eq!(fmt.compression_type, CompressionType::Normal);
	assert_eq!(fmt.window_size, Some(0x100000));
	let basefile = xex.extract_basefile().unwrap();
	assert_eq!(&basefile[0..2], b"MZ");
}

#[test]
fn extract_multiple_normal_compression() {
	for name in &["xbdm.xex", "mfgbootlauncher.xex", "BBNeo!_0424.xex"] {
		let xex = load_xex(name);
		let fmt = xex.header.file_format_info(xex.raw()).unwrap();
		if fmt.compression_type == CompressionType::Normal {
			let basefile = xex.extract_basefile().unwrap();
			assert_eq!(&basefile[0..2], b"MZ", "failed for {}", name);
		}
	}
}
