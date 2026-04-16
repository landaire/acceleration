use xex2::Xex2;
use xex2::header::CompressionType;

fn load_xex(name: &str) -> Xex2 {
	let path = format!("../xex_files/{}", name);
	let data = std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {}", path, e));
	Xex2::parse(data).unwrap_or_else(|e| panic!("failed to parse {}: {}", name, e))
}

#[test]
fn parse_devkit_basic() {
	let xex = load_xex("afplayer.xex");
	assert_eq!(xex.header.module_flags.bits(), 0x09);
	assert_eq!(xex.security_info.image_info.load_address, xex2::header::VirtualAddress(0x9ef30000));
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
fn ratings_are_ordered_by_age() {
	use xex2::opt::CeroRating;
	use xex2::opt::EsrbRating;
	use xex2::opt::PegiRating;
	use xex2::opt::Rating;
	use xex2::opt::UskRating;

	// Within a rating board, higher = older
	assert!(EsrbRating::EC < EsrbRating::E);
	assert!(EsrbRating::E < EsrbRating::E10);
	assert!(EsrbRating::E10 < EsrbRating::T);
	assert!(EsrbRating::T < EsrbRating::M);
	assert!(PegiRating::Three < PegiRating::Eighteen);
	assert!(CeroRating::A < CeroRating::Z);
	assert!(UskRating::Zero < UskRating::Eighteen);

	// Rating<T> ordering: Rated < Unknown < Unrated
	let rated_low = Rating::Rated(EsrbRating::EC);
	let rated_high = Rating::Rated(EsrbRating::M);
	let unknown: Rating<EsrbRating> = Rating::Unknown(0x99);
	let unrated: Rating<EsrbRating> = Rating::Unrated;

	assert!(rated_low < rated_high);
	assert!(rated_high < unknown);
	assert!(unknown < unrated);

	// Cross-check with actual game data
	let portal = load_xex("Portal 2.xex");
	let deus_ex = load_xex("Deus Ex.xex");
	let p_ratings = portal.header.game_ratings().unwrap();
	let d_ratings = deus_ex.header.game_ratings().unwrap();
	// Portal 2 (E10+) is rated lower than Deus Ex HR (M)
	assert!(p_ratings.esrb < d_ratings.esrb);
	// Portal 2 PEGI 12+ < Deus Ex HR PEGI 18+
	assert!(p_ratings.pegi < d_ratings.pegi);
}

#[test]
fn security_info_file_key_not_all_zeros_for_encrypted() {
	let xex = load_xex("AntiPiracyUI.xex");
	assert_ne!(xex.security_info.image_info.file_key, xex2::header::AesKey([0u8; 16]));
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

#[test]
fn patch_resign_verifies_with_devkit_key() {
	let xex = load_xex("haloreach-powerhouse.xex");
	let mut limits = xex2::writer::RemoveLimits::default();
	limits.region = true;
	limits.media = true;

	let patched_data = xex.modify(&limits).unwrap();

	let sec_off = u32::from_be_bytes(patched_data[0x10..0x14].try_into().unwrap()) as usize;
	let info_size_off = sec_off + 0x108;
	let info_size = u32::from_be_bytes(patched_data[info_size_off..info_size_off + 4].try_into().unwrap()) as usize;
	let image_info_len = info_size - 0x100;
	let image_info = &patched_data[info_size_off..info_size_off + image_info_len];

	let digest = xecrypt::symmetric::xe_crypt_rot_sum_sha(image_info, &[]);
	let sig = &patched_data[sec_off + 0x08..sec_off + 0x108];

	xecrypt::RsaKeyKind::Pirs
		.verify_signature(xecrypt::ConsoleKind::Devkit, sig, &digest)
		.expect("devkit PIRS signature should verify after re-signing");

	assert_eq!(u32::from_be_bytes(patched_data[sec_off + 0x174..sec_off + 0x178].try_into().unwrap()), 0xFFFFFFFF,);
	assert_eq!(u32::from_be_bytes(patched_data[sec_off + 0x178..sec_off + 0x17C].try_into().unwrap()), 0xFFFFFFFF,);
}
