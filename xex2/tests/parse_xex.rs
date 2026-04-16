use xex2::Xex2;
use xex2::header::CompressionType;

fn load_xex(name: &str) -> (Vec<u8>, Xex2) {
	let path = format!("../xex_files/{}", name);
	let data = std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {}", path, e));
	let xex = Xex2::parse(&data).unwrap_or_else(|e| panic!("failed to parse {}: {}", name, e));
	(data, xex)
}

#[test]
fn parse_devkit_basic() {
	let (_data, xex) = load_xex("afplayer.xex");
	assert_eq!(xex.header.module_flags.bits(), 0x09);
	assert_eq!(xex.security_info.image_info.load_address, xex2::header::VirtualAddress(0x9ef30000));
	assert!(xex.header.entry_point().is_some());

	let fmt = xex.header.file_format_info().unwrap();
	assert_eq!(fmt.encryption_type, xex2::header::EncryptionType::None);
	assert_eq!(fmt.compression_type, CompressionType::Basic);
}

#[test]
fn parse_encrypted_basic() {
	let (_data, xex) = load_xex("AntiPiracyUI.xex");
	let fmt = xex.header.file_format_info().unwrap();
	assert_eq!(fmt.encryption_type, xex2::header::EncryptionType::Normal);
	assert_eq!(fmt.compression_type, CompressionType::Basic);
}

#[test]
fn extract_devkit_basic_produces_pe() {
	let (data, xex) = load_xex("afplayer.xex");
	let basefile = xex.extract_basefile(&data).unwrap();
	assert_eq!(&basefile[0..2], b"MZ");
	assert_eq!(basefile.len(), xex.security_info.image_size as usize);
}

#[test]
fn extract_encrypted_basic_produces_pe() {
	let (data, xex) = load_xex("AntiPiracyUI.xex");
	let basefile = xex.extract_basefile(&data).unwrap();
	assert_eq!(&basefile[0..2], b"MZ");
}

#[test]
fn extract_multiple_encrypted_basic() {
	for name in &["Portal 2.xex", "xlaunch.xex", "HvxDump.xex"] {
		let (data, xex) = load_xex(name);
		let fmt = xex.header.file_format_info().unwrap();
		if fmt.compression_type == CompressionType::Basic {
			let basefile = xex.extract_basefile(&data).unwrap();
			assert_eq!(&basefile[0..2], b"MZ", "failed for {}", name);
		}
	}
}

#[test]
fn execution_info_parsed() {
	let (_data, xex) = load_xex("afplayer.xex");
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
	let (_pd, portal) = load_xex("Portal 2.xex");
	let (_dd, deus_ex) = load_xex("Deus Ex.xex");
	let p_ratings = portal.header.game_ratings().unwrap();
	let d_ratings = deus_ex.header.game_ratings().unwrap();
	// Portal 2 (E10+) is rated lower than Deus Ex HR (M)
	assert!(p_ratings.esrb < d_ratings.esrb);
	// Portal 2 PEGI 12+ < Deus Ex HR PEGI 18+
	assert!(p_ratings.pegi < d_ratings.pegi);
}

#[test]
fn security_info_file_key_not_all_zeros_for_encrypted() {
	let (_data, xex) = load_xex("AntiPiracyUI.xex");
	assert_ne!(xex.security_info.image_info.file_key, xex2::header::AesKey([0u8; 16]));
}

#[test]
fn extract_unencrypted_normal_produces_pe() {
	let (data, xex) = load_xex("xshell twi.xex");
	let fmt = xex.header.file_format_info().unwrap();
	assert_eq!(fmt.encryption_type, xex2::header::EncryptionType::None);
	assert_eq!(fmt.compression_type, CompressionType::Normal);
	let basefile = xex.extract_basefile(&data).unwrap();
	assert_eq!(&basefile[0..2], b"MZ");
}

#[test]
fn extract_encrypted_normal_produces_pe() {
	let (data, xex) = load_xex("ArchEngine.xex");
	let fmt = xex.header.file_format_info().unwrap();
	assert_eq!(fmt.encryption_type, xex2::header::EncryptionType::Normal);
	assert_eq!(fmt.compression_type, CompressionType::Normal);
	let basefile = xex.extract_basefile(&data).unwrap();
	assert_eq!(&basefile[0..2], b"MZ");
}

#[test]
fn extract_large_window_normal() {
	let (data, xex) = load_xex("xshell - Copy.xex");
	let fmt = xex.header.file_format_info().unwrap();
	assert_eq!(fmt.compression_type, CompressionType::Normal);
	assert_eq!(fmt.window_size, Some(0x100000));
	let basefile = xex.extract_basefile(&data).unwrap();
	assert_eq!(&basefile[0..2], b"MZ");
}

#[test]
fn extract_multiple_normal_compression() {
	for name in &["xbdm.xex", "mfgbootlauncher.xex", "BBNeo!_0424.xex"] {
		let (data, xex) = load_xex(name);
		let fmt = xex.header.file_format_info().unwrap();
		if fmt.compression_type == CompressionType::Normal {
			let basefile = xex.extract_basefile(&data).unwrap();
			assert_eq!(&basefile[0..2], b"MZ", "failed for {}", name);
		}
	}
}

