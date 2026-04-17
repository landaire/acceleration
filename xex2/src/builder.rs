//! Build an XEX file from scratch.
//!
//! [`Xex2Builder`] assembles a minimal, valid XEX2 layout around a user-supplied
//! PE image. The result is uncompressed and unencrypted, signed with the devkit
//! PIRS private key.
//!
//! **Scope**: the builder covers the common "devkit-style" case only --
//! no compression, no encryption, no delta patches. Page descriptors and
//! `image_hash` use the kernel-verified formula from [`crate::page_descriptors`].
//!
//! # Example
//!
//! ```no_run
//! use xex2::builder::Xex2Builder;
//! use xenon_types::{TitleId, VirtualAddress};
//!
//! let pe = std::fs::read("game.pe").unwrap();
//! let bytes = Xex2Builder::new(pe)
//!     .title_id(TitleId(0x4D530914))
//!     .load_address(VirtualAddress(0x82000000))
//!     .build()
//!     .unwrap();
//! std::fs::write("game.xex", bytes).unwrap();
//! ```

use crate::crypto;
use crate::error::Result;
use crate::error::Xex2Error;
use crate::hashes;
use crate::header::EncryptionType;
use crate::header::OptionalHeaderKey;
use crate::opt::ImageFlags;
use crate::opt::ModuleFlags;
use crate::page_descriptors;
use byteorder::BigEndian;
use byteorder::ByteOrder;
use rootcause::IntoReport;
use xenon_types::MediaId;
use xenon_types::TitleId;
use xenon_types::Version;
use xenon_types::VirtualAddress;

/// Build a valid, devkit-signed XEX from a PE image + metadata.
pub struct Xex2Builder {
	pe: Vec<u8>,
	module_flags: ModuleFlags,
	image_flags: ImageFlags,
	load_address: VirtualAddress,
	title_id: TitleId,
	media_id: MediaId,
	version: Version,
	base_version: Version,
	entry_point: Option<VirtualAddress>,
	/// If `Some(window_size_bytes)`, the builder LZX-compresses `pe` and emits
	/// a Normal-compressed stream. Otherwise the PE is written uncompressed.
	compress_window: Option<u32>,
}

impl Xex2Builder {
	pub fn new(pe: Vec<u8>) -> Self {
		Self {
			pe,
			module_flags: ModuleFlags::TITLE,
			image_flags: ImageFlags::empty(),
			load_address: VirtualAddress(0x82000000),
			title_id: TitleId(0),
			media_id: MediaId(0),
			version: Version::from(0),
			base_version: Version::from(0),
			entry_point: None,
			compress_window: None,
		}
	}

	/// Emit an LZX-compressed (Normal) XEX using the default 64 KB window,
	/// which matches what most shipping XEX files use. Call
	/// [`Self::compress_with`] to pick a different window size.
	pub fn compress(self) -> Self {
		self.compress_with(lzxc::WindowSize::KB64)
	}

	/// Emit an LZX-compressed (Normal) XEX with an explicit window size.
	pub fn compress_with(mut self, window: lzxc::WindowSize) -> Self {
		self.compress_window = Some(window.bytes());
		self
	}

	pub fn module_flags(mut self, flags: ModuleFlags) -> Self {
		self.module_flags = flags;
		self
	}

	pub fn image_flags(mut self, flags: ImageFlags) -> Self {
		self.image_flags = flags;
		self
	}

	pub fn load_address(mut self, addr: VirtualAddress) -> Self {
		self.load_address = addr;
		self
	}

	pub fn title_id(mut self, id: TitleId) -> Self {
		self.title_id = id;
		self
	}

	pub fn media_id(mut self, id: MediaId) -> Self {
		self.media_id = id;
		self
	}

	pub fn version(mut self, version: Version) -> Self {
		self.version = version;
		self
	}

	pub fn base_version(mut self, version: Version) -> Self {
		self.base_version = version;
		self
	}

	pub fn entry_point(mut self, addr: VirtualAddress) -> Self {
		self.entry_point = Some(addr);
		self
	}

	pub fn build(self) -> Result<Vec<u8>> {
		build_inner(self)
	}
}

// XEX layout constants.
const MAGIC: &[u8; 4] = b"XEX2";
const OPT_INDEX_START: usize = 0x18;
const PAGE_ALIGN: usize = 0x1000;

