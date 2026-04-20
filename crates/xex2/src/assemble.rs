//! Full-rebuild assembler for length-changing transforms.
//!
//! [`crate::patch::Patch`] and [`crate::writer::plan_edits`] handle
//! length-preserving edits. Compression transforms (None <-> Normal) change the
//! size of both the FileFormatInfo blob and the data region, so they require
//! laying the file out from scratch. PE replacement on a compressed source is
//! the same story: we decompress, (re)compress under the target, then assemble.
//!
//! The output preserves every optional header the source XEX had (via the
//! parsed `header.optional_headers` map) *except* FileFormatInfo, which is
//! regenerated for the target compression. Security info fields carry over
//! from the source; `image_hash`, `header_hash`, and the RSA signature are
//! recomputed at the end.
//!
//! Per-field edits from [`crate::writer::EditPlan`] (limits, flags, etc.)
//! **aren't** applied here -- the caller is expected to run `plan_edits` on
//! the assembled file afterward to stage those via the normal patch path.

use crate::Xex2;
use crate::compress;
use crate::error::Result;
use crate::error::Xex2Error;
use crate::hashes;
use crate::header::CompressionType;
use crate::header::EncryptionType;
use crate::header::OptionalHeaderKey;
use crate::header::OptionalHeaderValue;
use crate::header::SecurityInfo;
use crate::header::XEX2_MAGIC;
use crate::header::Xex2Header;
use crate::opt::ImageFlags;
use crate::page_descriptors;
use crate::writer::TargetCompression;
use byteorder::BigEndian;
use byteorder::ByteOrder;
use rootcause::IntoReport;

const OPT_INDEX_START: usize = 0x18;
const PAGE_ALIGN: usize = 0x1000;
/// Default LZX window for compressed XEX output. 64 KB matches what most
/// real XEX files ship with; callers needing a different size should go
/// through [`crate::builder::Xex2Builder`] (future API).
const DEFAULT_LZX_WINDOW: u32 = 0x10000;

/// Assemble a new XEX swapping the data region / FileFormatInfo under the
/// requested compression. Returns the full file bytes; the caller writes
/// them to their sink and, if desired, runs [`crate::writer::plan_edits`]
/// on the result to apply any remaining per-field edits.
///
/// `target_compression` selects the new compression state. `replace_pe`, if
/// provided, swaps the inner PE for these bytes (decompressed+decrypted
/// reference) before (re)compression.
pub fn rebuild_with_compression(
	xex: &Xex2,
	source: &[u8],
	target_compression: TargetCompression,
	replace_pe: Option<Vec<u8>>,
) -> Result<Vec<u8>> {
	let original_ff = xex.header.file_format_info()?;

	let pe: Vec<u8> = match replace_pe {
		Some(new_pe) => new_pe,
		None => crate::basefile::extract_basefile(source, &xex.header, &xex.security_info)?,
	};

	// Generate descriptors up front so the match arms below can *move* `pe`
	// (or the compressed stream) into `data_region` without cloning.
	let page_size: u32 =
		if xex.security_info.image_info.image_flags.contains(ImageFlags::SMALL_PAGES) { 0x1000 } else { 0x10000 };
	let template = existing_descriptor_template(source, xex);
	let page_descriptors::GeneratedDescriptors { descriptors, image_hash } =
		page_descriptors::generate(&pe, page_size, template.as_deref());
	let image_size = pe.len() as u32;

	// Preserve the source's encryption state here. Re-encryption is separate
	// and handled by `plan_edits` on the assembled output.
	let encryption = original_ff.encryption_type;
	let session_key = (encryption == EncryptionType::Normal).then(|| {
		let keys = crate::crypto::decrypt_file_key(&xex.security_info.image_info.file_key);
		pick_session_key(source, xex, &original_ff, &keys)
	});
	let (data_region, ff_blob) = match target_compression {
		TargetCompression::Uncompressed => {
			let data = match session_key {
				Some(key) => crate::crypto::encrypt_data(&pe, &key),
				None => pe,
			};
			(data, compress::file_format_info_blob_none(encryption))
		}
		TargetCompression::Normal => {
			let stream = compress::compress_normal(&pe, DEFAULT_LZX_WINDOW)?;
			let blob = compress::file_format_info_blob_normal(encryption, &stream);
			let data = match session_key {
				Some(key) => crate::crypto::encrypt_data(&stream.data, &key),
				None => stream.data,
			};
			(data, blob)
		}
		TargetCompression::Basic => {
			return Err(Xex2Error::RebuildTransformNotImplemented.into_report());
		}
	};

	let layout = Layout::compute(&xex.header, &ff_blob, descriptors.len());
	let out_size = layout.data_offset + data_region.len();
	let mut out = vec![0u8; out_size];

	write_main_header(&mut out, &xex.header, &layout);
	write_opt_index(&mut out, &xex.header, &layout);
	write_blobs(&mut out, &xex.header, &layout, &ff_blob);
	write_security_info(&mut out, &xex.security_info, &layout, image_size, image_hash, &descriptors);
	out[layout.data_offset..layout.data_offset + data_region.len()].copy_from_slice(&data_region);

	// header_hash covers bytes we just wrote, so compute it after layout.
	let parsed = Xex2Header::parse(&out[..])?;
	let parsed_sec = SecurityInfo::parse(&out[..], parsed.security_offset as usize)?;
	let header_hash = hashes::compute_header_hash(&out, &parsed, &parsed_sec);
	let ii_start = layout.security_offset + 0x108;
	out[ii_start + 0x5C..ii_start + 0x70].copy_from_slice(&*header_hash);

	let image_info_bytes = &out[ii_start..ii_start + 0x74];
	let digest = xecrypt::symmetric::xe_crypt_rot_sum_sha(image_info_bytes, &[]);
	let sig = xecrypt::RsaKeyKind::Pirs
		.sign(xecrypt::ConsoleKind::Devkit, &digest)
		.map_err(|_| Xex2Error::SigningFailed.into_report())?;
	out[layout.security_offset + 0x08..layout.security_offset + 0x108].copy_from_slice(&sig);

	Ok(out)
}

