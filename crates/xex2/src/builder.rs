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

use crate::error::BasefileDefect;
use crate::error::Result;
use crate::error::Xex2Error;
use crate::hashes;
use crate::header::EncryptionType;
use crate::header::OptionalHeaderKey;
use crate::imports::ImportLibrary;
use crate::imports::serialize_import_libraries;
use crate::opt::ImageFlags;
use crate::opt::ModuleFlags;
use crate::page_descriptors;
use crate::page_descriptors::SectionType;
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
	/// Import libraries to emit in the `ImportLibraries` optional header. Empty
	/// yields the minimal empty table.
	imports: Vec<ImportLibrary>,
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
			imports: Vec::new(),
		}
	}

	/// Set the import libraries emitted in the `ImportLibraries` header. Each
	/// library's records are VAs of import descriptors inside the PE image.
	pub fn imports(mut self, imports: Vec<ImportLibrary>) -> Self {
		self.imports = imports;
		self
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
	let import_libs = if b.imports.is_empty() {
		empty_import_libraries_bytes()
	} else {
		serialize_import_libraries(&b.imports)
	};
	let file_format = match &compressed_stream {
		Some(stream) => crate::compress::file_format_info_blob_normal(EncryptionType::None, stream),
		None => file_format_info_bytes(EncryptionType::None),
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

	// Page descriptors: one per page, each recording the page's section type.
	// The kernel and hypervisor read this to protect memory: a page that is not
	// marked as code is not added to the executable range, so code placed there
	// cannot run. Derive each page's type from the PE section covering it. The
	// hash chain applies to every page regardless of type.
	let page_size: u32 = if b.image_flags.contains(ImageFlags::SMALL_PAGES) { 0x1000 } else { 0x10000 };
	let page_count_total = (b.pe.len() as u32).div_ceil(page_size);
	let template: Vec<page_descriptors::DescriptorSlot> = page_section_types(&b.pe, page_size, page_count_total)?
		.into_iter()
		.map(|section_type| page_descriptors::DescriptorSlot { page_count: 1, section_type })
		.collect();
	let page_descriptors::GeneratedDescriptors { descriptors, image_hash } =
		page_descriptors::generate(&b.pe, page_size, Some(&template));

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
	// import_table_count = number of import libraries.
	BigEndian::write_u32(&mut out[ii_start + 0x20..ii_start + 0x24], b.imports.len() as u32);
	// import_table_hash left zero (Xenia does not verify it)
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

	// Chain the import-library digests and record image_info.import_table_hash.
	// The real kernel recomputes this chain and rejects a non-empty import
	// table whose hash is zero, so it must be set before header_hash + signing.
	if !b.imports.is_empty() {
		let blob = &mut out[import_libs_off..import_libs_off + import_libs.len()];
		if let Some(hash) = hashes::rewrite_import_table_hashes(blob) {
			out[ii_start + 0x24..ii_start + 0x38].copy_from_slice(&*hash);
		}
	}

	// Compute header_hash now that the whole pre-PE region is finalized.
	// We need a Xex2Header value to call compute_header_hash -- re-parse.
	let parsed = crate::header::Xex2Header::parse(&out[..])?;
	let parsed_sec = crate::header::SecurityInfo::parse(&out[..], parsed.security_offset as usize)?;
	let header_hash = hashes::compute_header_hash(&out, &parsed, &parsed_sec);
	out[ii_start + 0x5C..ii_start + 0x70].copy_from_slice(&*header_hash);

	// RotSumSha + sign. NOTE: PKCS#1 here is accepted by Xenia only; real
	// devkit hardware needs XeCryptBnQwBeSig (see xecrypt::xex_sig) and an
	// encrypted payload. Hardware XEXs currently go through imagexex instead.
	let image_info = &out[ii_start..ii_start + 0x74];
	let digest = xecrypt::symmetric::xe_crypt_rot_sum_sha(image_info, &[]);
	let sig = xecrypt::RsaKeyKind::Pirs
		.sign(xecrypt::ConsoleKind::Devkit, &digest)
		.map_err(|_| Xex2Error::SigningFailed.into_report())?;
	out[security_offset + 0x08..security_offset + 0x108].copy_from_slice(&sig);

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

// PE section header characteristics bits.
const SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const SCN_MEM_WRITE: u32 = 0x8000_0000;

/// Determine the section type of each page from the PE sections that cover it.
///
/// A page may span sections with different protections when the linker packs
/// code and data together. A page descriptor records one section type, not a
/// set, so the type is chosen by precedence: a page covered by any executable
/// section is code; otherwise a page covered by any writable section is data;
/// otherwise it is read-only data. A page that holds both code and writable
/// data is therefore mapped read-only, so writable data must occupy a separate
/// page to remain writable. Pages that no section covers, such as the PE
/// headers, are read-only data.
///
/// A parse failure returns an error rather than a default, because typing every
/// page as read-only data would yield a XEX with no executable page whose entry
/// point cannot run.
fn page_section_types(pe: &[u8], page_size: u32, page_count: u32) -> Result<Vec<SectionType>> {
	let rd_u32 = |off: usize| -> Option<u32> {
		pe.get(off..off + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
	};
	let rd_u16 = |off: usize| -> Option<u16> {
		pe.get(off..off + 2).map(|b| u16::from_le_bytes(b.try_into().unwrap()))
	};
	let defect = |d: BasefileDefect| Xex2Error::MalformedBasefilePe(d).into_report();

	let lfanew = rd_u32(0x3C).map(|v| v as usize).ok_or_else(|| defect(BasefileDefect::TruncatedDosHeader))?;
	// COFF header at lfanew + 4: NumberOfSections at + 2, SizeOfOptionalHeader at + 16.
	let num_sections = rd_u16(lfanew + 6).ok_or_else(|| defect(BasefileDefect::TruncatedCoffHeader))?;
	let opt_size = rd_u16(lfanew + 20).map(|v| v as usize).ok_or_else(|| defect(BasefileDefect::TruncatedCoffHeader))?;
	let sec_table = lfanew + 4 + 20 + opt_size;

	let mut any_exec = vec![false; page_count as usize];
	let mut any_write = vec![false; page_count as usize];
	for i in 0..num_sections as usize {
		let hdr = sec_table + i * 40;
		let (Some(vsize), Some(vaddr), Some(chars)) =
			(rd_u32(hdr + 8), rd_u32(hdr + 12), rd_u32(hdr + 36))
		else {
			return Err(defect(BasefileDefect::TruncatedSectionHeader));
		};
		let first_page = vaddr / page_size;
		let last_page = vaddr.saturating_add(vsize).div_ceil(page_size).min(page_count);
		for p in first_page..last_page {
			if chars & SCN_MEM_EXECUTE != 0 {
				any_exec[p as usize] = true;
			}
			if chars & SCN_MEM_WRITE != 0 {
				any_write[p as usize] = true;
			}
		}
	}
	Ok((0..page_count as usize)
		.map(|p| {
			if any_exec[p] {
				SectionType::Code
			} else if any_write[p] {
				SectionType::Data
			} else {
				SectionType::ReadOnlyData
			}
		})
		.collect())
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

fn file_format_info_bytes(encryption: EncryptionType) -> Vec<u8> {
	// u32 info_size, u16 encryption_type, u16 compression_type=None
	let mut out = vec![0u8; 8];
	BigEndian::write_u32(&mut out[0..4], 8);
	BigEndian::write_u16(&mut out[4..6], encryption as u16);
	// compression_type = 0 (None)
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Build a little-endian PE that populates only the header fields
	/// [`page_section_types`] reads. Each section is `(virtual_address,
	/// virtual_size, characteristics)`.
	fn synthetic_pe(sections: &[(u32, u32, u32)]) -> Vec<u8> {
		let lfanew = 0x40usize;
		let opt_size = 0xE0usize;
		let sec_table = lfanew + 24 + opt_size;
		let mut pe = vec![0u8; sec_table + sections.len() * 40];
		pe[0x3C..0x40].copy_from_slice(&(lfanew as u32).to_le_bytes());
		pe[lfanew..lfanew + 4].copy_from_slice(b"PE\0\0");
		pe[lfanew + 6..lfanew + 8].copy_from_slice(&(sections.len() as u16).to_le_bytes());
		pe[lfanew + 20..lfanew + 22].copy_from_slice(&(opt_size as u16).to_le_bytes());
		for (i, (vaddr, vsize, chars)) in sections.iter().enumerate() {
			let h = sec_table + i * 40;
			pe[h + 8..h + 12].copy_from_slice(&vsize.to_le_bytes());
			pe[h + 12..h + 16].copy_from_slice(&vaddr.to_le_bytes());
			pe[h + 36..h + 40].copy_from_slice(&chars.to_le_bytes());
		}
		pe
	}

	#[test]
	fn page_type_follows_section_protection() {
		let ps = 0x10000u32;
		let pe = synthetic_pe(&[
			(0x10000, ps, SCN_MEM_EXECUTE),
			(0x20000, ps, SCN_MEM_WRITE),
			(0x30000, ps, 0),
		]);
		let types = page_section_types(&pe, ps, 5).unwrap();
		assert_eq!(
			types,
			vec![
				SectionType::ReadOnlyData, // page 0: no section (PE headers)
				SectionType::Code,         // page 1: executable
				SectionType::Data,         // page 2: writable
				SectionType::ReadOnlyData, // page 3: read-only
				SectionType::ReadOnlyData, // page 4: no section
			]
		);
	}

	#[test]
	fn code_takes_precedence_on_a_shared_page() {
		let ps = 0x10000u32;
		let pe = synthetic_pe(&[
			(0x10000, 0x1000, SCN_MEM_EXECUTE),
			(0x18000, 0x1000, SCN_MEM_WRITE),
		]);
		let types = page_section_types(&pe, ps, 2).unwrap();
		assert_eq!(types, vec![SectionType::ReadOnlyData, SectionType::Code]);
	}

	#[test]
	fn truncated_pe_reports_a_defect() {
		let err = page_section_types(&[0u8; 8], 0x10000, 1).unwrap_err();
		assert!(matches!(
			err.current_context(),
			Xex2Error::MalformedBasefilePe(BasefileDefect::TruncatedDosHeader)
		));
	}
}
