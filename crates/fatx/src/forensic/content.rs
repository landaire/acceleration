//! File types recognizable from their first bytes, and checks of recovered bytes against the
//! name and size a directory record gives them.

use std::path::Path;

/// Bytes at the start of a cluster that identification looks at.
pub const HEAD_SIZE: usize = 0x1000;

/// An STFS package's signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StfsKind {
	Con,
	Live,
	Pirs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContentKind {
	Xex2,
	Stfs(StfsKind),
	Bink,
	Xtaf,
	Png,
	Jpeg,
	Bmp,
	Riff,
	Dds,
	/// XPR texture bundle (`.xpr`, `.xbx`).
	Xpr,
	/// XACT wave bank (`.xwb`).
	WaveBank,
	/// XACT sound bank (`.xsb`).
	SoundBank,
	/// XDBF database, such as a gamer profile's `.gpd`.
	Xdbf,
	/// Printable ASCII. Recognized only when checking a recovered file, never when carving.
	Text,
}

/// The file size a header declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredSize {
	Exact(u64),
	/// The header is this long; the file is at least as long.
	AtLeast(u64),
	Unknown,
}

/// Header fields worth reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentDetail {
	None,
	Xex2 {
		image_size: Option<u32>,
	},
	/// `None` when the header is cut off before the name.
	Stfs {
		display_name: Option<String>,
	},
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentMatch {
	pub kind: ContentKind,
	pub size: DeclaredSize,
	pub detail: ContentDetail,
}

/// Whether a recovered file's bytes agree with what its directory record says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agreement {
	Agrees,
	/// A lower bound that the record's size meets.
	Consistent,
	Disagrees,
	/// Nothing to compare.
	Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentCheck {
	pub detected: Option<ContentMatch>,
	/// The type the record's file name extension implies.
	pub expected: Option<ContentKind>,
	pub kind: Agreement,
	pub size: Agreement,
}

/// The type of the file whose first bytes are `head`, by its binary signature.
pub fn identify(head: &[u8]) -> Option<ContentMatch> {
	let magic = head.get(..4)?;
	let found = |kind, size| Some(ContentMatch { kind, size, detail: ContentDetail::None });

	match magic {
		b"XEX2" => identify_xex2(head),
		b"CON " => identify_stfs(head, StfsKind::Con),
		b"LIVE" => identify_stfs(head, StfsKind::Live),
		b"PIRS" => identify_stfs(head, StfsKind::Pirs),
		[b'B', b'I', b'K', b'a'..=b'k'] | [b'K', b'B', b'2', b'a'..=b'k'] => {
			found(ContentKind::Bink, exact_le(head, 4, 8))
		}
		b"RIFF" => found(ContentKind::Riff, exact_le(head, 4, 8)),
		b"XTAF" => found(ContentKind::Xtaf, DeclaredSize::Unknown),
		b"\x89PNG" if head.get(4..8) == Some(b"\r\n\x1a\n") => found(ContentKind::Png, DeclaredSize::Unknown),
		[0xff, 0xd8, 0xff, _] => found(ContentKind::Jpeg, DeclaredSize::Unknown),
		b"DDS " => found(ContentKind::Dds, DeclaredSize::Unknown),
		b"XPR0" | b"XPR1" => found(ContentKind::Xpr, exact_le(head, 4, 0)),
		b"XPR2" => found(ContentKind::Xpr, xpr2_size(head)),
		b"DNBW" => found(ContentKind::WaveBank, wave_bank_size(head, u32::from_be_bytes)),
		b"WBND" => found(ContentKind::WaveBank, wave_bank_size(head, u32::from_le_bytes)),
		b"KBDS" | b"SDBK" => found(ContentKind::SoundBank, DeclaredSize::Unknown),
		b"XDBF" => found(ContentKind::Xdbf, DeclaredSize::Unknown),
		[b'B', b'M', _, _] => identify_bmp(head),
		_ => None,
	}
}

/// The type a file name's extension implies.
pub fn kind_from_name(name: &str) -> Option<ContentKind> {
	let extension = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();

	Some(match extension.as_str() {
		"xex" => ContentKind::Xex2,
		"bik" | "bk2" => ContentKind::Bink,
		"png" => ContentKind::Png,
		"jpg" | "jpeg" => ContentKind::Jpeg,
		"bmp" => ContentKind::Bmp,
		"wav" | "xma" => ContentKind::Riff,
		"dds" => ContentKind::Dds,
		"xpr" | "xbx" => ContentKind::Xpr,
		"xwb" => ContentKind::WaveBank,
		"xsb" => ContentKind::SoundBank,
		"gpd" => ContentKind::Xdbf,
		"fdf" => ContentKind::Xtaf,
		"txt" | "xml" | "ini" | "log" | "cfg" | "csv" | "json" | "htm" | "html" => ContentKind::Text,
		_ => return None,
	})
}