fn build_inner(b: Xex2Builder) -> Result<Vec<u8>> {
	// Optional headers we emit (in order):
	// - 0x00040006 ExecutionInfo (0x18 bytes of data)
	// - 0x000103FF ImportLibraries (variable, empty table)
	// - 0x000003FF FileFormatInfo (variable, "none/none")
	// - 0x00010100 EntryPoint (inline u32) -- if provided
	//
	// Each entry is keyed by (key_u32, value_u32). The low byte of the key
	// encodes the size class:
	//   - 0x00 / 0x01: inline (value IS the data)
	//   - 0xFF:        variable-length (value is a file offset to u32 size + body)
	//   - other:       (N * 4) bytes (value is a file offset)

	// If compression is requested, compress the PE up front so we can stitch
	// the first_block_hash into the FileFormatInfo blob. The data region
	// written later is either `b.pe` (uncompressed) or `stream.data`.
	let compressed_stream: Option<crate::compress::CompressedStream> = match b.compress_window {
		Some(window) => Some(crate::compress::compress_normal(&b.pe, window)?),
		None => None,
	};

	// Build optional-header data blobs we'll need to place in the file.
	let exec_info = execution_info_bytes(&b);
	let import_libs = empty_import_libraries_bytes();
	let file_format = match &compressed_stream {
		Some(stream) => crate::compress::file_format_info_blob_normal(EncryptionType::None, stream),
		None => file_format_info_bytes(),
	};

	// Compute file layout:
	//   0x00..0x18: main header
	//   0x18..:     optional header index (8 bytes per entry)
	//   data blobs: ExecutionInfo, ImportLibraries, FileFormatInfo
	//   security_info
	//   pad to page_align
	//   PE data
	let mut entries: Vec<OptEntry> = Vec::new();
	let mut blob_bytes: Vec<BlobPlacement> = Vec::new();

	// Calculate where blobs will land. Start placing them right after the
	// optional-header index.
	let entry_count = 3 + b.entry_point.is_some() as usize;
	let mut cursor = OPT_INDEX_START + entry_count * 8;

	// ExecutionInfo: fixed-size 0x18 bytes, key has size_class = 0x18/4 = 0x06.
	let exec_info_off = cursor;
	entries.push(OptEntry::Data { key: OptionalHeaderKey::ExecutionInfo as u32, offset: exec_info_off as u32 });
	blob_bytes.push(BlobPlacement { offset: exec_info_off, bytes: exec_info.clone() });
	cursor += exec_info.len();
	cursor = align_up(cursor, 4);

	// ImportLibraries: variable-length (size_class 0xFF).
	let import_libs_off = cursor;
	entries.push(OptEntry::Data { key: OptionalHeaderKey::ImportLibraries as u32, offset: import_libs_off as u32 });
	blob_bytes.push(BlobPlacement { offset: import_libs_off, bytes: import_libs.clone() });
	cursor += import_libs.len();
	cursor = align_up(cursor, 4);

	// FileFormatInfo: variable-length (size_class 0xFF).
	let file_format_off = cursor;
	entries.push(OptEntry::Data { key: OptionalHeaderKey::FileFormatInfo as u32, offset: file_format_off as u32 });
	blob_bytes.push(BlobPlacement { offset: file_format_off, bytes: file_format.clone() });
	cursor += file_format.len();
	cursor = align_up(cursor, 4);

	// EntryPoint: inline value.
	if let Some(entry) = b.entry_point {
		entries.push(OptEntry::Inline { key: OptionalHeaderKey::EntryPoint as u32, value: entry.0 });
	}

	// Page descriptors: we need to know image_size = pe.len(). One descriptor
	// covering the whole image with FLAG_HASHED.
	let page_size: u32 = if b.image_flags.contains(ImageFlags::SMALL_PAGES) { 0x1000 } else { 0x10000 };
	let page_descriptors::GeneratedDescriptors { descriptors, image_hash } =
		page_descriptors::generate(&b.pe, page_size, None);

	// security_info: fixed 0x184 + descriptors*24 bytes.
	let security_offset = cursor;
	let security_info_len = 0x184 + descriptors.len() * 24;
	cursor += security_info_len;

	// PE data at page-aligned offset. When compressed, we write the
	// compressed stream in place of `b.pe`.
	let data_region: &[u8] = compressed_stream.as_ref().map_or(b.pe.as_slice(), |s| s.data.as_slice());
	let data_offset = align_up(cursor, PAGE_ALIGN);
	let total_size = data_offset + data_region.len();

	// Assemble the file.
	let mut out = vec![0u8; total_size];

	// XEX main header.
	out[0..4].copy_from_slice(MAGIC);
	BigEndian::write_u32(&mut out[0x04..0x08], b.module_flags.bits());
	BigEndian::write_u32(&mut out[0x08..0x0C], data_offset as u32);
	// +0x0C: reserved (already zero)
	BigEndian::write_u32(&mut out[0x10..0x14], security_offset as u32);
	BigEndian::write_u32(&mut out[0x14..0x18], entry_count as u32);

	// Optional header index entries.
	for (i, entry) in entries.iter().enumerate() {
		let off = OPT_INDEX_START + i * 8;
		let (key, value) = match *entry {
			OptEntry::Inline { key, value } => (key, value),
			OptEntry::Data { key, offset } => (key, offset),
		};
		BigEndian::write_u32(&mut out[off..off + 4], key);
		BigEndian::write_u32(&mut out[off + 4..off + 8], value);
	}

	// Optional header data blobs.
	for blob in &blob_bytes {
		out[blob.offset..blob.offset + blob.bytes.len()].copy_from_slice(&blob.bytes);
	}

	// security_info header_size (at security_offset + 0x00) and image_size.
	BigEndian::write_u32(&mut out[security_offset..security_offset + 4], security_info_len as u32);
	BigEndian::write_u32(&mut out[security_offset + 0x04..security_offset + 0x08], b.pe.len() as u32);
	// RSA signature placeholder at security_offset + 0x08..0x108 (filled after signing).

	// image_info at security_offset + 0x108, 0x74 bytes.
	let ii_start = security_offset + 0x108;
	BigEndian::write_u32(&mut out[ii_start..ii_start + 0x04], 0x174); // info_size
	BigEndian::write_u32(&mut out[ii_start + 0x04..ii_start + 0x08], b.image_flags.bits()); // image_flags
	BigEndian::write_u32(&mut out[ii_start + 0x08..ii_start + 0x0C], b.load_address.0); // load_address
	out[ii_start + 0x0C..ii_start + 0x20].copy_from_slice(&*image_hash);
	// import_table_count = 0 (we emit an empty import table)
	// import_table_hash left zero
	// media_id left zero
	// file_key left zero (encryption None)
	// export_table_address left zero
	// header_hash: to be computed after optional-header region is finalized.
	// game_regions default to 0xFFFFFFFF so the game can run on any region.
	BigEndian::write_u32(&mut out[ii_start + 0x70..ii_start + 0x74], 0xFFFFFFFFu32);

	// allowed_media_types at security_offset + 0x17C (outside image_info).
	BigEndian::write_u32(&mut out[security_offset + 0x17C..security_offset + 0x180], 0xFFFFFFFFu32);

	// page_descriptor_count + descriptors.
	BigEndian::write_u32(&mut out[security_offset + 0x180..security_offset + 0x184], descriptors.len() as u32);
	for (i, d) in descriptors.iter().enumerate() {
		let off = security_offset + 0x184 + i * 24;
		out[off..off + 24].copy_from_slice(&d.to_bytes());
	}

	// PE data (or compressed stream, selected above).
	out[data_offset..data_offset + data_region.len()].copy_from_slice(data_region);

	// Compute header_hash now that the whole pre-PE region is finalized.
	// We need a Xex2Header value to call compute_header_hash -- re-parse.
	let parsed = crate::header::Xex2Header::parse(&out[..])?;
	let parsed_sec = crate::header::SecurityInfo::parse(&out[..], parsed.security_offset as usize)?;
	let header_hash = hashes::compute_header_hash(&out, &parsed, &parsed_sec);
	out[ii_start + 0x5C..ii_start + 0x70].copy_from_slice(&*header_hash);

	// RotSumSha + sign.
	let image_info = &out[ii_start..ii_start + 0x74];
	let digest = xecrypt::symmetric::xe_crypt_rot_sum_sha(image_info, &[]);
	let sig = xecrypt::RsaKeyKind::Pirs
		.sign(xecrypt::ConsoleKind::Devkit, &digest)
		.map_err(|_| Xex2Error::SigningFailed.into_report())?;
	out[security_offset + 0x08..security_offset + 0x108].copy_from_slice(&sig);

	// Silence unused warnings for fields we may wire up later.
	let _ = crypto::DEVKIT_KEY;
	let _ = EncryptionType::None;

	Ok(out)
}

