mod common;

use common::josh_block;
use xecrypt::CertificateStatus;
use xecrypt::console::CertificateKey;
use xenon_disk::JoshBlock;
use xenon_disk::JoshBlockError;
use xenon_disk::LogoBitmap;
use xenon_disk::LogoBitmapError;
use xenon_disk::SecuritySector;
use xenon_disk::josh::JOSH_BLOCK_SIZE;
use xenon_disk::josh::JOSH_BLOCK_VERSION;
use xenon_disk::security::LOGO_BITMAP_SIZE;

/// The made-up certificate carries no authority signature, so it is invalid while the console
/// signature over the cache table verifies with its key.
#[test]
fn josh_block_fields_and_console_signature() {
	let bytes: [u8; JOSH_BLOCK_SIZE] = josh_block().try_into().unwrap();

	let block = JoshBlock::parse(&bytes).unwrap();

	insta::assert_debug_snapshot!(block);
	let verification = block.verify_console_signature();
	assert_eq!(verification.certificate, CertificateStatus::Invalid { key: CertificateKey::Key3C });
	assert_eq!(verification.signature, Ok(()));
}

#[test]
fn a_changed_cache_table_fails_the_console_signature() {
	let mut bytes: [u8; JOSH_BLOCK_SIZE] = josh_block().try_into().unwrap();
	bytes[0x27c] ^= 1;

	let block = JoshBlock::parse(&bytes).unwrap();

	assert_eq!(block.verify_console_signature().signature, Err(xecrypt::ConsoleSignatureError::Mismatch));
}

#[test]
fn josh_block_needs_its_magic() {
	let mut bytes: [u8; JOSH_BLOCK_SIZE] = josh_block().try_into().unwrap();
	bytes[0] = b'j';

	assert_eq!(JoshBlock::parse(&bytes), Err(JoshBlockError::BadMagic { magic: *b"josh" }));
}

#[test]
fn logo_length_must_fit_the_region() {
	let mut region = [0u8; LOGO_BITMAP_SIZE];
	region[..4].copy_from_slice(&0xbfdu32.to_be_bytes());

	assert_eq!(LogoBitmap::parse(&region), Err(LogoBitmapError::LengthPastRegion { length: 0xbfd, available: 0xbfc }));

	region[..4].copy_from_slice(&0xbfcu32.to_be_bytes());
	let logo = LogoBitmap::parse(&region).unwrap();
	assert_eq!(logo.image().len(), 0xbfc);
	assert!(!logo.is_png());
}

/// The first bytes of a real hard disk, at the path in `XENON_DISK_HDD_HEAD`, through at least
/// the security sector. The logo region is checked when the dump holds all of it. Only structure
/// is checked, so no drive identity is recorded here.
#[test]
#[ignore = "reads a real disk head from the path in XENON_DISK_HDD_HEAD"]
fn real_disk_head() {
	let path = std::env::var_os("XENON_DISK_HDD_HEAD").expect("XENON_DISK_HDD_HEAD names a disk head dump");
	let head = std::fs::read(path).unwrap();

	let josh = JoshBlock::parse(head[0x800..0x800 + JOSH_BLOCK_SIZE].try_into().unwrap()).unwrap();
	let certificate = &josh.console_signature.certificate;
	assert_eq!(certificate.cert_size, 0x1a8);
	assert!(certificate.console_part_number.is_printable());
	assert_eq!(certificate.manufacturing_date.bytes()[2], b'-');
	assert_eq!(josh.version, JOSH_BLOCK_VERSION);
	let verification = josh.verify_console_signature();
	assert_eq!(verification.certificate, CertificateStatus::Verified { key: CertificateKey::Key3C });
	assert_eq!(verification.signature, Ok(()));

	let sector = SecuritySector::parse(head[0x2000..0x2200].try_into().unwrap());
	assert!(sector.is_plausible());
	println!(
		"console signature verified; console kind {:?}; security sector signature absent: {}",
		certificate.console_type.kind(),
		sector.is_signature_absent()
	);

	let Some(region) = head.get(0x2200..0x2e00) else {
		return;
	};
	let region: &[u8; LOGO_BITMAP_SIZE] = region.try_into().unwrap();
	assert!(sector.check(region, sector.user_addressable_sectors).is_empty());
	let logo = LogoBitmap::parse(region).unwrap();
	assert!(logo.is_png());
	println!("logo {:#x} bytes", logo.image().len());
}
