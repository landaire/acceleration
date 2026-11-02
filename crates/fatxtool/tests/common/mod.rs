//! Synthetic FATX volumes, laid out with the formulas of mountvol.c FatxProcessBootSector, and
//! a scratch directory that holds a test's input and outputs.

use std::fs::File;
use std::path::Path;
use std::path::PathBuf;

use clap::Parser;
use fatx::IntegrityPolicy;
use fatx::io::SliceSource;
use fatxtool::FatxCommand;
use fatxtool::FatxToolError;
use fatxtool::InputFile;
use fatxtool::Session;
use fatxtool::VolumeLocation;

const PAGE: u64 = 0x1000;
const SECTOR: u64 = 0x200;
const SECTORS_PER_CLUSTER: u32 = 8;
const SERIAL: u32 = 0x51ec_f2c0;
/// 2010-09-09 19:00:42.
const TIME: u32 = 0x3d29_9815;
const LAST: u32 = u32::MAX;

const DELETED: u8 = 0xe5;
const DIRECTORY: u8 = 0x10;
const ARCHIVE: u8 = 0x20;

/// A directory record to write.
#[derive(Debug, Clone, Copy)]
pub struct Record<'a> {
	name: &'a str,
	attributes: u8,
	first_cluster: u32,
	size: u32,
	deleted: bool,
}

impl<'a> Record<'a> {
	pub fn file(name: &'a str, first_cluster: u32, size: u32) -> Self {
		Self { name, attributes: ARCHIVE, first_cluster, size, deleted: false }
	}

	pub fn directory(name: &'a str, first_cluster: u32) -> Self {
		Self { name, attributes: DIRECTORY, first_cluster, size: 0, deleted: false }
	}

	pub fn deleted(self) -> Self {
		Self { deleted: true, ..self }
	}

	fn encode(&self) -> [u8; 0x40] {
		let mut bytes = [0xff; 0x40];
		bytes[0] = if self.deleted { DELETED } else { self.name.len() as u8 };
		bytes[1] = self.attributes;
		bytes[2..2 + self.name.len()].copy_from_slice(self.name.as_bytes());
		bytes[0x2c..0x30].copy_from_slice(&self.first_cluster.to_be_bytes());
		bytes[0x30..0x34].copy_from_slice(&self.size.to_be_bytes());
		for offset in [0x34, 0x38, 0x3c] {
			bytes[offset..offset + 4].copy_from_slice(&TIME.to_be_bytes());
		}
		bytes
	}
}

/// The first `image_length` bytes of a volume whose partition is `partition_length` bytes,
/// with 4 KiB clusters and the root directory in cluster 1.
#[derive(Debug, Clone)]
pub struct Builder {
	image: Vec<u8>,
	cluster_size: u64,
	entry_size: u64,
	file_area: u64,
}

impl Builder {
	pub fn new(partition_length: u64, image_length: u64) -> Self {
		let cluster_size = u64::from(SECTORS_PER_CLUSTER) * SECTOR;
		let entries = partition_length / cluster_size + 1;
		let entry_size = if entries < 0xfff0 { 2 } else { 4 };
		let fat_length = (entries * entry_size).next_multiple_of(PAGE);

		let mut builder =
			Self { image: vec![0; image_length as usize], cluster_size, entry_size, file_area: PAGE + fat_length };
		let mut header = Vec::new();
		header.extend_from_slice(b"XTAF");
		header.extend_from_slice(&SERIAL.to_be_bytes());
		header.extend_from_slice(&SECTORS_PER_CLUSTER.to_be_bytes());
		header.extend_from_slice(&1u32.to_be_bytes());
		builder.write(0, &header);
		builder.set_fat(0, 0xffff_fff8);
		builder.directory(&[1]);
		builder
	}

	fn write(&mut self, offset: u64, bytes: &[u8]) {
		let start = offset as usize;
		self.image[start..start + bytes.len()].copy_from_slice(bytes);
	}