/// Checks recovered bytes against the type a record's file name implies, if any, and the
/// record's size. `head` is the start of the recovered file, at most the record's size.
pub fn check(expected: Option<ContentKind>, size: u64, head: &[u8]) -> ContentCheck {
	let detected = identify(head).or_else(|| {
		(expected == Some(ContentKind::Text) && is_text(head)).then_some(ContentMatch {
			kind: ContentKind::Text,
			size: DeclaredSize::Unknown,
			detail: ContentDetail::None,
		})
	});

	let kind = match (&detected, expected) {
		(Some(detected), Some(expected)) if detected.kind == expected => Agreement::Agrees,
		(Some(_), Some(_)) => Agreement::Disagrees,
		// Every type a name implies has a signature; its absence means the bytes are another type.
		(None, Some(_)) if !head.is_empty() => Agreement::Disagrees,
		_ => Agreement::Unknown,
	};
	let size = match detected.as_ref().map(|detected| detected.size) {
		Some(DeclaredSize::Exact(declared)) if declared == size => Agreement::Agrees,
		Some(DeclaredSize::Exact(_)) => Agreement::Disagrees,
		Some(DeclaredSize::AtLeast(minimum)) if minimum <= size => Agreement::Consistent,
		Some(DeclaredSize::AtLeast(_)) => Agreement::Disagrees,
		Some(DeclaredSize::Unknown) | None => Agreement::Unknown,
	};

	ContentCheck { detected, expected, kind, size }
}

fn is_text(head: &[u8]) -> bool {
	const SAMPLE: usize = 0x200;

	let sample = &head[..head.len().min(SAMPLE)];
	let sample = sample.strip_prefix(b"\xef\xbb\xbf").unwrap_or(sample);

	!sample.is_empty() && sample.iter().all(|byte| matches!(byte, b'\t' | b'\n' | b'\r' | 0x20..=0x7e))
}

fn read_le(head: &[u8], offset: usize) -> Option<u32> {
	head.get(offset..offset + 4).map(|bytes| u32::from_le_bytes(bytes.try_into().expect("four bytes")))
}

fn read_be(head: &[u8], offset: usize) -> Option<u32> {
	head.get(offset..offset + 4).map(|bytes| u32::from_be_bytes(bytes.try_into().expect("four bytes")))
}

/// A little-endian size field plus the bytes before the counted region.
fn exact_le(head: &[u8], offset: usize, adjustment: u64) -> DeclaredSize {
	match read_le(head, offset) {
		Some(size) => DeclaredSize::Exact(u64::from(size) + adjustment),
		None => DeclaredSize::Unknown,
	}
}

/// XEX2 header: the PE data offset at 0x8 is the header length; the security info, whose offset
/// is at 0x10, declares the loaded image size at its offset 4.
fn identify_xex2(head: &[u8]) -> Option<ContentMatch> {
	const MINIMUM_HEADER_SIZE: u32 = 0x18;

	let header_size = read_be(head, 0x8)?;
	if header_size < MINIMUM_HEADER_SIZE {
		return None;
	}
	let image_size =
		read_be(head, 0x10).and_then(|offset| read_be(head, usize::try_from(offset).ok()?.checked_add(4)?));

	Some(ContentMatch {
		kind: ContentKind::Xex2,
		size: DeclaredSize::AtLeast(u64::from(header_size)),
		detail: ContentDetail::Xex2 { image_size },
	})
}

/// STFS header: the header size at 0x340, the content size at 0x34c and the UTF-16 display name
/// at 0x411. The package holds the block-aligned header and at least the content.
fn identify_stfs(head: &[u8], kind: StfsKind) -> Option<ContentMatch> {
	const HEADER_SIZE_OFFSET: usize = 0x340;
	const CONTENT_SIZE_OFFSET: usize = 0x34c;
	const DISPLAY_NAME: std::ops::Range<usize> = 0x411..0x491;
	const BLOCK_SIZE: u64 = 0x1000;

	let content_size = head
		.get(CONTENT_SIZE_OFFSET..CONTENT_SIZE_OFFSET + 8)
		.map(|bytes| u64::from_be_bytes(bytes.try_into().expect("eight bytes")));
	let minimum = read_be(head, HEADER_SIZE_OFFSET).zip(content_size).and_then(|(header_size, content_size)| {
		u64::from(header_size).next_multiple_of(BLOCK_SIZE).checked_add(content_size)
	});
	let size = match minimum {
		Some(minimum) => DeclaredSize::AtLeast(minimum),
		None => DeclaredSize::Unknown,
	};
	let display_name = head.get(DISPLAY_NAME).map(|bytes| {
		let units: Vec<u16> = bytes
			.as_chunks::<2>()
			.0
			.iter()
			.map(|unit| u16::from_be_bytes(*unit))
			.take_while(|unit| *unit != 0)
			.collect();
		String::from_utf16_lossy(&units)
	});

	Some(ContentMatch { kind: ContentKind::Stfs(kind), size, detail: ContentDetail::Stfs { display_name } })
}

