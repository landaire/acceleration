//! Page-descriptor generation and verification for the inner PE image.
//!
//! The XEX security info stores a table of per-page descriptors that the
//! hypervisor uses to verify each page of the decompressed PE image. Each
//! descriptor is 24 bytes: `u32 (page_count << 4 | flags)` + `[u8; 20]` hash.
//!
//! # Verification chain (reverse-engineered from `HvxLoadImageData` in the HV)
//!
//! The HV maintains a running "expected" state initialized to
//! `image_info.image_hash`. For each descriptor `i`:
//!
//! 1. `H[i] = SHA1(page_data[i] || descriptor[i].bytes(24))`
//! 2. Assert `H[i] == expected[i]`
//! 3. `expected[i+1] = descriptor[i].stored_hash` (bytes 4..24)
//!
//! This is a forward-chained hash that anchors at `image_info.image_hash`.
//!
//! # Generation (inverse)
//!
//! Work backwards from a terminator hash (nothing reads the last descriptor's
//! stored_hash, so any value works -- we use zeros):
//!
//! - `descriptor[N-1].stored_hash = zeros`
//! - `descriptor[i-1].stored_hash = SHA1(page_data[i] || descriptor[i].bytes(24))`
//! - `image_info.image_hash = SHA1(page_data[0] || descriptor[0].bytes(24))`

use sha1::Digest;
use sha1::Sha1;

use crate::error::Result;
use crate::error::Xex2Error;
use crate::header::SecurityInfo;
use crate::header::Sha1Hash;
use crate::header::Xex2Header;
use crate::opt::ImageFlags;
use rootcause::IntoReport;
use xenon_types::VirtualAddress;

/// Section type stored in the four-bit info field of a page descriptor. The
/// kernel and hypervisor read it to assign memory protection to each page.
/// `Other` preserves any four-bit value that is not one of the three defined
/// types, so descriptors copied from an existing image round-trip unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionType {
	/// Executable, mapped read-only.
	Code,
	/// Read/write.
	Data,
	/// Read-only, not executable.
	ReadOnlyData,
	/// Any other four-bit value.
	Other(u8),
}

impl SectionType {
	pub fn nibble(self) -> u32 {
		match self {
			SectionType::Code => 1,
			SectionType::Data => 2,
			SectionType::ReadOnlyData => 3,
			SectionType::Other(value) => (value & 0xF) as u32,
		}
	}

	pub fn from_nibble(value: u32) -> SectionType {
		match value & 0xF {
			1 => SectionType::Code,
			2 => SectionType::Data,
			3 => SectionType::ReadOnlyData,
			other => SectionType::Other(other as u8),
		}
	}
}

impl std::fmt::Display for SectionType {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			SectionType::Code => f.write_str("code"),
			SectionType::Data => f.write_str("data"),
			SectionType::ReadOnlyData => f.write_str("ro"),
			SectionType::Other(value) => write!(f, "other({value})"),
		}
	}
}

/// Section type for the appended pages in `length` bytes from `address`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageTypeRange {
	pub address: VirtualAddress,
	pub length: u32,
	pub section_type: SectionType,
}

/// Most pages one generated descriptor covers.
const MAX_PAGES_PER_DESCRIPTOR: u32 = 16;
/// Generated descriptors never cross this virtual-address boundary.
const DESCRIPTOR_BOUNDARY: u64 = 0x1_0000;