	fn set_fat(&mut self, cluster: u32, value: u32) {
		let offset = PAGE + u64::from(cluster) * self.entry_size;
		if self.entry_size == 2 {
			self.write(offset, &(value as u16).to_be_bytes());
		} else {
			self.write(offset, &value.to_be_bytes());
		}
	}

	fn chain(&mut self, clusters: &[u32]) {
		for pair in clusters.windows(2) {
			self.set_fat(pair[0], pair[1]);
		}
		self.set_fat(*clusters.last().expect("a chain has a cluster"), LAST);
	}

	fn cluster_offset(&self, cluster: u32) -> u64 {
		self.file_area + u64::from(cluster - 1) * self.cluster_size
	}

	/// Allocates a directory's chain and fills it with 0xff, as allocsup.c
	/// FatxInitializeDirectoryCluster does.
	pub fn directory(&mut self, clusters: &[u32]) {
		self.chain(clusters);
		for cluster in clusters {
			let offset = self.cluster_offset(*cluster);
			self.write(offset, &vec![0xff; self.cluster_size as usize]);
		}
	}

	pub fn record(&mut self, cluster: u32, slot: u64, record: Record<'_>) {
		let offset = self.cluster_offset(cluster) + slot * 0x40;
		self.write(offset, &record.encode());
	}

	/// Writes `data` across consecutive clusters from `first` without touching the FAT.
	pub fn data(&mut self, first: u32, data: &[u8]) {
		let offset = self.cluster_offset(first);
		self.write(offset, data);
	}

	/// Writes a live file's data and chain.
	pub fn file(&mut self, clusters: &[u32], data: &[u8]) {
		self.chain(clusters);
		for (cluster, chunk) in clusters.iter().zip(data.chunks(self.cluster_size as usize)) {
			self.data(*cluster, chunk);
		}
	}

	pub fn finish(self) -> Vec<u8> {
		self.image
	}
}

/// A Bink header declaring `size` bytes, followed by filler.
pub fn bink(size: usize) -> Vec<u8> {
	let mut data = b"BIKi".to_vec();
	data.extend_from_slice(&(size as u32 - 8).to_le_bytes());
	data.extend((0..size - 8).map(|index| (index / 7) as u8));
	data
}

/// A directory, removed on drop, that holds a test's input image and outputs.
#[derive(Debug)]
pub struct Scratch {
	directory: PathBuf,
	image: Vec<u8>,
	input: InputFile,
}

impl Scratch {
	pub fn new(test: &str, image: Vec<u8>) -> Self {
		let directory = std::env::temp_dir().join(format!("fatxtool-{test}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&directory);
		std::fs::create_dir(&directory).unwrap();
		let image_path = directory.join("image.bin");
		std::fs::write(&image_path, &image).unwrap();
		let input = InputFile::new(&File::open(&image_path).unwrap(), &image_path).unwrap();

		Self { directory, image, input }
	}

	pub fn path(&self, name: &str) -> PathBuf {
		self.directory.join(name)
	}

	pub fn image_path(&self) -> &Path {
		self.input.path()
	}

	/// Runs a command line on the whole image, with the partition length inferred, and returns
	/// what it printed to standard output.
	pub fn run(&self, arguments: &[&str]) -> Result<String, FatxToolError> {
		#[derive(Parser)]
		struct Cli {
			#[command(subcommand)]
			command: FatxCommand,
		}

		let command =
			Cli::try_parse_from(std::iter::once("fatxtool").chain(arguments.iter().copied())).unwrap().command;
		let source = SliceSource(self.image.as_slice());
		let location = VolumeLocation::infer(&source, 0..self.image.len() as u64)?;
		let session = Session::new("fatxtool", IntegrityPolicy::Ignore, &self.input);
		let mut out = Vec::new();
		fatxtool::run(&command, &source, &location, &session, &mut out)?;

		let report = String::from_utf8(out).unwrap().replace(self.directory.to_str().unwrap(), "SCRATCH");
		// Host paths in the report use the platform separator; snapshots use `/`.
		Ok(if cfg!(windows) { report.replace('\\', "/") } else { report })
	}
}

impl Drop for Scratch {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.directory);
	}
}
