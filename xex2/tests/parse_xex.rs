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
	let limits = xex2::writer::RemoveLimits { region: true, media: true, ..Default::default() };

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
	let limits = xex2::writer::RemoveLimits { region: true, media: true, zero_media_id: true, ..Default::default() };

	let (data, xex) = load_xex("haloreach-powerhouse.xex");
	let via_modify = xex.modify(&data, &limits).unwrap();

	let (data2, xex2) = load_xex("haloreach-powerhouse.xex");
	let mut via_stream = Vec::new();
	xex2.rebuild(&data2).remove_limits(limits).write_to(&mut via_stream).unwrap();

	assert_eq!(via_modify, via_stream);
}

#[test]
fn page_descriptor_chain_matches_fixtures() {
	let mut failures = Vec::new();
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
		let basefile = xex.extract_basefile(&data).unwrap();
		match xex2::page_descriptors::verify_chain(&basefile, &xex.header, &xex.security_info, &data) {
			Ok(()) => eprintln!("{}: OK", name),
			Err(e) => {
				eprintln!("{}: {:?}", name, e);
				failures.push(*name);
			}
		}
	}
	assert!(failures.is_empty(), "chain mismatch: {:?}", failures);
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
		assert_eq!(computed, xex.security_info.image_info.header_hash, "header_hash mismatch for {}", name);
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

	let limits = xex2::writer::RemoveLimits { bounding_path: true, ..Default::default() };
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

	let limits = xex2::writer::RemoveLimits { device_id: true, ..Default::default() };
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
	let limits = xex2::writer::RemoveLimits {
		region: true,
		keyvault_privileges: true,
		signed_keyvault_only: true,
		..Default::default()
	};

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
		if xex.header.optional_header_source_range(&data, xex2::header::OptionalHeaderKey::ConsoleSerialList).is_none()
		{
			continue;
		}
		let limits = xex2::writer::RemoveLimits { console_id: true, ..Default::default() };
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
	let Some(span) = xex.header.optional_header_source_range(&data, xex2::header::OptionalHeaderKey::DateRange) else {
		eprintln!("skipping: no DateRange in fixture");
		return;
	};
	let off = span.offset.as_usize();

	let limits = xex2::writer::RemoveLimits { dates: true, ..Default::default() };
	let patched = xex.modify(&data, &limits).unwrap();

	assert_eq!(&patched[off..off + 8], &[0u8; 8], "not_before should be 0");
	assert_eq!(&patched[off + 8..off + 16], &u64::MAX.to_be_bytes(), "not_after should be max");
	assert!(span.len >= 16);

	verify_devkit_signature(&patched);
	verify_header_hash(&patched);
}

