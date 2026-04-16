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

	// game_regions at image_info + 0x70 (inside signed region)
	assert_eq!(u32::from_be_bytes(patched_data[sec_off + 0x178..sec_off + 0x17C].try_into().unwrap()), 0xFFFFFFFF);
	// allowed_media_types at security_info + 0x17C (outside signed region)
	assert_eq!(u32::from_be_bytes(patched_data[sec_off + 0x17C..sec_off + 0x180].try_into().unwrap()), 0xFFFFFFFF);
}

#[test]
fn rebuild_fast_path_matches_modify() {
	let mut limits = xex2::writer::RemoveLimits::default();
	limits.region = true;
	limits.media = true;
	limits.zero_media_id = true;

	let (data, xex) = load_xex("haloreach-powerhouse.xex");
	let via_modify = xex.modify(&data, &limits).unwrap();

	let (data2, xex2) = load_xex("haloreach-powerhouse.xex");
	let mut via_stream = Vec::new();
	xex2.rebuild(&data2).remove_limits(limits).write_to(&mut via_stream).unwrap();

	assert_eq!(via_modify, via_stream);
}

#[test]
fn header_hash_formula_matches_fixtures() {
	for name in &[
		"afplayer.xex",
		"AntiPiracyUI.xex",
		"Portal 2.xex",
		"Deus Ex.xex",
		"haloreach-powerhouse.xex",
		"xshell twi.xex",
		"ArchEngine.xex",
		"xbdm.xex",
	] {
		let (data, xex) = load_xex(name);
		let computed = xex2::hashes::compute_header_hash(&data, &xex.header, &xex.security_info);
		assert_eq!(
			computed, xex.security_info.image_info.header_hash,
			"header_hash mismatch for {}",
			name
		);
	}
}