/// Extend `template` (the source descriptors) to cover `required_pages`.
///
/// Without `ranges`, appended pages take the last descriptor's type in runs of
/// `0x10000 / page_size` pages counted from the end of the source pages, as
/// before page-type ranges existed. With `ranges`, each appended page takes
/// the type of the range containing it (else the last type), and a descriptor
/// ends at each type change, 64 KiB virtual-address boundary, or 16 pages.
pub(crate) fn extend_template(
	template: &mut Vec<DescriptorSlot>,
	required_pages: u32,
	page_size: u32,
	load_address: VirtualAddress,
	ranges: &[PageTypeRange],
) -> Result<()> {
	let covered_pages: u32 = template.iter().map(|slot| slot.page_count).sum();
	let base = u64::from(load_address.0);
	let appended_start = base + u64::from(covered_pages) * u64::from(page_size);
	let appended_end = base + u64::from(required_pages.max(covered_pages)) * u64::from(page_size);
	let mut sorted = ranges.to_vec();
	sorted.sort_by_key(|range| range.address.0);
	for range in &sorted {
		if let SectionType::Other(value) = range.section_type {
			return Err(Xex2Error::PageTypeRangeType { address: range.address.0, value }.into_report());
		}
		if range.length == 0 || !range.address.0.is_multiple_of(page_size) || !range.length.is_multiple_of(page_size) {
			return Err(Xex2Error::PageTypeRangeAlignment {
				address: range.address.0,
				length: range.length,
				page_size,
			}
			.into_report());
		}
		let start = u64::from(range.address.0);
		if start < appended_start || start + u64::from(range.length) > appended_end {
			return Err(Xex2Error::PageTypeRangeOutsideAppended {
				address: range.address.0,
				length: range.length,
				appended_start,
				appended_end,
			}
			.into_report());
		}
	}
	let range_end = |range: &PageTypeRange| u64::from(range.address.0) + u64::from(range.length);
	if let Some(pair) = sorted.windows(2).find(|pair| range_end(&pair[0]) > u64::from(pair[1].address.0)) {
		return Err(
			Xex2Error::PageTypeRangeOverlap { first: pair[0].address.0, second: pair[1].address.0 }.into_report()
		);
	}
	if required_pages <= covered_pages {
		return Ok(());
	}
	let Some(last) = template.last().copied() else {
		return Err(Xex2Error::InvalidFileFormatInfo.into_report());
	};

	if sorted.is_empty() {
		let pages_per_descriptor = 0x1_0000 / page_size;
		let mut remaining = required_pages - covered_pages;
		while remaining != 0 {
			let page_count = remaining.min(pages_per_descriptor);
			template.push(DescriptorSlot { page_count, section_type: last.section_type });
			remaining -= page_count;
		}
		return Ok(());
	}

	let mut open: Option<DescriptorSlot> = None;
	for page in covered_pages..required_pages {
		let address = base + u64::from(page) * u64::from(page_size);
		let section_type = sorted
			.iter()
			.find(|range| (u64::from(range.address.0)..range_end(range)).contains(&address))
			.map_or(last.section_type, |range| range.section_type);
		let split = open.is_none_or(|slot| {
			slot.section_type != section_type
				|| slot.page_count == MAX_PAGES_PER_DESCRIPTOR
				|| address.is_multiple_of(DESCRIPTOR_BOUNDARY)
		});
		if split {
			template.extend(open.take());
		}
		open.get_or_insert(DescriptorSlot { page_count: 0, section_type }).page_count += 1;
	}
	template.extend(open);
	Ok(())
}

/// Read the page count and section type of each stored descriptor.
/// [`Xex2Error::PageDescriptorsTruncated`] when the table runs past `source`.
pub fn descriptor_slots(source: &[u8], security_offset: u32, count: u32) -> Result<Vec<DescriptorSlot>> {
	let truncated = || Xex2Error::PageDescriptorsTruncated.into_report();
	let base = (security_offset as usize).checked_add(0x184).ok_or_else(truncated)?;
	let table_len = (count as usize).checked_mul(24).ok_or_else(truncated)?;
	let table = base
		.checked_add(table_len)
		.and_then(|end| source.get(base..end))
		.ok_or_else(truncated)?;
	let mut out = Vec::new();
	out.try_reserve_exact(count as usize).map_err(|_| truncated())?;
	out.extend(table.as_chunks::<24>().0.iter().map(|entry| {
		let info = u32::from_be_bytes([entry[0], entry[1], entry[2], entry[3]]);
		DescriptorSlot { page_count: info >> 4, section_type: SectionType::from_nibble(info) }
	}));
	Ok(out)
}

/// A single 24-byte page descriptor entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PageDescriptor {
	/// Number of `page_size` pages this descriptor covers.
	pub page_count: u32,
	/// Section type stored in the descriptor info field.
	pub section_type: SectionType,
	/// For descriptor `i`, the expected hash of descriptor `i + 1` in the
	/// verification chain.
	pub hash: Sha1Hash,
}

impl PageDescriptor {
	pub(crate) fn to_bytes(self) -> [u8; 24] {
		let info = (self.page_count << 4) | self.section_type.nibble();
		let mut buf = [0u8; 24];
		buf[0..4].copy_from_slice(&info.to_be_bytes());
		buf[4..24].copy_from_slice(&*self.hash);
		buf
	}

