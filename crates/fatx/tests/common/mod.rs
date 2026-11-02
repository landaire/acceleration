//! Synthetic FATX volumes, laid out with the formulas of mountvol.c FatxProcessBootSector.

use std::collections::BTreeMap;
use std::io;
use std::ops::Range;

use fatx::ReadAt;

pub const PAGE: u64 = 0x1000;
pub const SECTOR: u64 = 0x200;
pub const SERIAL: u32 = 0x51ec_f2c0;
/// 2010-09-09 19:00:42.
pub const TIME: u32 = 0x3d29_9815;
pub const LAST: u32 = u32::MAX;

pub const DELETED: u8 = 0xe5;
pub const DIRECTORY: u8 = 0x10;
pub const ARCHIVE: u8 = 0x20;

/// A sparse image: unwritten pages read as zero.
#[derive(Debug, Clone)]
pub struct Image {
	length: u64,
	pages: BTreeMap<u64, Vec<u8>>,
}

impl Image {
	pub fn new(length: u64) -> Self {
		Self { length, pages: BTreeMap::new() }
	}

	pub fn write(&mut self, offset: u64, bytes: &[u8]) {
		assert!(offset + bytes.len() as u64 <= self.length, "write past the image");
		for (index, byte) in bytes.iter().enumerate() {
			let position = offset + index as u64;
			let page = self.pages.entry(position / PAGE).or_insert_with(|| vec![0; PAGE as usize]);
			page[(position % PAGE) as usize] = *byte;
		}
	}

	pub fn read(&self, range: Range<u64>) -> Vec<u8> {
		let mut bytes = vec![0; (range.end - range.start) as usize];
		let mut position = range.start;
		while position < range.end {
			let page_end = ((position / PAGE + 1) * PAGE).min(range.end);
			if let Some(page) = self.pages.get(&(position / PAGE)) {
				let source = &page[(position % PAGE) as usize..(page_end - (position / PAGE) * PAGE) as usize];
				let start = (position - range.start) as usize;
				bytes[start..start + source.len()].copy_from_slice(source);
			}
			position = page_end;
		}

		bytes
	}
}

impl ReadAt for Image {
	fn read_at(&self, _context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		if range.end > self.length {
			return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read past the image"));
		}

		Ok(self.read(range))
	}
}

/// A directory record to write.
#[derive(Debug, Clone, Copy)]
pub struct Record<'a> {
	pub name_length: u8,
	pub attributes: u8,
	pub name: &'a [u8],
	pub first_cluster: u32,
	pub size: u32,
}

impl<'a> Record<'a> {
	pub fn file(name: &'a str, first_cluster: u32, size: u32) -> Self {
		Self { name_length: name.len() as u8, attributes: ARCHIVE, name: name.as_bytes(), first_cluster, size }
	}

	pub fn directory(name: &'a str, first_cluster: u32) -> Self {
		Self { name_length: name.len() as u8, attributes: DIRECTORY, name: name.as_bytes(), first_cluster, size: 0 }
	}

	pub fn deleted(self) -> Self {
		Self { name_length: DELETED, ..self }
	}

	pub fn encode(&self) -> [u8; 0x40] {
		let mut bytes = [0xff; 0x40];
		bytes[0] = self.name_length;
		bytes[1] = self.attributes;
		bytes[2..2 + self.name.len()].copy_from_slice(self.name);
		bytes[0x2c..0x30].copy_from_slice(&self.first_cluster.to_be_bytes());
		bytes[0x30..0x34].copy_from_slice(&self.size.to_be_bytes());
		for offset in [0x34, 0x38, 0x3c] {
			bytes[offset..offset + 4].copy_from_slice(&TIME.to_be_bytes());
		}
		bytes
	}
}

#[derive(Debug, Clone)]
pub struct Builder {
	pub image: Image,
	pub cluster_size: u64,
	pub entry_size: u64,
	pub file_area: u64,
}

impl Builder {
	pub fn new(partition_length: u64, sectors_per_cluster: u32) -> Self {
		let entries = partition_length / (u64::from(sectors_per_cluster) * SECTOR) + 1;
		let entry_size = if entries < 0xfff0 { 2 } else { 4 };

		Self::with_entry_size(partition_length, sectors_per_cluster, entry_size)
	}