#[test]
fn library_versions_zeroes_version_min_and_reverifies() {
	let (data, xex) = load_xex("haloreach-powerhouse.xex");
	let limits = xex2::writer::RemoveLimits { library_versions: true, ..Default::default() };
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
	if xex.header.optional_header_source_range(&data, xex2::header::OptionalHeaderKey::DateRange).is_none() {
		eprintln!("skipping: no DateRange");
		return;
	}
	let mut sink = Vec::new();
	xex.rebuild(&data)
		.set_date_range(xex2::writer::DateRangeEdit { not_before: 100, not_after: 200 })
		.write_to(&mut sink)
		.unwrap();
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
	xex.rebuild(&data).target_encryption(xex2::writer::TargetEncryption::Decrypted).write_to(&mut sink).unwrap();

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
	xex.rebuild(&data).target_encryption(xex2::writer::TargetEncryption::Decrypted).write_to(&mut decrypted).unwrap();

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
fn replace_pe_rejects_basic_compression() {
	// Basic-compressed sources aren't handled by the full rebuild path yet.
	let (data, xex) = load_xex("afplayer.xex");
	let mut sink = Vec::new();
	let mut pe = vec![0u8; 1024];
	pe[0] = b'M';
	pe[1] = b'Z';
	let result = xex.rebuild(&data).replace_pe(pe).write_to(&mut sink);
	assert!(result.is_err(), "replace_pe on Basic-compressed XEX isn't supported yet");
}

#[test]
fn replace_pe_on_normal_compressed_source() {
	// xshell twi.xex is Normal-compressed. With the full-rebuild path we can
	// decompress under the hood, swap the PE, and recompress transparently.
	let (data, xex) = load_xex("xshell twi.xex");
	let image_size = xex.security_info.image_size as usize;

	// Craft a replacement PE that matches the original image_size (page
	// descriptors cover the decompressed bytes).
	let mut pe = vec![0u8; image_size];
	pe[0] = b'M';
	pe[1] = b'Z';
	for (i, b) in pe.iter_mut().enumerate().skip(2) {
		*b = ((i * 17) & 0xFF) as u8;
	}

	let mut sink = Vec::new();
	xex.rebuild(&data).replace_pe(pe.clone()).write_to(&mut sink).unwrap();

	let rebuilt = Xex2::parse(&sink).unwrap();
	let extracted = rebuilt.extract_basefile(&sink).unwrap();
	assert_eq!(extracted, pe, "replace_pe on Normal-compressed source must round-trip the replacement");
}

#[test]
fn builder_produces_parseable_xex() {
	use xex2::builder::Xex2Builder;

	// Fabricate a minimal 64KB PE (MZ header + zeros).
	let mut pe = vec![0u8; 64 * 1024];
	pe[0] = b'M';
	pe[1] = b'Z';

	let bytes = Xex2Builder::new(pe.clone())
		.title_id(xenon_types::TitleId(0x4D530914))
		.media_id(xenon_types::MediaId(0xDEADBEEF))
		.version(xenon_types::Version::from(0x2000_0000))
		.entry_point(xenon_types::VirtualAddress(0x82001000))
		.load_address(xenon_types::VirtualAddress(0x82000000))
		.build()
		.unwrap();

	// Must parse back.
	let parsed = xex2::Xex2::parse(&bytes).unwrap();
	assert_eq!(parsed.header.module_flags, xex2::opt::ModuleFlags::TITLE);
	assert_eq!(parsed.security_info.image_info.load_address, xenon_types::VirtualAddress(0x82000000));
	assert_eq!(parsed.security_info.image_size as usize, pe.len());

	let exec = parsed.header.execution_info().unwrap();
	assert_eq!(exec.title_id, xenon_types::TitleId(0x4D530914));
	assert_eq!(exec.media_id, xenon_types::MediaId(0xDEADBEEF));
	assert_eq!(parsed.header.entry_point(), Some(0x82001000));

	// Basefile extraction should return the original PE.
	let extracted = parsed.extract_basefile(&bytes).unwrap();
	assert_eq!(&extracted[0..2], b"MZ");
	assert_eq!(extracted.len(), pe.len());

	// Signature + header_hash must verify.
	verify_devkit_signature(&bytes);
	verify_header_hash(&bytes);
}

#[test]
fn builder_produces_compressed_xex() {
	use xex2::builder::Xex2Builder;

	// 128 KB PE so we get multiple LZX chunks.
	let mut pe = vec![0u8; 128 * 1024];
	pe[0] = b'M';
	pe[1] = b'Z';
	for (i, b) in pe.iter_mut().enumerate().skip(64) {
		*b = (i & 0xFF) as u8;
	}

	let bytes = Xex2Builder::new(pe.clone())
		.title_id(xenon_types::TitleId(0x4D530914))
		.entry_point(xenon_types::VirtualAddress(0x82001000))
		.load_address(xenon_types::VirtualAddress(0x82000000))
		.compress()
		.build()
		.unwrap();

	let parsed = xex2::Xex2::parse(&bytes).unwrap();
	let fmt = parsed.header.file_format_info().unwrap();
	assert_eq!(fmt.compression_type, CompressionType::Normal);
	assert_eq!(fmt.window_size, Some(0x10000));
	assert_eq!(parsed.security_info.image_size as usize, pe.len());

	let extracted = parsed.extract_basefile(&bytes).unwrap();
	assert_eq!(extracted, pe, "compressed builder output must round-trip the PE");
}

#[test]
fn basic_compression_transform_not_implemented() {
	// afplayer.xex is Basic-compressed. Basic ↔ anything isn't wired up yet;
	// confirm we still bail cleanly rather than produce garbage.
	let (data, xex) = load_xex("afplayer.xex");
	let mut sink = Vec::new();
	let result =
		xex.rebuild(&data).target_compression(xex2::writer::TargetCompression::Uncompressed).write_to(&mut sink);
	assert!(result.is_err(), "basic → uncompressed transform shouldn't be supported yet");
}

#[test]
fn rebuild_decompresses_normal_xex() {
	// xshell twi.xex is Normal-compressed, unencrypted. Rebuild it as
	// uncompressed and verify the extracted PE matches direct extraction.
	let (data, xex) = load_xex("xshell twi.xex");
	assert_eq!(xex.header.file_format_info().unwrap().compression_type, CompressionType::Normal);

	let original_pe = xex.extract_basefile(&data).unwrap();

	let mut sink = Vec::new();
	Xex2::parse(&data)
		.unwrap()
		.rebuild(&data)
		.target_compression(xex2::writer::TargetCompression::Uncompressed)
		.write_to(&mut sink)
		.unwrap();

	let rebuilt = Xex2::parse(&sink).unwrap();
	let fmt = rebuilt.header.file_format_info().unwrap();
	assert_eq!(fmt.compression_type, CompressionType::None);
	let rebuilt_pe = rebuilt.extract_basefile(&sink).unwrap();
	assert_eq!(rebuilt_pe, original_pe, "uncompressed rebuild must round-trip the PE");
}

#[test]
fn rebuild_compresses_uncompressed_xex() {
	// Start from a Normal XEX → decompress → compress. The compressed
	// rebuild must round-trip back to the same PE bytes.
	let (data, xex) = load_xex("xshell twi.xex");
	let original_pe = xex.extract_basefile(&data).unwrap();

	// Step 1: decompress the source.
	let mut decompressed = Vec::new();
	Xex2::parse(&data)
		.unwrap()
		.rebuild(&data)
		.target_compression(xex2::writer::TargetCompression::Uncompressed)
		.write_to(&mut decompressed)
		.unwrap();

	// Step 2: recompress the now-uncompressed output.
	let uncompressed = Xex2::parse(&decompressed).unwrap();
	let mut recompressed = Vec::new();
	Xex2::parse(&decompressed)
		.unwrap()
		.rebuild(&decompressed)
		.target_compression(xex2::writer::TargetCompression::Normal)
		.write_to(&mut recompressed)
		.unwrap();
	let _ = uncompressed;

	let rebuilt = Xex2::parse(&recompressed).unwrap();
	let fmt = rebuilt.header.file_format_info().unwrap();
	assert_eq!(fmt.compression_type, CompressionType::Normal);
	let rebuilt_pe = rebuilt.extract_basefile(&recompressed).unwrap();
	assert_eq!(rebuilt_pe, original_pe, "compressed rebuild must round-trip the PE");
}
