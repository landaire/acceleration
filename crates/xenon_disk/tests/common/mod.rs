// Each test binary uses part of these helpers.
#![allow(dead_code)]

//! Sparse synthetic devices: zeros except for the bytes a test places, so that layouts of
//! hundreds of gigabytes cost only the bytes they hold.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::io;
use std::ops::Range;

use sha1::Digest;
use sha1::Sha1;
use xenon_disk::PartitionMap;
use xenon_disk::ReadAt;
use xenon_disk::VolumeKind;

#[derive(Debug, Clone)]
pub struct SparseImage {
	length: u64,
	regions: BTreeMap<u64, Vec<u8>>,
}

impl SparseImage {
	pub fn new(length: u64) -> Self {
		Self { length, regions: BTreeMap::new() }
	}

	pub fn length(&self) -> u64 {
		self.length
	}

	pub fn write(&mut self, offset: u64, bytes: &[u8]) -> &mut Self {
		assert!(offset + bytes.len() as u64 <= self.length, "region past the image");
		self.regions.insert(offset, bytes.to_vec());
		self
	}

	pub fn volume(&mut self, offset: u64) -> &mut Self {
		self.write(offset, b"XTAF")
	}

	/// A security sector at 0x2000 that names a drive and limits it to `sectors`, followed by a
	/// logo region whose digest it holds: a 0x100-byte image with the PNG signature after a
	/// big-endian length.
	pub fn security_sector(&mut self, sectors: u32) -> &mut Self {
		let mut logo = vec![0u8; 0xc00];
		logo[..4].copy_from_slice(&0x100u32.to_be_bytes());
		logo[4..12].copy_from_slice(b"\x89PNG\r\n\x1a\n");
		for (index, byte) in logo[12..0x104].iter_mut().enumerate() {
			*byte = (index % 251) as u8;
		}

		let mut sector = vec![0u8; 0x200];
		sector[0x00..0x14].copy_from_slice(b"            5VJ1AB2C");
		sector[0x14..0x1c].copy_from_slice(b"JC4OHE  ");
		sector[0x1c..0x44].copy_from_slice(b"ST9250315AS                             ");
		let digest: [u8; 20] = Sha1::digest(&logo).into();
		sector[0x44..0x58].copy_from_slice(&digest);
		sector[0x58..0x5c].copy_from_slice(&sectors.to_le_bytes());
		sector[0x5c..0x15c].fill(0xa5);

		self.write(0x2000, &sector);
		self.write(0x2200, &logo)
	}

	/// Zeros the signature of the security sector.
	pub fn unsigned(&mut self) -> &mut Self {
		self.write(0x205c, &[0; 0x100])
	}

	/// A Josh block at 0x800 with made-up field values.
	pub fn josh_block(&mut self) -> &mut Self {
		self.write(0x800, &josh_block())
	}
}

/// The primes of a made-up RSA-1024 console key with exponent 3.
const CONSOLE_KEY_P: &str = "fb89a051aeab79f763a85d81fe420eee3b5f42693ffd7a15cb7fbeb21e8c415940c598a39643757dd8529c904f3f10c8246eff231d648a67a3e6a121c5e3d417";
const CONSOLE_KEY_Q: &str = "e998ed26189daf65b93d6a607086db23770b72ce031a41b2abfe972ea7907ce269332b4fe9a37abdc17d6341206126b22dc4f64fc70b0939e1eebb9a85bc8ec3";

/// The 0x284 bytes of a Josh block with made-up field values: console id 1122334455, part
/// number X000000-001, Cache1 owned by title 4D530001, and a console signature over the cache
/// table made with [`CONSOLE_KEY_P`] and [`CONSOLE_KEY_Q`].
pub fn josh_block() -> Vec<u8> {
	let mut block = vec![0u8; 0x284];
	block[..4].copy_from_slice(b"Josh");

	let key = console_key();
	let certificate = &mut block[0x04..0x1ac];
	certificate[0x00..0x02].copy_from_slice(&0x1a8u16.to_be_bytes());
	certificate[0x02..0x07].copy_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x55]);
	certificate[0x07..0x12].copy_from_slice(b"X000000-001");
	certificate[0x16..0x18].copy_from_slice(&1u16.to_be_bytes());
	certificate[0x18..0x1c].copy_from_slice(&1u32.to_be_bytes());
	certificate[0x1c..0x24].copy_from_slice(b"01-02-03");
	certificate[0x24..0x28].copy_from_slice(&3u32.to_be_bytes());
	let modulus = rsa::traits::PublicKeyParts::n(&key).to_bytes_be();
	// XeCrypt stores 64-bit big-endian words, least significant first.
	let words = modulus.as_chunks::<8>().0.iter().rev();
	for (target, source) in certificate[0x28..0xa8].as_chunks_mut::<8>().0.iter_mut().zip(words) {
		*target = *source;
	}

	let table = &mut block[0x22c..];
	table[0x24..0x28].copy_from_slice(&[0x24, 0xf0, 0x00, 0x01]);
	table[0x2c..0x40].fill(0x5a);
	table[0x40..0x44].copy_from_slice(&4u32.to_be_bytes());
	table[0x44..0x48].copy_from_slice(&2u32.to_be_bytes());
	table[0x48..0x4c].copy_from_slice(&1u32.to_be_bytes());
	table[0x4c..0x50].copy_from_slice(&1u32.to_be_bytes());
	table[0x54..0x58].copy_from_slice(&0x4d53_0001u32.to_be_bytes());

	let digest = Sha1::digest(&block[0x22c..]);
	let prefix = [0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04, 0x14];
	let scheme = rsa::Pkcs1v15Sign { hash_len: Some(20), prefix: prefix[..].into() };
	let mut signature = key.sign(scheme, &digest).unwrap();
	signature.reverse();
	block[0x1ac..0x22c].copy_from_slice(&signature);

	block
}

fn console_key() -> rsa::RsaPrivateKey {
	let prime = |hex: &str| rsa::BigUint::parse_bytes(hex.as_bytes(), 16).unwrap();

	rsa::RsaPrivateKey::from_p_q(prime(CONSOLE_KEY_P), prime(CONSOLE_KEY_Q), rsa::BigUint::from(3u32)).unwrap()
}

impl ReadAt for SparseImage {
	fn read_at(&self, _context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		if range.end > self.length || range.start > range.end {
			return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "past the image"));
		}

		let mut bytes = vec![0; (range.end - range.start) as usize];
		for (offset, region) in &self.regions {
			let start = range.start.max(*offset);
			let end = range.end.min(offset + region.len() as u64);
			if start < end {
				bytes[(start - range.start) as usize..(end - range.start) as usize]
					.copy_from_slice(&region[(start - offset) as usize..(end - offset) as usize]);
			}
		}

		Ok(bytes)
	}
}

/// One line per partition: name, byte range, length and flags.
pub fn render(map: &PartitionMap) -> String {
	let mut text = format!("{} of {:#x} bytes ({:#x} sectors)\n", map.device(), map.length().get(), map.sector_count());
	for partition in map.partitions() {
		let range = partition.range();
		let mut flags = Vec::new();
		if partition.is_raw_only() {
			flags.push("raw");
		}
		if partition.volume() == VolumeKind::Alternate {
			flags.push("alternate");
		}
		let line = format!(
			"{:<24} {:#013x}..{:#013x} {:#013x} {}",
			partition.name(),
			range.start,
			range.end,
			range.end - range.start,
			flags.join(",")
		);
		writeln!(text, "{}", line.trim_end()).unwrap();
	}

	text
}
