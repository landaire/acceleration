//! The write commands, run through the executable on image files in a temporary directory.

#![cfg(unix)]

use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

const LENGTH: u64 = 0x0100_0000;

/// A directory, removed on drop, for a test's images.
struct Directory(PathBuf);

impl Directory {
	fn new(test: &str) -> Self {
		let path = std::env::temp_dir().join(format!("fatxtool-write-{test}-{}", std::process::id()));
		let _ = fs::remove_dir_all(&path);
		fs::create_dir(&path).unwrap();
		Self(path)
	}

	fn join(&self, name: &str) -> PathBuf {
		self.0.join(name)
	}

	/// A zeroed image file of `length` bytes.
	fn image(&self, name: &str, length: u64) -> PathBuf {
		let image = self.join(name);
		File::create(&image).unwrap().set_len(length).unwrap();
		image
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
	assert!(output.status.success(), "{arguments:?}: {}", String::from_utf8_lossy(&output.stderr));
	assert_eq!(String::from_utf8_lossy(&output.stderr), "", "{arguments:?}");
	String::from_utf8(output.stdout).unwrap()
}

/// Standard error of a run that must fail.
fn failure(arguments: &[&str]) -> String {
	let output = fatxtool(arguments);
	assert!(!output.status.success(), "{arguments:?} succeeded");
	String::from_utf8(output.stderr).unwrap()
}

fn path(path: &Path) -> &str {
	path.to_str().unwrap()
}

#[test]
fn format_then_change_then_read_back() {
	let directory = Directory::new("round-trip");
	let image = directory.image("volume.bin", LENGTH);
	let image = path(&image);
	let host = directory.join("host.wav");
	let mut data = b"RIFF".to_vec();
	data.extend(0x7ff8u32.to_le_bytes());
	data.extend((0..0x7ff8u32).map(|value| value as u8));
	fs::write(&host, &data).unwrap();

	assert_eq!(
		stdout(&[image, "format", "--serial", "0x1234"]),
		"formatted serial 00001234 with 1023 clusters of 0x4000 bytes and a Fat16 FAT\n"
	);
	assert_eq!(stdout(&[image, "mkdir", "/Content"]), "created /Content (0 bytes)\n");
	assert_eq!(stdout(&[image, "put", path(&host), "/Content/a.wav"]), "created /Content/a.wav (32768 bytes)\n");
	assert_eq!(stdout(&[image, "put", path(&host), "/content/A.WAV"]), "replaced /Content/a.wav (32768 bytes)\n");
	assert_eq!(stdout(&[image, "mv", "/Content/a.wav", "/b.wav"]), "moved /Content/a.wav to /b.wav\n");
	insta::assert_snapshot!(stdout(&[image, "list"]));

	let extracted = directory.join("extracted.wav");
	stdout(&[image, "extract", "/b.wav", path(&extracted)]);
	assert_eq!(fs::read(&extracted).unwrap(), data);

	assert_eq!(stdout(&[image, "rm", "/b.wav"]), "removed /b.wav (first cluster 0x3, 32768 bytes)\n");
	assert!(stdout(&[image, "list", "--deleted"]).contains("/b.wav"));
	let recovered = directory.join("recovered.wav");
	stdout(&[image, "recover", "/b.wav", path(&recovered)]);
	assert_eq!(fs::read(&recovered).unwrap(), data);

	let error = failure(&[image, "rm", "/Content/missing"]);
	assert!(error.contains("error: /Content/missing does not exist"), "{error}");
	let error = failure(&[image, "mkdir", "/a:b"]);
	assert!(error.contains("the name holds the illegal character ':'"), "{error}");
}

#[test]
fn format_sizes_the_fat_for_the_partition_length() {
	let directory = Directory::new("partition-length");
	// A 16 MiB partition with 4 KiB clusters: a 0x3000-byte FAT and a root cluster ending at
	// 0x5000.
	let image = directory.image("short.bin", 0x5000);
	let image = path(&image);
	let too_short = directory.image("shorter.bin", 0x4fff);

	let format = [image, "--partition-length", "0x1000000", "format", "--cluster-size", "0x1000", "--serial", "1"];
	assert_eq!(stdout(&format), "formatted serial 00000001 with 4092 clusters of 0x1000 bytes and a Fat16 FAT\n");
	let info = fatxtool(&[image, "--partition-length", "0x1000000", "info"]);
	assert!(info.status.success());
	assert!(String::from_utf8_lossy(&info.stdout).contains("partition length: 0x1000000\n"));

	let error = failure(&[path(&too_short), "--partition-length", "0x1000000", "format", "--cluster-size", "0x1000"]);
	assert!(error.contains("the volume needs 0x5000 bytes but the image range holds 0x4fff"), "{error}");
}

#[test]
fn alternate_partition_round_trip() {
	let directory = Directory::new("alternate");
	let image = directory.image("storage.bin", LENGTH);
	let image = path(&image);

	let error = failure(&[image, "--alternate-partition", "format", "--cluster-size", "0x1000"]);
	assert!(error.contains("an alternate partition needs"), "{error}");
	assert!(stdout(&[image, "--alternate-partition", "format"]).ends_with("a Fat32 FAT\n"));
	stdout(&[image, "--alternate-partition", "mkdir", "/Content"]);
	assert_eq!(stdout(&[image, "--alternate-partition", "list"]), "/Content/\n");
}

#[test]
fn format_keeps_an_existing_volume_without_force() {
	let directory = Directory::new("format-force");
	let image = directory.image("volume.bin", LENGTH);
	let image = path(&image);
	stdout(&[image, "format", "--serial", "1"]);
	stdout(&[image, "mkdir", "/keep"]);

	let error = failure(&[image, "format"]);
	assert!(error.contains("a volume already starts at"), "{error}");
	assert_eq!(stdout(&[image, "list"]), "/keep/\n");

	stdout(&[image, "format", "--force"]);
	assert_eq!(stdout(&[image, "list"]), "");
}

#[test]
fn writes_wait_for_force_while_checks_fail() {
	let directory = Directory::new("warnings");
	let image_path = directory.image("volume.bin", LENGTH);
	let image = path(&image_path);
	stdout(&[image, "format", "--serial", "1"]);
	stdout(&[image, "mkdir", "/bad"]);
	// Attribute 0x80 makes the driver skip /bad's record, the first of the root directory.
	OpenOptions::new().write(true).open(&image_path).unwrap().write_all_at(&[0x80], 0x2001).unwrap();

	let error = failure(&[image, "mkdir", "/new"]);
	assert!(error.contains("pass --force to change it anyway"), "{error}");
	assert!(error.contains("warning: record at cluster 0x1 offset 0x0 skipped"), "{error}");
	let error = failure(&[image, "--strict", "mkdir", "/new", "--force"]);
	assert!(error.contains("error: 2 integrity checks failed, the first: record at cluster 0x1"), "{error}");

	let output = fatxtool(&[image, "mkdir", "/new", "--force"]);
	assert!(output.status.success());
	assert_eq!(String::from_utf8_lossy(&output.stdout), "created /new (0 bytes)\n");
}

#[test]
fn writes_refuse_devices_links_and_directories() {
	let directory = Directory::new("refuse");
	let image = directory.image("volume.bin", LENGTH);
	stdout(&[path(&image), "format"]);
	let link = directory.join("link.bin");
	std::os::unix::fs::symlink(&image, &link).unwrap();

	for target in [Path::new("/dev/null"), link.as_path(), directory.0.as_path()] {
		let error = failure(&[path(target), "mkdir", "/x"]);
		assert!(error.contains("is not a regular file"), "{target:?}: {error}");
		let error = failure(&[path(target), "format", "--force"]);
		assert!(error.contains("is not a regular file"), "{target:?}: {error}");
	}

	let mut head = [0; 4];
	File::open(&image).unwrap().read_exact_at(&mut head, 0).unwrap();
	assert_eq!(&head, b"XTAF");
}