fn existing_descriptor_template(source: &[u8], xex: &Xex2) -> Option<Vec<page_descriptors::DescriptorSlot>> {
	let base = xex.header.security_offset as usize + 0x184;
	let count = xex.security_info.page_descriptor_count as usize;
	let mut out = Vec::with_capacity(count);
	for i in 0..count {
		let off = base + i * 24;
		let bytes: &[u8; 4] = source.get(off..off + 4)?.try_into().ok()?;
		let info = u32::from_be_bytes(*bytes);
		out.push(page_descriptors::DescriptorSlot { page_count: info >> 4, flags: info & 0xF });
	}
	Some(out)
}

fn pick_session_key(
	source: &[u8],
	xex: &Xex2,
	ff: &crate::header::FileFormatInfo,
	keys: &crate::crypto::DecryptedKeys,
) -> crate::header::AesKey {
	// Minimal probe: try retail; fall back to devkit if the first block looks
	// wrong. Mirrors `basefile::try_decrypt_with_key`'s heuristic.
	let data_off = xex.header.data_offset as usize;
	let probe_len = 32.min(source.len().saturating_sub(data_off));
	let probe = &source[data_off..data_off + probe_len];
	let looks_valid = |k: &crate::header::AesKey| {
		if probe.len() < 16 {
			return false;
		}
		let plain = crate::crypto::decrypt_data(&probe[..16], k);
		match ff.compression_type {
			CompressionType::None | CompressionType::Basic => plain.starts_with(b"MZ"),
			_ => true,
		}
	};
	if looks_valid(&keys.retail) { keys.retail } else { keys.devkit }
}

/// Tracks blob placements + the resulting security / data offsets.
struct Layout {
	/// Offset of each optional-header data blob. Inline entries aren't tracked
	/// (the value IS the data).
	blob_offset_by_key: std::collections::BTreeMap<u32, usize>,
	security_offset: usize,
	data_offset: usize,
}

impl Layout {
	/// Compute placements for every blob in `header.optional_headers`, using
	/// `ff_blob.len()` for the FileFormatInfo entry. Subsequent offsets (and
	/// eventually `security_offset` / page-aligned `data_offset`) fall out.
	fn compute(header: &Xex2Header, ff_blob: &[u8], descriptor_count: usize) -> Self {
		let mut blob_offset_by_key = std::collections::BTreeMap::new();
		let mut cursor = OPT_INDEX_START + header.optional_header_count as usize * 8;
		for (&key, value) in &header.optional_headers {
			let OptionalHeaderValue::Data(bytes) = value else {
				continue;
			};
			let blob_len = if key == OptionalHeaderKey::FileFormatInfo as u32 { ff_blob.len() } else { bytes.len() };
			blob_offset_by_key.insert(key, cursor);
			cursor += blob_len;
			cursor = align_up(cursor, 4);
		}
		let security_offset = cursor;
		cursor += 0x184 + descriptor_count * 24;
		let data_offset = align_up(cursor, PAGE_ALIGN);
		Self { blob_offset_by_key, security_offset, data_offset }
	}
}