/// XPR2: big-endian header and data lengths at 4 and 8, after the 12 bytes that hold them.
fn xpr2_size(head: &[u8]) -> DeclaredSize {
	const FIXED_HEADER_SIZE: u64 = 12;

	match read_be(head, 4).zip(read_be(head, 8)) {
		Some((header, data)) => DeclaredSize::Exact(FIXED_HEADER_SIZE + u64::from(header) + u64::from(data)),
		None => DeclaredSize::Unknown,
	}
}

/// BMP: little-endian file size at 2 and a reserved zero field at 6.
fn identify_bmp(head: &[u8]) -> Option<ContentMatch> {
	const MINIMUM_SIZE: u32 = 26;

	let size = read_le(head, 2)?;
	if size < MINIMUM_SIZE || read_le(head, 6)? != 0 {
		return None;
	}

	Some(ContentMatch {
		kind: ContentKind::Bmp,
		size: DeclaredSize::Exact(u64::from(size)),
		detail: ContentDetail::None,
	})
}

/// XACT wave bank: a signature, the tool version, the header version, then five (offset, length)
/// regions; the file ends with the last region. Only header versions from 42, the layout seen on
/// Xbox 360 volumes, are sized.
fn wave_bank_size(head: &[u8], read: fn([u8; 4]) -> u32) -> DeclaredSize {
	const MINIMUM_HEADER_VERSION: u32 = 42;
	const REGIONS_OFFSET: usize = 0xc;
	const REGION_COUNT: usize = 5;

	let field = |offset: usize| head.get(offset..offset + 4).map(|bytes| read(bytes.try_into().expect("four bytes")));
	let (Some(version), Some(header_version)) = (field(4), field(8)) else {
		return DeclaredSize::Unknown;
	};
	if !(MINIMUM_HEADER_VERSION..=version).contains(&header_version) {
		return DeclaredSize::Unknown;
	}

	let Some(regions) = head.get(REGIONS_OFFSET..REGIONS_OFFSET + REGION_COUNT * 8) else {
		return DeclaredSize::Unknown;
	};
	let end = regions
		.as_chunks::<8>()
		.0
		.iter()
		.map(|region| {
			let offset = read(region[..4].try_into().expect("four bytes"));
			let length = read(region[4..].try_into().expect("four bytes"));
			u64::from(offset) + u64::from(length)
		})
		.max();

	match end {
		Some(end) => DeclaredSize::Exact(end),
		None => DeclaredSize::Unknown,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn bink_size_counts_the_first_eight_bytes() {
		let mut head = b"BIKi".to_vec();
		head.extend_from_slice(&0x1000u32.to_le_bytes());

		let found = identify(&head).unwrap();

		assert_eq!(found.kind, ContentKind::Bink);
		assert_eq!(found.size, DeclaredSize::Exact(0x1008));
	}

	#[test]
	fn big_endian_wave_bank_ends_with_its_last_region() {
		let mut head = b"DNBW".to_vec();
		for value in [0x2b, 0x2a, 0x34, 0x60, 0x94, 0x78, 0x10c, 0x3c, 0x148, 0x140, 0x800, 0x6800u32] {
			head.extend_from_slice(&value.to_be_bytes());
		}

		assert_eq!(identify(&head).unwrap().size, DeclaredSize::Exact(0x7000));
	}

	#[test]
	fn check_reports_a_misnamed_file() {
		let check = check(kind_from_name("background1.bmp"), 46793, b"\xff\xd8\xff\xe0\x00\x10JFIF");

		assert_eq!(check.expected, Some(ContentKind::Bmp));
		assert_eq!(check.detected.map(|detected| detected.kind), Some(ContentKind::Jpeg));
		assert_eq!(check.kind, Agreement::Disagrees);
		assert_eq!(check.size, Agreement::Unknown);
	}

	#[test]
	fn check_recognizes_text_only_when_the_name_expects_it() {
		assert_eq!(check(kind_from_name("notes.txt"), 5, b"hello").kind, Agreement::Agrees);
		assert_eq!(check(kind_from_name("notes.bin"), 5, b"hello").detected, None);
	}

	#[test]
	fn rejects_unknown_bink_revisions() {
		assert_eq!(identify(b"BIKX\0\0\0\0"), None);
	}

	#[test]
	fn xpr2_size_counts_its_fixed_header() {
		let mut head = b"XPR2".to_vec();
		head.extend_from_slice(&0x800u32.to_be_bytes());
		head.extend_from_slice(&0x2000u32.to_be_bytes());

		assert_eq!(identify(&head).unwrap().size, DeclaredSize::Exact(0x280c));
	}

	#[test]
	fn wave_bank_without_header_version_is_not_sized() {
		let mut head = b"DNBW".to_vec();
		for value in [0x20, 0x34, 0x60, 0x94, 0x78, 0x10c, 0x3c, 0x148, 0x140, 0x800, 0x6800u32, 0] {
			head.extend_from_slice(&value.to_be_bytes());
		}

		assert_eq!(identify(&head).unwrap().size, DeclaredSize::Unknown);
	}
}
