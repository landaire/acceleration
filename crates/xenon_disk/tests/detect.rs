mod common;

use std::fmt::Write;

use common::SparseImage;
use common::render;
use xenon_disk::ByteCount;
use xenon_disk::Checked;
use xenon_disk::DetectError;
use xenon_disk::DetectOptions;
use xenon_disk::Detection;
use xenon_disk::DeviceKind;
use xenon_disk::DiskWarning;
use xenon_disk::MemoryDumpSize;
use xenon_disk::SectorCount;
use xenon_disk::UsbStorageLayout;
use xenon_disk::detect;
use xenon_disk::detect_async;

const OPTIONS: DetectOptions = DetectOptions { memory_dump: MemoryDumpSize::Standard };
/// The sector count of a 250 GB disk, and the end of Partition1 in the known devkit table.
const DISK_SECTORS: u32 = 0x1c9a_5970;

fn run(image: &SparseImage) -> Checked<Detection, DiskWarning> {
	detect(image, ByteCount::new(image.length()), OPTIONS).unwrap()
}

/// The detected kind, evidence, alternatives and warnings, then the map with a `*` on each
/// partition that starts with a volume header.
fn report(checked: &Checked<Detection, DiskWarning>) -> String {
	let detection = checked.value();
	let mut text = format!("kind: {:?}\n", detection.kind());
	for evidence in detection.evidence() {
		writeln!(text, "evidence: {evidence:?}").unwrap();
	}
	for alternative in detection.alternatives() {
		writeln!(text, "alternative: {alternative:?}").unwrap();
	}
	for warning in checked.warnings() {
		writeln!(text, "warning: {warning}").unwrap();
	}
	let headers: Vec<&str> = detection
		.map()
		.partitions()
		.iter()
		.filter(|partition| detection.has_volume_header(partition))
		.map(|partition| partition.name())
		.collect();
	writeln!(text, "volume headers: {}", headers.join(", ")).unwrap();
	text.push_str(&render(detection.map()));

	text
}

#[test]
fn sfc_memory_unit() {
	let mut image = SparseImage::new(0x1aca_0000);
	image.volume(0).volume(0x1a4a_1000);

	insta::assert_snapshot!(report(&run(&image)));
}

#[test]
fn usb_memory_unit() {
	let mut image = SparseImage::new(0x1_e000 * 0x200);
	image.volume(0).volume(0x7f_f000);

	let checked = run(&image);

	assert_eq!(checked.value().kind(), DeviceKind::UsbMemoryUnit);
	assert!(checked.value().alternatives().is_empty());
}

#[test]
fn a_header_at_offset_zero_alone_is_a_lone_volume() {
	let mut image = SparseImage::new(0x1_e000 * 0x200);
	image.volume(0);

	let error = detect(&image, ByteCount::new(image.length()), OPTIONS).unwrap_err();

	assert!(matches!(error, DetectError::LoneVolume { length } if length.get() == image.length()));
}

#[test]
fn a_volume_file_of_no_device_size_is_a_lone_volume() {
	let mut image = SparseImage::new(0x8000);
	image.volume(0);

	let error = detect(&image, ByteCount::new(image.length()), OPTIONS).unwrap_err();

	assert!(matches!(error, DetectError::LoneVolume { .. }));
}

#[test]
fn retail_disk() {
	let mut image = SparseImage::new(u64::from(DISK_SECTORS) * 0x200);
	image.security_sector(DISK_SECTORS - 0x1000).volume(0x1_20eb_0000).volume(0x1_30eb_0000);

	let checked = run(&image);

	assert!(checked.warnings().is_empty());
	let security = checked.value().security_sector().unwrap();
	assert_eq!(security.serial_number.to_string(), "5VJ1AB2C");
	assert_eq!(security.model_number.to_string(), "ST9250315AS");
	insta::assert_snapshot!(report(&checked));
}