	/// A partition in a growable file: 16 KiB clusters and 32-bit FAT entries at any length.
	pub fn alternate(partition_length: u64) -> Self {
		Self::with_entry_size(partition_length, 0x20, 4)
	}

	fn with_entry_size(partition_length: u64, sectors_per_cluster: u32, entry_size: u64) -> Self {
		let cluster_size = u64::from(sectors_per_cluster) * SECTOR;
		let entries = partition_length / cluster_size + 1;
		let fat_length = (entries * entry_size).next_multiple_of(PAGE);

		let mut builder =
			Self { image: Image::new(partition_length), cluster_size, entry_size, file_area: PAGE + fat_length };
		let mut header = Vec::new();
		header.extend_from_slice(b"XTAF");
		header.extend_from_slice(&SERIAL.to_be_bytes());
		header.extend_from_slice(&sectors_per_cluster.to_be_bytes());
		header.extend_from_slice(&1u32.to_be_bytes());
		builder.image.write(0, &header);
		builder.set_fat(0, 0xffff_fff8);
		builder.directory(&[1]);
		builder
	}

	pub fn set_fat(&mut self, cluster: u32, value: u32) {
		let offset = PAGE + u64::from(cluster) * self.entry_size;
		if self.entry_size == 2 {
			self.image.write(offset, &(value as u16).to_be_bytes());
		} else {
			self.image.write(offset, &value.to_be_bytes());
		}
	}

	pub fn chain(&mut self, clusters: &[u32]) {
		for pair in clusters.windows(2) {
			self.set_fat(pair[0], pair[1]);
		}
		self.set_fat(*clusters.last().expect("a chain has a cluster"), LAST);
	}

	pub fn cluster_offset(&self, cluster: u32) -> u64 {
		self.file_area + u64::from(cluster - 1) * self.cluster_size
	}

	pub fn write_cluster(&mut self, cluster: u32, offset: u64, bytes: &[u8]) {
		assert!(offset + bytes.len() as u64 <= self.cluster_size);
		let start = self.cluster_offset(cluster) + offset;
		self.image.write(start, bytes);
	}

	/// Allocates a directory's chain and fills it with 0xff, as allocsup.c
	/// FatxInitializeDirectoryCluster does.
	pub fn directory(&mut self, clusters: &[u32]) {
		self.chain(clusters);
		for cluster in clusters {
			self.write_cluster(*cluster, 0, &vec![0xff; self.cluster_size as usize]);
		}
	}

	pub fn record(&mut self, cluster: u32, slot: u64, record: Record<'_>) {
		self.write_cluster(cluster, slot * 0x40, &record.encode());
	}

	/// Writes `data` across `clusters` without touching the FAT.
	pub fn data(&mut self, clusters: &[u32], data: &[u8]) {
		for (cluster, chunk) in clusters.iter().zip(data.chunks(self.cluster_size as usize)) {
			self.write_cluster(*cluster, 0, chunk);
		}
	}

	/// Writes a live file's data and chain.
	pub fn file(&mut self, clusters: &[u32], data: &[u8]) {
		self.chain(clusters);
		self.data(clusters, data);
	}
}

/// `length` bytes that differ from cluster to cluster.
pub fn pattern(seed: u8, length: usize) -> Vec<u8> {
	(0..length).map(|index| seed.wrapping_add((index / 7) as u8)).collect()
}

/// A Bink header declaring `size` bytes, followed by filler.
pub fn bink(size: usize) -> Vec<u8> {
	let mut data = b"BIKi".to_vec();
	data.extend_from_slice(&(size as u32 - 8).to_le_bytes());
	data.extend(pattern(0x40, size - 8));
	data
}

/// A RIFF header declaring `size` bytes, followed by filler.
pub fn riff(size: usize) -> Vec<u8> {
	let mut data = b"RIFF".to_vec();
	data.extend_from_slice(&(size as u32 - 8).to_le_bytes());
	data.extend(pattern(0x10, size - 8));
	data
}
