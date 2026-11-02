//! The device-level commands, run through the executable on synthetic images: `partitions`,
//! `--partition`, a USB storage directory, and `partition-table write`.

// These tests build volumes with the shared builder but use none of its recovery helpers.
#[allow(dead_code)]
mod common;

use std::fs;
use std::fs::File;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

use common::Builder;
use common::Record;
use fatxtool::DiskToolError;
use fatxtool::usb_files::open_usb_storage;
use xenon_disk::DevkitEntry;
use xenon_disk::DevkitSlot;
use xenon_disk::DevkitTable;
use xenon_disk::Lba;
use xenon_disk::SectorCount;
use xenon_disk::UsbStorageError;
use xenon_disk::UsbStorageLayout;
use xenon_disk::devkit::DEVKIT_TABLE_MINIMUM;
use xenon_disk::usb_storage::MAXIMUM_FILE_COUNT;

/// The smallest USB memory unit: 0x6000 sectors.
const MEMORY_UNIT: u64 = 0xc0_0000;
const SYSTEM: u64 = 0x7f_f000;

/// A directory, removed on drop, for a test's images.
struct Directory(PathBuf);

impl Directory {
	fn new(test: &str) -> Self {
		let path = std::env::temp_dir().join(format!("fatxtool-disk-{test}-{}", std::process::id()));
		let _ = fs::remove_dir_all(&path);
		fs::create_dir(&path).unwrap();
		Self(path)
	}

	fn join(&self, name: &str) -> PathBuf {
		self.0.join(name)
	}
}

impl Drop for Directory {
	fn drop(&mut self) {
		let _ = fs::remove_dir_all(&self.0);
	}
}

fn fatxtool(arguments: &[&str]) -> Output {
	Command::new(env!("CARGO_BIN_EXE_fatxtool")).args(arguments).output().unwrap()
}

/// Standard output of a run that must succeed.
fn stdout(arguments: &[&str]) -> String {
	let output = fatxtool(arguments);
	assert!(output.status.success(), "{arguments:?} failed: {}", String::from_utf8_lossy(&output.stderr));

	String::from_utf8(output.stdout).unwrap()
}

/// A USB memory unit with a file in each partition.
fn memory_unit() -> Vec<u8> {
	let mut system = Builder::new(SYSTEM, SYSTEM);
	system.record(1, 0, Record::file("system.txt", 2, 6));
	system.file(&[2], b"system");

	let data_length = MEMORY_UNIT - SYSTEM;
	let mut data = Builder::new(data_length, data_length);
	data.record(1, 0, Record::file("data.txt", 2, 4));
	data.file(&[2], b"data");

	let mut image = system.finish();
	image.extend(data.finish());
	image
}

fn path(path: &Path) -> &str {
	path.to_str().unwrap()
}