#[test]
fn a_damaged_logo_bitmap_fails_authentication() {
	let mut image = SparseImage::new(u64::from(DISK_SECTORS) * 0x200);
	image.security_sector(DISK_SECTORS - 0x1000).write(0x2400, &[0xff; 4]);

	let checked = run(&image);

	assert_eq!(checked.value().kind(), DeviceKind::RetailHdd);
	assert!(matches!(checked.warnings(), [DiskWarning::LogoDigestMismatch { .. }]));
	// The kernel keeps the device count when the security sector does not authenticate.
	let partition0 = checked.value().map().partition("Partition0").unwrap();
	assert_eq!(partition0.range().end, image.length());
}

#[test]
fn devkit_disk_from_the_table() {
	let mut image = SparseImage::new(u64::from(DISK_SECTORS) * 0x200);
	image.write(0, &[0x00, 0x02, 0x00, 0x00, 0x53, 0x08, 0x00, 0x03, 0x01, 0x68, 0x75, 0x80, 0x1b, 0x31, 0xe3, 0xf0]);
	image.security_sector(DISK_SECTORS).unsigned().josh_block().volume(0x2_d0eb_0000);

	let checked = run(&image);

	let detection = checked.value();
	let josh = detection.josh_block().unwrap();
	assert_eq!(josh.console_signature.certificate.console_part_number.to_string(), "X000000-001");
	assert_eq!(josh.owners, [None, Some(xenon_types::TitleId(0x4d53_0001))]);
	assert_eq!(josh.verify_console_signature().signature, Ok(()));
	// The made-up certificate carries no authority signature.
	assert!(matches!(checked.warnings(), [DiskWarning::JoshCertificate(xecrypt::CertificateStatus::Invalid { .. })]));
	assert!(detection.security_sector().unwrap().is_signature_absent());
	let logo = detection.logo_bitmap().unwrap().unwrap();
	assert!(logo.is_png());
	assert_eq!(logo.image().len(), 0x100);
	insta::assert_snapshot!(report(&checked));
}

#[test]
fn built_in_memory_unit() {
	let mut image = SparseImage::new(0x20_0000 * 0x200);
	image.volume(0).volume(0x0800_0000).volume(0x1400_0000).volume(0x20e3_0000);

	insta::assert_snapshot!(report(&run(&image)));
}

#[test]
fn emmc_with_a_memory_unit() {
	let mut image = SparseImage::new(0x40_0000 * 0x200);
	image.volume(0x0300_0000).volume(0x0300_0000 + 0x20e3_0000);

	let checked = run(&image);

	assert_eq!(checked.value().kind(), DeviceKind::MmcMemoryUnit);
	assert!(checked.value().alternatives().is_empty());
}

#[test]
fn joined_usb_storage_files() {
	let layout = UsbStorageLayout::new(ByteCount::new(0x40_0000)).unwrap();
	let mut image = SparseImage::new(layout.logical_length().get());
	image.volume(0x400).volume(0x0800_0400).volume(0x1200_0400).volume(0x2000_0000);

	insta::assert_snapshot!(report(&run(&image)));
}

#[test]
fn zeros_are_unrecognized() {
	let image = SparseImage::new(0x1aca_0000);

	let error = detect(&image, ByteCount::new(image.length()), OPTIONS).unwrap_err();

	assert!(matches!(error, DetectError::Unrecognized { length } if length == ByteCount::new(0x1aca_0000)));
}

#[test]
fn a_failed_read_is_an_error() {
	let image = SparseImage::new(0x1000);

	let error = detect(&image, ByteCount::new(0x1aca_0000), OPTIONS).unwrap_err();

	assert!(matches!(error, DetectError::Read(_)));
}

#[test]
fn asynchronous_detection_agrees() {
	let mut image = SparseImage::new(u64::from(DISK_SECTORS) * 0x200);
	image.security_sector(DISK_SECTORS).volume(0x1_30eb_0000);

	let asynchronous =
		futures::executor::block_on(detect_async(&image, ByteCount::new(image.length()), OPTIONS)).unwrap();

	assert_eq!(asynchronous, run(&image));
	assert_eq!(asynchronous.value().map().sector_count(), u64::from(SectorCount::new(DISK_SECTORS).get()));
}