enum OptEntry {
	Inline { key: u32, value: u32 },
	Data { key: u32, offset: u32 },
}

struct BlobPlacement {
	offset: usize,
	bytes: Vec<u8>,
}

fn align_up(x: usize, align: usize) -> usize {
	x.div_ceil(align) * align
}

fn execution_info_bytes(b: &Xex2Builder) -> Vec<u8> {
	// 0x18 bytes:
	//   +0x00: media_id (u32)
	//   +0x04: version (u32)
	//   +0x08: base_version (u32)
	//   +0x0C: title_id (u32)
	//   +0x10: platform (u8)
	//   +0x11: executable_table (u8)
	//   +0x12: disc_number (u8)
	//   +0x13: disc_count (u8)
	//   +0x14: savegame_id (u32)
	let mut out = vec![0u8; 0x18];
	BigEndian::write_u32(&mut out[0x00..0x04], b.media_id.0);
	BigEndian::write_u32(&mut out[0x04..0x08], u32::from(b.version));
	BigEndian::write_u32(&mut out[0x08..0x0C], u32::from(b.base_version));
	BigEndian::write_u32(&mut out[0x0C..0x10], b.title_id.0);
	out
}

fn empty_import_libraries_bytes() -> Vec<u8> {
	// u32 total_size, u32 strings_size=0, u32 lib_count=0
	let mut out = vec![0u8; 12];
	BigEndian::write_u32(&mut out[0..4], 12);
	out
}

fn file_format_info_bytes() -> Vec<u8> {
	// u32 info_size, u16 encryption_type=None, u16 compression_type=None
	let mut out = vec![0u8; 8];
	BigEndian::write_u32(&mut out[0..4], 8);
	// encryption_type + compression_type = 0
	out
}