#[test]
fn partitions_of_a_usb_memory_unit() {
	let directory = Directory::new("mu");
	let image = directory.join("mu.bin");
	fs::write(&image, memory_unit()).unwrap();

	insta::assert_snapshot!(stdout(&[path(&image), "partitions"]), @r"
	device: USB memory unit
	evidence: Mu0System at 0x0 starts with XTAF
	evidence: Mu0 at 0x7ff000 starts with XTAF
	length: 0xc00000 bytes, 0x6000 sectors of 0x200 bytes
	PARTITION                         START            END         LENGTH  VOLUME FLAGS
	Mu0System                           0x0       0x7ff000       0x7ff000  XTAF
	Mu0                            0x7ff000       0xc00000       0x401000  XTAF
	");
}

#[test]
fn partition_selects_a_volume_by_name() {
	let directory = Directory::new("select");
	let image = directory.join("mu.bin");
	fs::write(&image, memory_unit()).unwrap();

	assert_eq!(stdout(&[path(&image), "--partition", "mu0", "list"]), "/data.txt\n");
	assert_eq!(stdout(&[path(&image), "--partition", "Mu0System", "list"]), "/system.txt\n");

	let unknown = fatxtool(&[path(&image), "--partition", "Partition1", "list"]);
	assert!(!unknown.status.success());
	assert!(String::from_utf8_lossy(&unknown.stderr).contains("has no partition Partition1"));
}

#[test]
fn partitions_of_a_lone_volume() {
	let directory = Directory::new("lone");
	let image = directory.join("volume.bin");
	fs::write(&image, Builder::new(0x10_0000, 0x10_0000).finish()).unwrap();

	assert_eq!(
		stdout(&[path(&image), "partitions"]),
		"the image is one FATX volume of 0x100000 bytes, not a device with a partition layout\n"
	);
}

/// An alternate Storage volume of 16 KiB clusters and a 32-bit FAT over four data clusters,
/// written across the `Data####` files of `directory`, with /hello.txt in cluster 2.
fn usb_storage(directory: &Path) {
	const CLUSTER: usize = 0x4000;
	let layout = UsbStorageLayout::new(xenon_disk::ByteCount::new(4 * CLUSTER as u64)).unwrap();
	let metadata = layout.metadata_size().get() as usize;

	let mut storage = vec![0u8; metadata + 4 * CLUSTER];
	storage[..4].copy_from_slice(b"XTAF");
	storage[4..8].copy_from_slice(&0x1234_5678u32.to_be_bytes());
	storage[8..12].copy_from_slice(&32u32.to_be_bytes());
	storage[12..16].copy_from_slice(&1u32.to_be_bytes());
	for (cluster, value) in [(0, 0xffff_fff8u32), (1, u32::MAX), (2, u32::MAX)] {
		storage[0x1000 + cluster * 4..0x1000 + cluster * 4 + 4].copy_from_slice(&value.to_be_bytes());
	}
	storage[metadata..metadata + CLUSTER].fill(0xff);
	let mut record = [0xffu8; 0x40];
	record[0] = 9;
	record[1] = 0x20;
	record[2..11].copy_from_slice(b"hello.txt");
	record[0x2c..0x30].copy_from_slice(&2u32.to_be_bytes());
	record[0x30..0x34].copy_from_slice(&5u32.to_be_bytes());
	storage[metadata..metadata + 0x40].copy_from_slice(&record);
	storage[metadata + CLUSTER..metadata + CLUSTER + 5].copy_from_slice(b"hello");

	let reserved = File::create(directory.join("Data0000")).unwrap();
	reserved.set_len(0x2000_0000).unwrap();
	fs::write(directory.join("Data0001"), &storage[..metadata]).unwrap();
	fs::write(directory.join("Data0002"), &storage[metadata..]).unwrap();
}

#[test]
fn usb_storage_directory_mounts_storage_as_an_alternate_partition() {
	let directory = Directory::new("usb");
	usb_storage(&directory.0);

	let partitions = stdout(&[path(&directory.0), "partitions"]);
	assert!(partitions.starts_with("device: USB storage files\nevidence: Storage at 0x20000000 starts with XTAF\n"));
	assert_eq!(stdout(&[path(&directory.0), "--partition", "Storage", "list"]), "/hello.txt\n");
	let info = stdout(&[path(&directory.0), "--partition", "Storage", "info"]);
	assert!(info.contains("partition kind: alternate, in a growable file\n"), "{info}");
}

#[test]
fn partition_table_write_changes_only_sector_0() {
	let directory = Directory::new("table");
	let image = directory.join("disk.bin");
	let disk = File::create(&image).unwrap();
	disk.set_len(0x4000_0000).unwrap();
	let mut filler = &disk;
	filler.write_all(&[0x5a; 0x400]).unwrap();
	drop(disk);

	let report = stdout(&[
		path(&image),
		"partition-table",
		"write",
		"--minimum",
		"0x53080003",
		"SystemPartition=0x10000000",
		"Cache0=0x1000",
		"Partition1=rest",
	]);

	insta::assert_snapshot!(report, @r"
	length: 0x40000000 bytes, 0x200000 sectors of 0x200 bytes
	PARTITION                         START            END         LENGTH  VOLUME FLAGS
	PhysicalDisk                        0x0     0x40000000     0x40000000  -      raw-only
	Partition0                          0x0     0x40000000     0x40000000  -      raw-only
	SystemPartition                 0x80000     0x10080000     0x10000000  -
	Cache0                       0x10080000     0x10081000         0x1000  -
	Partition1                   0x10081000     0x40000000     0x2ff7f000  -
	");

	let mut head = [0u8; 0x400];
	File::open(&image).unwrap().read_exact(&mut head).unwrap();
	let table = DevkitTable::parse(head[..0x200].try_into().unwrap()).unwrap();
	assert_eq!(table.minimum(), 0x5308_0003);
	assert_eq!(table.assigned(DevkitSlot::Partition1).unwrap().start_offset().get(), 0x1008_1000);
	assert_eq!(head[0x200..0x400], [0x5a; 0x200]);
	assert_eq!(fs::metadata(&image).unwrap().len(), 0x4000_0000);

	let partitions = stdout(&[path(&image), "partitions"]);
	assert!(
		partitions.starts_with("device: development kit hard disk\nevidence: sector 0 holds a devkit partition table")
	);
}

#[test]
fn usb_storage_directory_with_too_many_files_is_rejected() {
	let directory = Directory::new("usb-many");
	for index in 0..=MAXIMUM_FILE_COUNT {
		File::create(directory.join(&UsbStorageLayout::file_name(index))).unwrap();
	}

	let error = open_usb_storage(&directory.0).unwrap_err();

	assert!(matches!(
		error,
		DiskToolError::UsbStorage { source: UsbStorageError::TooManyFiles { count, maximum }, .. }
			if count == MAXIMUM_FILE_COUNT + 1 && maximum == MAXIMUM_FILE_COUNT
	));
}

#[test]
fn partitions_prints_the_josh_block_logo_and_an_absent_signature() {
	let directory = Directory::new("josh");
	let image = directory.join("devkit.bin");

	let mut table = DevkitTable::new(DEVKIT_TABLE_MINIMUM).unwrap();
	table.set(DevkitSlot::Partition1, Some(DevkitEntry { start: Lba::new(0x400), count: SectorCount::new(0x1000) }));

	// A block whose certificate holds no usable key, so its console signature cannot verify.
	let mut josh = vec![0u8; 0x284];
	josh[..4].copy_from_slice(b"Josh");
	josh[0x04..0x06].copy_from_slice(&0x1a8u16.to_be_bytes());
	josh[0x06..0x0b].copy_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55]);
	josh[0x0b..0x16].copy_from_slice(b"X000000-001");
	josh[0x1c..0x20].copy_from_slice(&1u32.to_be_bytes());
	josh[0x20..0x28].copy_from_slice(b"01-02-03");
	josh[0x28..0x2c].copy_from_slice(&3u32.to_be_bytes());
	josh[0x250..0x254].copy_from_slice(&[0x24, 0xf0, 0x00, 0x01]);
	josh[0x274..0x278].copy_from_slice(&1u32.to_be_bytes());
	josh[0x278..0x27c].copy_from_slice(&1u32.to_be_bytes());
	josh[0x280..0x284].copy_from_slice(&0x4d53_0001u32.to_be_bytes());

	// The digest stays zero, so the logo check fails with a warning; the output is the same.
	let mut logo = vec![0u8; 0xc00];
	logo[..4].copy_from_slice(&0x10u32.to_be_bytes());
	logo[4..12].copy_from_slice(b"\x89PNG\r\n\x1a\n");
	let mut security = vec![0u8; 0x200];
	security[..0x14].copy_from_slice(b"SERIAL0001          ");
	security[0x14..0x1c].copy_from_slice(b"FW01    ");
	security[0x1c..0x24].copy_from_slice(b"MODEL001");
	security[0x58..0x5c].copy_from_slice(&0x98_7580u32.to_le_bytes());

	let mut file = File::create(&image).unwrap();
	file.set_len(0x4000_0000).unwrap();
	for (offset, bytes) in [(0, &table.to_bytes()[..]), (0x800, &josh), (0x2000, &security), (0x2200, &logo)] {
		file.seek(SeekFrom::Start(offset)).unwrap();
		file.write_all(bytes).unwrap();
	}
	drop(file);

	let report = stdout(&[path(&image), "partitions"]);
	let start = report.find("Josh block").unwrap();
	let end = report.find("length:").unwrap();
	insta::assert_snapshot!(&report[start..end], @r"
	Josh block at 0x800 (utility drive cache table of titles; not read by the kernel):
	  console id: 1122334455
	  console part number: X000000-001
	  console type: 0x00000001 (Devkit)
	  manufacturing date: 01-02-03
	  certificate: invalid: key 0x3c does not verify it
	  console signature: the certificate's public key with exponent 0x3 is not a usable RSA key
	  version: 1
	  last assigned cache: 1
	  Cache0: owner none, entry zero
	  Cache1: owner 4D530001, entry in use
	security sector:
	  serial number: SERIAL0001
	  firmware revision: FW01
	  model number: MODEL001
	  user addressable sectors: 0x987580
	  logo bitmap digest: 0000000000000000000000000000000000000000
	  logo: PNG image of 0x10 bytes
	  signature: absent (all zero)
	");
}

#[cfg(unix)]
#[test]
fn partition_table_write_refuses_other_files() {
	let directory = Directory::new("refuse");
	let image = directory.join("disk.bin");
	File::create(&image).unwrap().set_len(0x4000_0000).unwrap();
	let link = directory.join("link.bin");
	std::os::unix::fs::symlink(&image, &link).unwrap();

	for target in [Path::new("/dev/null"), link.as_path()] {
		let output = fatxtool(&[path(target), "partition-table", "write", "Partition1=rest"]);
		assert!(!output.status.success());
		assert!(String::from_utf8_lossy(&output.stderr).contains("is not a regular file"), "{target:?}");
	}

	let invalid = fatxtool(&[path(&image), "partition-table", "write", "DumpPartition=0x1000"]);
	assert!(String::from_utf8_lossy(&invalid.stderr).contains("DumpPartition of 0x1000 bytes cannot hold"));
	let mut head = [0xffu8; 0x200];
	File::open(&image).unwrap().read_exact(&mut head).unwrap();
	assert_eq!(head, [0; 0x200]);
}
