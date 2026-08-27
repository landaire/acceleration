//! XEX import library parsing.
//!
//! The `ImportLibraries` optional header (key `0x000103FF`) contains a list
//! of libraries this XEX depends on. Each library has a name (e.g.
//! `xboxkrnl.exe`, `xam.xex`), a SHA-1 digest, a version, and a table of
//! import records. Import records are raw 32-bit values -- alternating pairs
//! of IAT and thunk addresses.
//!
//! Use [`Xex2Header::import_table`][crate::header::Xex2Header::import_table]
//! to get the parsed table.

use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use byteorder::WriteBytesExt;
#[cfg(feature = "serde")]
use serde::Serialize;
use std::io::Cursor;
use std::io::Read;

use crate::header::OptionalHeaderKey;
use crate::header::Xex2Header;
use xenon_types::Sha1Hash;
use xenon_types::Version;

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct ImportLibrary {
	pub name: String,
	pub digest: Sha1Hash,
	pub import_id: u32,
	pub version: Version,
	pub version_min: Version,
	pub records: Vec<u32>,
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct ImportTable {
	pub libraries: Vec<ImportLibrary>,
}

impl Xex2Header {
	pub fn import_table(&self) -> Option<ImportTable> {
		let data = self.get_optional_data(OptionalHeaderKey::ImportLibraries)?;
		parse_import_table(data)
	}
}

fn parse_import_table(data: &[u8]) -> Option<ImportTable> {
	if data.len() < 12 {
		return None;
	}
	let mut c = Cursor::new(data);
	let _total_size = c.read_u32::<BigEndian>().ok()?;
	let string_table_size = c.read_u32::<BigEndian>().ok()? as usize;
	let library_count = c.read_u32::<BigEndian>().ok()? as usize;

	let str_start = 12;
	let str_data = &data[str_start..str_start + string_table_size];
	let names: Vec<String> = str_data
		.split(|b| *b == 0)
		.filter(|s| !s.is_empty())
		.map(|s| String::from_utf8_lossy(s).into_owned())
		.collect();

	let mut lib_offset = str_start + string_table_size;
	if !lib_offset.is_multiple_of(4) {
		lib_offset += 4 - (lib_offset % 4);
	}

	let mut libraries = Vec::with_capacity(library_count);

	for _ in 0..library_count {
		if lib_offset + 40 > data.len() {
			break;
		}
		let mut c = Cursor::new(&data[lib_offset..]);
		let entry_size = c.read_u32::<BigEndian>().ok()? as usize;
		let mut digest_bytes = [0u8; 20];
		c.read_exact(&mut digest_bytes).ok()?;
		let digest = Sha1Hash(digest_bytes);
		let import_id = c.read_u32::<BigEndian>().ok()?;
		let version = Version::from(c.read_u32::<BigEndian>().ok()?);
		let version_min = Version::from(c.read_u32::<BigEndian>().ok()?);
		let name_index = c.read_u16::<BigEndian>().ok()? as usize;
		let record_count = c.read_u16::<BigEndian>().ok()? as usize;

		let name = match names.get(name_index) {
			Some(n) => n.clone(),
			None => return None,
		};

		let mut records = Vec::with_capacity(record_count);
		for _ in 0..record_count {
			records.push(c.read_u32::<BigEndian>().ok()?);
		}

		libraries.push(ImportLibrary { name, digest, import_id, version, version_min, records });

		lib_offset += entry_size;
	}

	Some(ImportTable { libraries })
}

/// Serialize an `ImportLibraries` optional-header blob (key `0x000103FF`).
///
/// Layout is the inverse of [`parse_import_table`]: a 12-byte header
/// (`total_size`, `string_table_size`, `library_count`), a NUL-terminated
/// name table padded to 4 bytes, then one variable-length record per library.
/// Each library's `name_index` is its position in `libraries`.
pub fn serialize_import_libraries(libraries: &[ImportLibrary]) -> Vec<u8> {
	let mut strings: Vec<u8> = Vec::new();
	for lib in libraries {
		strings.extend_from_slice(lib.name.as_bytes());
		strings.push(0);
	}
	while !strings.len().is_multiple_of(4) {
		strings.push(0);
	}

	let mut lib_blobs: Vec<u8> = Vec::new();
	for (index, lib) in libraries.iter().enumerate() {
		let entry_size = 0x28 + lib.records.len() * 4;
		lib_blobs.write_u32::<BigEndian>(entry_size as u32).unwrap();
		lib_blobs.extend_from_slice(&lib.digest.0);
		lib_blobs.write_u32::<BigEndian>(lib.import_id).unwrap();
		lib_blobs.write_u32::<BigEndian>(u32::from(lib.version)).unwrap();
		lib_blobs.write_u32::<BigEndian>(u32::from(lib.version_min)).unwrap();
		lib_blobs.write_u16::<BigEndian>(index as u16).unwrap();
		lib_blobs.write_u16::<BigEndian>(lib.records.len() as u16).unwrap();
		for &record in &lib.records {
			lib_blobs.write_u32::<BigEndian>(record).unwrap();
		}
	}

	let total_size = 12 + strings.len() + lib_blobs.len();
	let mut out: Vec<u8> = Vec::with_capacity(total_size);
	out.write_u32::<BigEndian>(total_size as u32).unwrap();
	out.write_u32::<BigEndian>(strings.len() as u32).unwrap();
	out.write_u32::<BigEndian>(libraries.len() as u32).unwrap();
	out.extend_from_slice(&strings);
	out.extend_from_slice(&lib_blobs);
	out
}