	#[cfg(test)]
	pub(crate) fn from_bytes(bytes: &[u8; 24]) -> Self {
		let info = u32::from_be_bytes(bytes[0..4].try_into().unwrap());
		let hash: [u8; 20] = bytes[4..24].try_into().unwrap();
		Self { page_count: info >> 4, section_type: SectionType::from_nibble(info), hash: Sha1Hash(hash) }
	}
}

fn page_size_for(flags: ImageFlags) -> u32 {
	if flags.contains(ImageFlags::SMALL_PAGES) { 0x1000 } else { 0x10000 }
}

/// Hash a page of exactly `declared_len` bytes followed by the 24-byte
/// descriptor. If the actual `page` is shorter than `declared_len` (last
/// page of the image), pad with zeros -- the HV sees virtual memory which
/// is zero-initialized past image_size.
fn sha1_page_and_descriptor(page: &[u8], declared_len: usize, descriptor_bytes: &[u8; 24]) -> Sha1Hash {
	let mut h = Sha1::new();
	h.update(page);
	if page.len() < declared_len {
		let padding = vec![0u8; declared_len - page.len()];
		h.update(&padding);
	}
	h.update(descriptor_bytes);
	Sha1Hash(h.finalize().into())
}

/// One descriptor slot in [`generate`]'s `template`: how many `page_size`
/// pages it covers and the section type to record. A named struct rather than
/// a bare tuple prevents transposing the two fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescriptorSlot {
	pub page_count: u32,
	pub section_type: SectionType,
}

/// Output of [`generate`]: the new `page_descriptors` array plus the
/// `image_info.image_hash` that roots its hash chain.
pub(crate) struct GeneratedDescriptors {
	pub descriptors: Vec<PageDescriptor>,
	pub image_hash: Sha1Hash,
}

/// Generate descriptors + `image_info.image_hash` for a new PE image.
///
/// `template` controls descriptor grouping (page count and section type per
/// entry). Pass `None` to use a single descriptor that covers the whole image
/// as [`SectionType::Code`].
pub(crate) fn generate(pe: &[u8], page_size: u32, template: Option<&[DescriptorSlot]>) -> GeneratedDescriptors {
	let page_size_usize = page_size as usize;
	let owned_template: Vec<DescriptorSlot>;
	let template = match template {
		Some(t) => t,
		None => {
			let total = pe.len().div_ceil(page_size_usize) as u32;
			owned_template = vec![DescriptorSlot { page_count: total, section_type: SectionType::Code }];
			&owned_template
		}
	};

	// Slice PE into per-descriptor ranges. Each range's *declared* length is
	// `count * page_size`; the actual PE byte span may be shorter for the
	// last descriptor, in which case hashing pads with zeros.
	struct Range {
		start: usize,
		end: usize,
		declared_len: usize,
	}
	let mut ranges: Vec<Range> = Vec::with_capacity(template.len());
	let mut cursor = 0usize;
	for slot in template {
		let declared = (slot.page_count as usize) * page_size_usize;
		let end = (cursor + declared).min(pe.len());
		ranges.push(Range { start: cursor, end, declared_len: declared });
		cursor = end;
	}

	// Last descriptor's stored_hash is the "terminator" -- nothing reads it
	// after the loop, so any value works. Zeros is simplest.
	let mut descriptors: Vec<PageDescriptor> = template
		.iter()
		.map(|s| PageDescriptor { page_count: s.page_count, section_type: s.section_type, hash: Sha1Hash::ZERO })
		.collect();

	// Work backwards: descriptor[i-1].stored_hash = SHA1(page_data[i] || descriptor[i].bytes).
	for i in (1..descriptors.len()).rev() {
		let desc_bytes = descriptors[i].to_bytes();
		let r = &ranges[i];
		descriptors[i - 1].hash = sha1_page_and_descriptor(&pe[r.start..r.end], r.declared_len, &desc_bytes);
	}

	// image_info.image_hash = SHA1(page_data[0] || descriptor[0].bytes).
	let desc0_bytes = descriptors[0].to_bytes();
	let r0 = &ranges[0];
	let image_hash = sha1_page_and_descriptor(&pe[r0.start..r0.end], r0.declared_len, &desc0_bytes);

	GeneratedDescriptors { descriptors, image_hash }
}