fn align_up(x: usize, align: usize) -> usize {
	x.div_ceil(align) * align
}

fn write_main_header(out: &mut [u8], header: &Xex2Header, layout: &Layout) {
	out[0..4].copy_from_slice(&XEX2_MAGIC);
	BigEndian::write_u32(&mut out[0x04..0x08], header.module_flags.bits());
	BigEndian::write_u32(&mut out[0x08..0x0C], layout.data_offset as u32);
	// +0x0C: reserved.
	BigEndian::write_u32(&mut out[0x10..0x14], layout.security_offset as u32);
	BigEndian::write_u32(&mut out[0x14..0x18], header.optional_header_count);
}

fn write_opt_index(out: &mut [u8], header: &Xex2Header, layout: &Layout) {
	let mut off = OPT_INDEX_START;
	for (&key, value) in &header.optional_headers {
		match value {
			OptionalHeaderValue::Inline(v) => {
				BigEndian::write_u32(&mut out[off..off + 4], key);
				BigEndian::write_u32(&mut out[off + 4..off + 8], *v);
			}
			OptionalHeaderValue::Data(_) => {
				let blob_off = layout
					.blob_offset_by_key
					.get(&key)
					.copied()
					.expect("blob placement missing for data-valued opt header");
				BigEndian::write_u32(&mut out[off..off + 4], key);
				BigEndian::write_u32(&mut out[off + 4..off + 8], blob_off as u32);
			}
		}
		off += 8;
	}
}

fn write_blobs(out: &mut [u8], header: &Xex2Header, layout: &Layout, ff_blob: &[u8]) {
	for (&key, value) in &header.optional_headers {
		let OptionalHeaderValue::Data(bytes) = value else {
			continue;
		};
		let off = layout.blob_offset_by_key.get(&key).copied().expect("blob placement missing");
		let src = if key == OptionalHeaderKey::FileFormatInfo as u32 { ff_blob } else { bytes.as_slice() };
		out[off..off + src.len()].copy_from_slice(src);
	}
}

fn write_security_info(
	out: &mut [u8],
	sec: &SecurityInfo,
	layout: &Layout,
	image_size: u32,
	image_hash: crate::header::Sha1Hash,
	descriptors: &[page_descriptors::PageDescriptor],
) {
	let so = layout.security_offset;
	let sec_info_size = 0x184 + descriptors.len() * 24;
	BigEndian::write_u32(&mut out[so..so + 4], sec_info_size as u32);
	BigEndian::write_u32(&mut out[so + 4..so + 8], image_size);
	// RSA signature placeholder at so+0x08..0x108 -- filled by caller.

	// image_info at so + 0x108.
	let ii = so + 0x108;
	BigEndian::write_u32(&mut out[ii..ii + 4], 0x174);
	BigEndian::write_u32(&mut out[ii + 0x04..ii + 0x08], sec.image_info.image_flags.bits());
	BigEndian::write_u32(&mut out[ii + 0x08..ii + 0x0C], sec.image_info.load_address.0);
	out[ii + 0x0C..ii + 0x20].copy_from_slice(&*image_hash);
	BigEndian::write_u32(&mut out[ii + 0x20..ii + 0x24], sec.image_info.import_table_count);
	out[ii + 0x24..ii + 0x38].copy_from_slice(&*sec.image_info.import_table_hash);
	out[ii + 0x38..ii + 0x48].copy_from_slice(&sec.image_info.media_id);
	out[ii + 0x48..ii + 0x58].copy_from_slice(&*sec.image_info.file_key);
	BigEndian::write_u32(&mut out[ii + 0x58..ii + 0x5C], sec.image_info.export_table_address);
	// header_hash: 0x5C..0x70 (filled by caller after recompute).
	BigEndian::write_u32(&mut out[ii + 0x70..ii + 0x74], sec.image_info.game_regions);

	// allowed_media_types at so + 0x17C.
	BigEndian::write_u32(&mut out[so + 0x17C..so + 0x180], sec.image_info.allowed_media_types.bits());

	// page_descriptor_count at so + 0x180, then entries at so + 0x184.
	BigEndian::write_u32(&mut out[so + 0x180..so + 0x184], descriptors.len() as u32);
	for (i, d) in descriptors.iter().enumerate() {
		let off = so + 0x184 + i * 24;
		out[off..off + 24].copy_from_slice(&d.to_bytes());
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn layout_align_up_rounds_as_expected() {
		assert_eq!(align_up(1, 0x1000), 0x1000);
		assert_eq!(align_up(0x1000, 0x1000), 0x1000);
		assert_eq!(align_up(0x1001, 0x1000), 0x2000);
	}
}