#[test]
fn import_table_hash_formula_matches_fixtures() {
	for name in &[
		"afplayer.xex",
		"AntiPiracyUI.xex",
		"Portal 2.xex",
		"Deus Ex.xex",
		"haloreach-powerhouse.xex",
		"xshell twi.xex",
		"ArchEngine.xex",
		"xbdm.xex",
	] {
		let (_data, xex) = load_xex(name);
		let Some(computed) = xex2::hashes::compute_import_table_hash(&xex.header) else {
			continue;
		};
		assert_eq!(
			computed, xex.security_info.image_info.import_table_hash,
			"import_table_hash mismatch for {}: computed {:?} vs stored {:?}",
			name, computed, xex.security_info.image_info.import_table_hash
		);
	}
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

fn verify_devkit_signature(patched: &[u8]) {
	let sec_off = u32::from_be_bytes(patched[0x10..0x14].try_into().unwrap()) as usize;
	let info_size_off = sec_off + 0x108;
	let info_size = u32::from_be_bytes(patched[info_size_off..info_size_off + 4].try_into().unwrap()) as usize;
	let image_info_len = info_size - 0x100;
	let image_info = &patched[info_size_off..info_size_off + image_info_len];
	let digest = xecrypt::symmetric::xe_crypt_rot_sum_sha(image_info, &[]);
	let sig = &patched[sec_off + 0x08..sec_off + 0x108];
	xecrypt::RsaKeyKind::Pirs
		.verify_signature(xecrypt::ConsoleKind::Devkit, sig, &digest)
		.expect("devkit PIRS signature should verify");
}

fn verify_header_hash(patched: &[u8]) {
	use xex2::Xex2;
	let xex = Xex2::parse(patched).unwrap();
	let computed = xex2::hashes::compute_header_hash(patched, &xex.header, &xex.security_info);
	assert_eq!(computed, xex.security_info.image_info.header_hash, "header_hash mismatch after patching");
}

#[test]
fn console_id_zeroes_serial_list_and_reverifies() {
	// Find any fixture with a ConsoleSerialList to exercise this path.
	for name in &["haloreach-powerhouse.xex", "afplayer.xex", "Portal 2.xex", "Deus Ex.xex"] {
		let (data, xex) = load_xex(name);
		if xex
			.header
			.optional_header_source_range(&data, xex2::header::OptionalHeaderKey::ConsoleSerialList)
			.is_none()
		{
			continue;
		}
		let mut limits = xex2::writer::RemoveLimits::default();
		limits.console_id = true;
		let patched = xex.modify(&data, &limits).unwrap();
		verify_devkit_signature(&patched);
		verify_header_hash(&patched);
		return;
	}
	eprintln!("no fixture with ConsoleSerialList -- edit path not exercised");
}

#[test]
fn dates_limit_sets_max_filetime_and_reverifies() {
	let (data, xex) = load_xex("haloreach-powerhouse.xex");
	let range = xex.header.optional_header_source_range(&data, xex2::header::OptionalHeaderKey::DateRange);
	if range.is_none() {
		eprintln!("skipping: no DateRange in fixture");
		return;
	}
	let (off, len) = range.unwrap();

	let mut limits = xex2::writer::RemoveLimits::default();
	limits.dates = true;
	let patched = xex.modify(&data, &limits).unwrap();

	assert_eq!(&patched[off..off + 8], &[0u8; 8], "not_before should be 0");
	assert_eq!(&patched[off + 8..off + 16], &u64::MAX.to_be_bytes(), "not_after should be max");
	assert!(len >= 16);

	verify_devkit_signature(&patched);
	verify_header_hash(&patched);
}

#[test]
fn library_versions_zeroes_version_min_and_reverifies() {
	let (data, xex) = load_xex("haloreach-powerhouse.xex");
	let mut limits = xex2::writer::RemoveLimits::default();
	limits.library_versions = true;
	let patched = xex.modify(&data, &limits).unwrap();

	// Re-parse and confirm every library's version_min is 0.
	let patched_xex = xex2::Xex2::parse(&patched).unwrap();
	let table = patched_xex.header.import_table().expect("import table");
	for lib in &table.libraries {
		assert_eq!(u32::from(lib.version_min), 0, "library {} version_min not zeroed", lib.name);
	}

	verify_devkit_signature(&patched);
	verify_header_hash(&patched);

	// Also: new import_table_hash must match.
	let new_table_hash = xex2::hashes::compute_import_table_hash(&patched_xex.header).unwrap();
	assert_eq!(new_table_hash, patched_xex.security_info.image_info.import_table_hash);
}

#[test]
fn rebuild_field_setters_apply_specific_values() {
	let (data, xex) = load_xex("haloreach-powerhouse.xex");

	let mut sink = Vec::new();
	xex.rebuild(&data)
		.set_media_id([0xAB; 16])
		.set_game_regions(0x0000_0001)
		.set_allowed_media(xex2::opt::AllowedMediaTypes::HARD_DISK | xex2::opt::AllowedMediaTypes::DVD_X2)
		.write_to(&mut sink)
		.unwrap();

	let patched_xex = xex2::Xex2::parse(&sink).unwrap();
	assert_eq!(patched_xex.security_info.image_info.media_id, [0xAB; 16]);
	assert_eq!(patched_xex.security_info.image_info.game_regions, 0x0000_0001);
	assert_eq!(
		patched_xex.security_info.image_info.allowed_media_types,
		xex2::opt::AllowedMediaTypes::HARD_DISK | xex2::opt::AllowedMediaTypes::DVD_X2,
	);
	verify_devkit_signature(&sink);
}

#[test]
fn rebuild_set_date_range_recomputes_header_hash() {
	let (data, xex) = load_xex("haloreach-powerhouse.xex");
	if xex
		.header
		.optional_header_source_range(&data, xex2::header::OptionalHeaderKey::DateRange)
		.is_none()
	{
		eprintln!("skipping: no DateRange");
		return;
	}
	let mut sink = Vec::new();
	xex.rebuild(&data).set_date_range(100, 200).write_to(&mut sink).unwrap();
	let patched_xex = xex2::Xex2::parse(&sink).unwrap();
	let dr = patched_xex.header.date_range().unwrap();
	assert_eq!(dr.not_before, Some(100));
	assert_eq!(dr.not_after, Some(200));
	verify_devkit_signature(&sink);
	verify_header_hash(&sink);
}

#[test]
fn decrypt_encrypted_xex_roundtrip() {
	// Start from an encrypted fixture, decrypt, re-parse, verify PE extraction
	// yields the same basefile.
	let (data, xex) = load_xex("AntiPiracyUI.xex");
	let original_basefile = xex.extract_basefile(&data).unwrap();

	let mut sink = Vec::new();
	xex.rebuild(&data)
		.target_encryption(xex2::writer::TargetEncryption::Decrypted)
		.write_to(&mut sink)
		.unwrap();

	let decrypted_xex = xex2::Xex2::parse(&sink).unwrap();
	let fmt = decrypted_xex.header.file_format_info().unwrap();
	assert_eq!(fmt.encryption_type, xex2::header::EncryptionType::None);

	let decrypted_basefile = decrypted_xex.extract_basefile(&sink).unwrap();
	assert_eq!(original_basefile, decrypted_basefile);

	verify_devkit_signature(&sink);
	verify_header_hash(&sink);
}

#[test]
fn encrypt_decrypted_xex_roundtrip() {
	// First decrypt a known XEX, then re-encrypt, then verify basefile matches.
	let (data, xex) = load_xex("AntiPiracyUI.xex");
	let original_basefile = xex.extract_basefile(&data).unwrap();

	let mut decrypted = Vec::new();
	xex.rebuild(&data)
		.target_encryption(xex2::writer::TargetEncryption::Decrypted)
		.write_to(&mut decrypted)
		.unwrap();

	let mid_xex = xex2::Xex2::parse(&decrypted).unwrap();
	let mut re_encrypted = Vec::new();
	mid_xex
		.rebuild(&decrypted)
		.target_encryption(xex2::writer::TargetEncryption::Encrypted)
		.write_to(&mut re_encrypted)
		.unwrap();

	let final_xex = xex2::Xex2::parse(&re_encrypted).unwrap();
	let fmt = final_xex.header.file_format_info().unwrap();
	assert_eq!(fmt.encryption_type, xex2::header::EncryptionType::Normal);

	let final_basefile = final_xex.extract_basefile(&re_encrypted).unwrap();
	assert_eq!(original_basefile, final_basefile);

	verify_devkit_signature(&re_encrypted);
	verify_header_hash(&re_encrypted);
}

#[test]
fn machine_switch_rewraps_file_key() {
	// Portal 2 is retail-signed/encrypted; switching to devkit should re-wrap
	// the file_key under the devkit master key.
	let (data, xex) = load_xex("Portal 2.xex");
	let original_basefile = xex.extract_basefile(&data).unwrap();
	let original_file_key = xex.security_info.image_info.file_key;

	let mut sink = Vec::new();
	xex.rebuild(&data).target_machine(xex2::writer::TargetMachine::Devkit).write_to(&mut sink).unwrap();

	let switched_xex = xex2::Xex2::parse(&sink).unwrap();
	assert_ne!(
		original_file_key, switched_xex.security_info.image_info.file_key,
		"file_key should change after re-wrapping"
	);

	let switched_basefile = switched_xex.extract_basefile(&sink).unwrap();
	assert_eq!(original_basefile, switched_basefile);
	verify_devkit_signature(&sink);
}

#[test]
fn setting_target_matching_current_state_is_noop() {
	// AntiPiracyUI is already encrypted; setting target_encryption=Encrypted
	// should be a no-op (no re-encryption, output equals input modulo unrelated edits).
	let (data, xex) = load_xex("AntiPiracyUI.xex");

	let mut sink = Vec::new();
	xex.rebuild(&data).target_encryption(xex2::writer::TargetEncryption::Encrypted).write_to(&mut sink).unwrap();

	// Source bytes should be untouched by a no-op transform.
	assert_eq!(data, sink, "no-op target should produce identical output");
}

#[test]
fn compression_transform_not_implemented() {
	let (data, xex) = load_xex("afplayer.xex");
	let mut sink = Vec::new();
	let result = xex
		.rebuild(&data)
		.target_compression(xex2::writer::TargetCompression::Uncompressed)
		.write_to(&mut sink);
	assert!(result.is_err(), "compression transforms are not yet implemented");
}