/// Verify the complete page-descriptor chain against a decompressed PE
/// image and an XEX's `source` bytes.
pub fn verify_chain(pe: &[u8], header: &Xex2Header, security_info: &SecurityInfo, source: &[u8]) -> Result<()> {
	let sec = header.security_offset as usize;
	let count = security_info.page_descriptor_count as usize;
	let pd_off = sec + 0x184;
	let page_size = page_size_for(security_info.image_info.image_flags) as usize;

	let mut expected: Sha1Hash = security_info.image_info.image_hash;
	let mut pe_cursor = 0usize;

	for i in 0..count {
		if pd_off + (i + 1) * 24 > source.len() {
			return Err(Xex2Error::PageDescriptorMismatch { descriptor_index: i }.into_report());
		}
		let desc_bytes: [u8; 24] = source[pd_off + i * 24..pd_off + (i + 1) * 24].try_into().unwrap();
		let page_count = u32::from_be_bytes(desc_bytes[0..4].try_into().unwrap()) >> 4;
		let declared_len = (page_count as usize) * page_size;
		let range_end = (pe_cursor + declared_len).min(pe.len());

		let computed = sha1_page_and_descriptor(&pe[pe_cursor..range_end], declared_len, &desc_bytes);
		if computed != expected {
			return Err(Xex2Error::PageDescriptorMismatch { descriptor_index: i }.into_report());
		}
		expected.0.copy_from_slice(&desc_bytes[4..24]);
		pe_cursor = range_end;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn roundtrip_bytes() {
		let d = PageDescriptor { page_count: 16, section_type: SectionType::ReadOnlyData, hash: Sha1Hash([0xAB; 20]) };
		let bytes = d.to_bytes();
		let parsed = PageDescriptor::from_bytes(&bytes);
		assert_eq!(d, parsed);
	}

	#[test]
	fn generate_chain_self_verifies() {
		// Construct a synthetic PE and template, generate descriptors, and
		// confirm the verification walk accepts them.
		let pe: Vec<u8> = (0..(4 * 0x10000)).map(|i| (i & 0xFF) as u8).collect();
		let template = &[
			DescriptorSlot { page_count: 2, section_type: SectionType::Code },
			DescriptorSlot { page_count: 2, section_type: SectionType::Data },
		];
		let GeneratedDescriptors { descriptors, image_hash } = generate(&pe, 0x10000, Some(template));

		let mut expected = image_hash;
		let mut cursor = 0usize;
		for d in &descriptors {
			let bytes = d.to_bytes();
			let declared = (d.page_count as usize) * 0x10000;
			let end = (cursor + declared).min(pe.len());
			let computed = sha1_page_and_descriptor(&pe[cursor..end], declared, &bytes);
			assert_eq!(computed, expected);
			expected = d.hash;
			cursor = end;
		}
	}

	#[test]
	fn default_template_single_descriptor() {
		let pe = vec![0u8; 64 * 1024 * 3];
		let descs = generate(&pe, 0x10000, None).descriptors;
		assert_eq!(descs.len(), 1);
		assert_eq!(descs[0].page_count, 3);
		assert_eq!(descs[0].section_type, SectionType::Code);
	}

	const BASE: VirtualAddress = VirtualAddress(0x9200_0000);
	const PAGE: u32 = 0x1000;

	fn slot(page_count: u32, section_type: SectionType) -> DescriptorSlot {
		DescriptorSlot { page_count, section_type }
	}

	fn range(address: u32, length: u32, section_type: SectionType) -> PageTypeRange {
		PageTypeRange { address: VirtualAddress(address), length, section_type }
	}

	fn extend(covered: u32, required: u32, ranges: &[PageTypeRange]) -> Result<Vec<DescriptorSlot>> {
		let mut template = vec![slot(covered, SectionType::ReadOnlyData)];
		extend_template(&mut template, required, PAGE, BASE, ranges)?;
		Ok(template[1..].to_vec())
	}

	fn error(covered: u32, required: u32, ranges: &[PageTypeRange]) -> crate::error::Report {
		extend(covered, required, ranges).unwrap_err()
	}

	#[test]
	fn untyped_append_keeps_legacy_runs() {
		// Unaligned source end: legacy runs of 16 pages, not 64 KiB splits.
		let slots = extend(0x14, 0x14 + 20, &[]).unwrap();
		assert_eq!(slots, vec![slot(16, SectionType::ReadOnlyData), slot(4, SectionType::ReadOnlyData)]);
	}

	#[test]
	fn typed_append_splits_at_type_changes() {
		let ranges = [range(0x9201_0000, 0x3000, SectionType::Code), range(0x9201_3000, 0x2000, SectionType::Data)];
		let slots = extend(0x10, 0x20, &ranges).unwrap();
		assert_eq!(
			slots,
			vec![slot(3, SectionType::Code), slot(2, SectionType::Data), slot(11, SectionType::ReadOnlyData)]
		);
	}

	#[test]
	fn typed_append_splits_at_64k_boundary() {
		// Source ends at 0x92008000; the code range spans 0x92010000.
		let ranges = [range(0x9200_8000, 0xC000, SectionType::Code)];
		let slots = extend(8, 0x18, &ranges).unwrap();
		assert_eq!(
			slots,
			vec![slot(8, SectionType::Code), slot(4, SectionType::Code), slot(4, SectionType::ReadOnlyData)]
		);
	}

	#[test]
	fn typed_append_caps_descriptor_pages() {
		let ranges = [range(0x9201_0000, 0x20000, SectionType::Data)];
		let slots = extend(0x10, 0x30, &ranges).unwrap();
		assert_eq!(slots, vec![slot(16, SectionType::Data), slot(16, SectionType::Data)]);
	}

	#[test]
	fn rejects_unaligned_range() {
		let ranges = [range(0x9201_0800, 0x1000, SectionType::Code)];
		assert!(matches!(
			error(0x10, 0x20, &ranges).current_context(),
			Xex2Error::PageTypeRangeAlignment { address: 0x9201_0800, .. }
		));
		let ranges = [range(0x9201_0000, 0x1800, SectionType::Code)];
		assert!(matches!(
			error(0x10, 0x20, &ranges).current_context(),
			Xex2Error::PageTypeRangeAlignment { length: 0x1800, .. }
		));
		let ranges = [range(0x9201_0000, 0, SectionType::Code)];
		assert!(matches!(
			error(0x10, 0x20, &ranges).current_context(),
			Xex2Error::PageTypeRangeAlignment { length: 0, .. }
		));
	}

	#[test]
	fn rejects_range_outside_appended_pages() {
		let ranges = [range(0x9200_F000, 0x2000, SectionType::Code)];
		assert!(matches!(
			error(0x10, 0x20, &ranges).current_context(),
			Xex2Error::PageTypeRangeOutsideAppended { appended_start: 0x9201_0000, appended_end: 0x9202_0000, .. }
		));
		let ranges = [range(0x9201_F000, 0x2000, SectionType::Code)];
		assert!(matches!(error(0x10, 0x20, &ranges).current_context(), Xex2Error::PageTypeRangeOutsideAppended { .. }));
	}

	#[test]
	fn rejects_overlapping_ranges() {
		let ranges = [range(0x9201_2000, 0x2000, SectionType::Data), range(0x9201_0000, 0x3000, SectionType::Code)];
		assert!(matches!(
			error(0x10, 0x20, &ranges).current_context(),
			Xex2Error::PageTypeRangeOverlap { first: 0x9201_0000, second: 0x9201_2000 }
		));
	}

	#[test]
	fn rejects_other_section_type() {
		let ranges = [range(0x9201_0000, 0x1000, SectionType::Other(5))];
		assert!(matches!(error(0x10, 0x20, &ranges).current_context(), Xex2Error::PageTypeRangeType { value: 5, .. }));
	}

	#[test]
	fn descriptor_slots_rejects_count_past_source() {
		let mut source = vec![0u8; 0x184 + 2 * 24];
		source[0x184..0x188].copy_from_slice(&0x11u32.to_be_bytes());
		assert_eq!(descriptor_slots(&source, 0, 2).unwrap().len(), 2);
		for (offset, count) in [(0, 3), (0, u32::MAX), (u32::MAX, 1)] {
			assert!(matches!(
				descriptor_slots(&source, offset, count).unwrap_err().current_context(),
				Xex2Error::PageDescriptorsTruncated
			));
		}
	}
}