#[test]
fn patch_resign_verifies_with_devkit_key() {
	let (data, xex) = load_xex("haloreach-powerhouse.xex");
	let mut limits = xex2::writer::RemoveLimits::default();
	limits.region = true;
	limits.media = true;

	let patched_data = xex.modify(&data, &limits).unwrap();

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

#[test]
fn rebuild_fast_path_matches_modify() {
	let (data, xex) = load_xex("haloreach-powerhouse.xex");
	let mut limits = xex2::writer::RemoveLimits::default();
	limits.region = true;
	limits.media = true;
	limits.zero_media_id = true;

	let via_modify = xex.modify(&data, &limits).unwrap();

	let mut via_stream = Vec::new();
	xex.rebuild(&data).remove_limits(limits.clone()).write_to(&mut via_stream).unwrap();

	assert_eq!(via_modify.len(), via_stream.len());
	assert_eq!(via_modify, via_stream);
}

#[test]
fn bounding_path_clears_module_flag() {
	let (data, xex) = load_xex("haloreach-powerhouse.xex");
	let original = xex.header.module_flags;

	let mut limits = xex2::writer::RemoveLimits::default();
	limits.bounding_path = true;
	let patched = xex.modify(&data, &limits).unwrap();

	let new_flags = u32::from_be_bytes(patched[0x04..0x08].try_into().unwrap());
	let new_flags = xex2::opt::ModuleFlags::from_bits_retain(new_flags);
	assert!(!new_flags.contains(xex2::opt::ModuleFlags::BOUND_PATH));
	// Other flag bits untouched.
	let mask = !xex2::opt::ModuleFlags::BOUND_PATH.bits();
	assert_eq!(new_flags.bits() & mask, original.bits() & mask);
}

#[test]
fn device_id_clears_module_flag() {
	let (data, xex) = load_xex("haloreach-powerhouse.xex");
	let original = xex.header.module_flags;

	let mut limits = xex2::writer::RemoveLimits::default();
	limits.device_id = true;
	let patched = xex.modify(&data, &limits).unwrap();

	let new_flags = u32::from_be_bytes(patched[0x04..0x08].try_into().unwrap());
	let new_flags = xex2::opt::ModuleFlags::from_bits_retain(new_flags);
	assert!(!new_flags.contains(xex2::opt::ModuleFlags::DEVICE_ID));
	let mask = !xex2::opt::ModuleFlags::DEVICE_ID.bits();
	assert_eq!(new_flags.bits() & mask, original.bits() & mask);
}

#[test]
fn image_flag_limits_re_sign() {
	let (data, xex) = load_xex("haloreach-powerhouse.xex");
	// Combine with `region` to guarantee image_info changes and thus a re-sign,
	// regardless of whether this XEX has the keyvault bits originally set.
	let mut limits = xex2::writer::RemoveLimits::default();
	limits.region = true;
	limits.keyvault_privileges = true;
	limits.signed_keyvault_only = true;

	let patched = xex.modify(&data, &limits).unwrap();

	let sec_off = u32::from_be_bytes(patched[0x10..0x14].try_into().unwrap()) as usize;
	let flags_off = sec_off + 0x10C;
	let new_flags = u32::from_be_bytes(patched[flags_off..flags_off + 4].try_into().unwrap());
	let new_flags = xex2::opt::ImageFlags::from_bits_retain(new_flags);
	assert!(!new_flags.contains(xex2::opt::ImageFlags::KV_PRIVILEGES_REQUIRED));
	assert!(!new_flags.contains(xex2::opt::ImageFlags::SIGNED_KEYVAULT_REQUIRED));

	// Signature must re-verify with the devkit key since image_info changed.
	let info_size_off = sec_off + 0x108;
	let info_size = u32::from_be_bytes(patched[info_size_off..info_size_off + 4].try_into().unwrap()) as usize;
	let image_info_len = info_size - 0x100;
	let image_info = &patched[info_size_off..info_size_off + image_info_len];
	let digest = xecrypt::symmetric::xe_crypt_rot_sum_sha(image_info, &[]);
	let sig = &patched[sec_off + 0x08..sec_off + 0x108];
	xecrypt::RsaKeyKind::Pirs
		.verify_signature(xecrypt::ConsoleKind::Devkit, sig, &digest)
		.expect("devkit PIRS signature should verify after image_flags edit");
}

#[test]
fn unimplemented_limits_error() {
	let (data, xex) = load_xex("afplayer.xex");

	let mut l = xex2::writer::RemoveLimits::default();
	l.dates = true;
	let err = xex.modify(&data, &l).unwrap_err();
	assert!(format!("{}", err).contains("dates"));

	let mut l = xex2::writer::RemoveLimits::default();
	l.console_id = true;
	let err = xex.modify(&data, &l).unwrap_err();
	assert!(format!("{}", err).contains("console_id"));

	let mut l = xex2::writer::RemoveLimits::default();
	l.library_versions = true;
	let err = xex.modify(&data, &l).unwrap_err();
	assert!(format!("{}", err).contains("library_versions"));

	let mut l = xex2::writer::RemoveLimits::default();
	l.revocation_check = true;
	let err = xex.modify(&data, &l).unwrap_err();
	assert!(format!("{}", err).contains("revocation_check"));
}

#[test]
fn rebuild_transform_not_implemented() {
	let (data, xex) = load_xex("afplayer.xex");
	let mut sink = Vec::new();
	let result = xex
		.rebuild(&data)
		.target_compression(xex2::writer::TargetCompression::Uncompressed)
		.write_to(&mut sink);
	assert!(result.is_err(), "rebuild with non-Unchanged compression should error");
}
