mod common;

use common::RawImage;
use common::header;
use sfcx::BootChain;
use sfcx::ChainKind;
use sfcx::FlashImage;
use sfcx::FormatSelection;
use sfcx::ImageFormat;
use sfcx::ParseError;
use sfcx::PatchSlotContents;
use sfcx::RetailCb;
use sfcx::SfcxError;
use sfcx::StageName;
use sfcx::StageRole;
use sfcx::io::SliceSource;
use sfcx::raw::RawKind;

const SMALL_BLOCK_LOGICAL_SIZE: usize = 992 * 0x4000;
const BIG_BLOCK_LOGICAL_SIZE: usize = 480 * 0x2_0000;
const BOOT_SIZE: usize = 0x7_0000;
const PATCH_SLOTS: usize = 0x7_0000;
const PATCH_SLOT_SIZE: usize = 0x1_0000;
const SPLIT: u16 = 0x0800;

/// A retail flash header and boot chain in a logical device of `size` bytes: `stages` from
/// 0x8000, each at the end of the previous one rounded up to 16 bytes, and kernel patch slots
/// after the boot region with CF and CG in the first slot.
fn retail_logical(size: usize, stages: &[([u8; 2], u16, u32)]) -> Vec<u8> {
	let mut logical = vec![0xffu8; size];
	logical[..BOOT_SIZE].fill(0);
	logical[..16].copy_from_slice(&header(*b"\xff\x4f", 1888, 0, BOOT_SIZE as u32));
	logical[8..12].copy_from_slice(&0x8000u32.to_be_bytes());
	let fields = [(0x64, PATCH_SLOTS as u32), (0x6c, 0x4000), (0x70, 0), (0x78, 0x3000), (0x7c, 0x1000)];
	for (offset, value) in fields {
		logical[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
	}

	let mut offset = 0x8000;
	for (name, flags, size) in stages.iter().copied() {
		logical[offset..offset + 16].copy_from_slice(&header(name, 9188, flags, size));
		offset += (size as usize).next_multiple_of(16);
	}
	logical[PATCH_SLOTS..PATCH_SLOTS + 16].copy_from_slice(&header(*b"CF", 17559, 0, 0x234));
	logical[PATCH_SLOTS + 0x240..PATCH_SLOTS + 0x250].copy_from_slice(&header(*b"CG", 17559, 0, 0x3000));
	logical
}

fn split_chain() -> [([u8; 2], u16, u32); 4] {
	[(*b"CB", SPLIT, 0x404), (*b"CB", 0, 0x2000), (*b"CD", 0, 0x1000), (*b"CE", 0, 0x5004)]
}

fn single_chain() -> [([u8; 2], u16, u32); 3] {
	[(*b"CB", 0, 0x2400), (*b"CD", 0, 0x1000), (*b"CE", 0, 0x5004)]
}

/// The roles, offsets and sizes of the image's boot chain.
fn layout(image: &FlashImage) -> Vec<(StageRole, usize, usize)> {
	let region = image.boot_region();
	region.chain().stages().map(|(role, stage)| (role, stage.offset().get(), stage.size().get())).collect()
}

/// A split CB places CB_B at the end of CB_A rounded up to 16 bytes, and both are named "CB".
#[test]
fn logical_retail_image_has_a_split_cb_and_patch_slots() {
	let logical = retail_logical(SMALL_BLOCK_LOGICAL_SIZE, &split_chain());
	let source = SliceSource(logical.as_slice());
	let image =
		FlashImage::open(&source, logical.len() as u64, FormatSelection::Auto).unwrap().deny_warnings().unwrap();

	assert_eq!(image.format(), ImageFormat::Logical);
	let region = image.boot_region();
	assert_eq!(region.chain().kind(), ChainKind::Retail);
	let BootChain::Retail { cb: RetailCb::Split { cb_a, cb_b }, .. } = *region.chain() else {
		panic!("expected a split CB, got {:?}", region.chain());
	};
	assert_eq!((cb_a.name(), cb_b.name()), (StageName::CB, StageName::CB));
	assert_eq!(cb_a.flags(), SPLIT);
	assert_eq!(
		layout(&image),
		[
			(StageRole::CbA, 0x8000, 0x404),
			(StageRole::CbB, 0x8410, 0x2000),
			(StageRole::Cd, 0xa410, 0x1000),
			(StageRole::Ce, 0xb410, 0x5004),
		]
	);
	assert_eq!(region.boot_stage_data(&cb_b)[..2], *b"CB");
	assert_eq!(region.key_vault(), Ok(Some(0x4000..0x8000)));

	let slots = image.patch_slots(&source).unwrap().deny_warnings().unwrap();
	assert_eq!(slots.len(), 2);
	assert_eq!((slots[0].offset().get(), slots[0].size().get()), (PATCH_SLOTS, PATCH_SLOT_SIZE));
	let PatchSlotContents::Loaders { cf, cg: Some(cg) } = slots[0].contents() else {
		panic!("expected CF and CG, got {:?}", slots[0].contents());
	};
	assert_eq!((cf.name(), cf.offset().get(), cf.size().get()), (StageName::CF, 0, 0x234));
	assert_eq!((cg.name(), cg.offset().get(), cg.size().get()), (StageName::CG, 0x240, 0x3000));
	assert_eq!(slots[1].offset().get(), PATCH_SLOTS + PATCH_SLOT_SIZE);
	assert_eq!(slots[1].contents(), PatchSlotContents::Empty);
}

/// A raw 16 MiB small-block image opens with its geometry detected from the spare areas.
#[test]
fn raw_small_block_retail_image_opens() {
	for (chain, expected) in [(split_chain().to_vec(), 4), (single_chain().to_vec(), 3)] {
		let logical = retail_logical(SMALL_BLOCK_LOGICAL_SIZE, &chain);
		let raw = RawImage::new(RawKind::SmallBlock, 1024, &logical).finish();
		let source = SliceSource(raw.as_slice());
		let image =
			FlashImage::open(&source, raw.len() as u64, FormatSelection::Auto).unwrap().deny_warnings().unwrap();

		let ImageFormat::Raw(geometry) = image.format() else {
			panic!("raw image detected as {}", image.format());
		};
		assert_eq!(geometry.kind(), RawKind::SmallBlock);
		assert_eq!(layout(&image).len(), expected);
		assert_eq!(image.boot_region().bytes(), &logical[..BOOT_SIZE]);
		let slots = image.patch_slots(&source).unwrap().deny_warnings().unwrap();
		assert!(matches!(slots[0].contents(), PatchSlotContents::Loaders { cg: Some(_), .. }));
	}
}

/// A raw 64 MiB big-block image with a single CB.
#[test]
fn raw_big_block_retail_image_opens() {
	let logical = retail_logical(BIG_BLOCK_LOGICAL_SIZE, &single_chain());
	let raw = RawImage::new(RawKind::BigBlock, 512, &logical).finish();
	let source = SliceSource(raw.as_slice());
	let image = FlashImage::open(&source, raw.len() as u64, FormatSelection::Auto).unwrap().deny_warnings().unwrap();

	let ImageFormat::Raw(geometry) = image.format() else {
		panic!("raw image detected as {}", image.format());
	};
	assert_eq!(geometry.kind(), RawKind::BigBlock);
	assert!(matches!(image.boot_region().chain(), BootChain::Retail { cb: RetailCb::Single(_), .. }));
	assert_eq!(
		layout(&image),
		[(StageRole::Cb, 0x8000, 0x2400), (StageRole::Cd, 0xa400, 0x1000), (StageRole::Ce, 0xb400, 0x5004)]
	);
}

/// A chain that starts with neither "SB" nor "CB", and chains that break off, name the stages in
/// quotes.
#[test]
fn malformed_chains_name_their_stages() {
	let open = |stages: &[([u8; 2], u16, u32)]| {
		let logical = retail_logical(BOOT_SIZE + 2 * PATCH_SLOT_SIZE, stages);
		let source = SliceSource(logical.as_slice());
		match FlashImage::open(&source, logical.len() as u64, FormatSelection::Logical) {
			Ok(_) => panic!("{stages:?} opened"),
			Err(SfcxError::Parse(error)) => error,
			Err(error) => panic!("unexpected error {error}"),
		}
	};

	let unknown = open(&[(*b"XB", 0, 0x10)]);
	assert_eq!(unknown, ParseError::UnknownBootChain { offset: 0x8000, name: StageName::new(*b"XB") });
	let missing_cb_b = open(&[(*b"CB", SPLIT, 0x10), (*b"CD", 0, 0x10), (*b"CE", 0, 0x10)]);
	let devkit_mix = open(&[(*b"SB", 0, 0x10), (*b"CD", 0, 0x10)]);
	let name = open(&[(*b"C\x00", 0, 0x10)]);
	let messages = [unknown, missing_cb_b, devkit_mix, name].map(|error| error.to_string());
	assert_eq!(
		messages,
		[
			"boot stage \"XB\" at 0x8000 starts neither a devkit chain (\"SB\") nor a retail chain (\"CB\")",
			"expected boot stage \"CB\" at 0x8010, got \"CD\"",
			"expected boot stage \"SC\" at 0x8010, got \"CD\"",
			"boot stage \"C\\x00\" at 0x8000 starts neither a devkit chain (\"SB\") nor a retail chain (\"CB\")",
		]
	);
}

/// A patch slot that does not start with CF is reported when the slots are read.
#[test]
fn patch_slot_must_start_with_cf() {
	let mut logical = retail_logical(SMALL_BLOCK_LOGICAL_SIZE, &single_chain());
	logical[PATCH_SLOTS + PATCH_SLOT_SIZE..PATCH_SLOTS + PATCH_SLOT_SIZE + 16]
		.copy_from_slice(&header(*b"CE", 1888, 0, 0x10));
	let source = SliceSource(logical.as_slice());
	let image =
		FlashImage::open(&source, logical.len() as u64, FormatSelection::Logical).unwrap().deny_warnings().unwrap();

	let error = image.patch_slots(&source).unwrap_err();
	assert_eq!(error.to_string(), "expected boot stage \"CF\" at 0x80000, got \"CE\"");
}

/// A shadowboot ROM can put its chain right after the 0x80-byte flash header, with no SMC and no
/// declared boot size. The boot region is then the whole image.
#[test]
fn compact_rom_without_boot_size_or_smc_opens() {
	let mut rom = vec![0u8; 0x80];
	rom[..16].copy_from_slice(&header(*b"\xff\x4f", 17559, 0, 0));
	rom[8..12].copy_from_slice(&0x80u32.to_be_bytes());
	for (name, size) in [(*b"SB", 0x4140), (*b"SC", 0x5bf0), (*b"SD", 0x4de0), (*b"SE", 0x372)] {
		let mut stage = vec![0u8; size];
		stage[..16].copy_from_slice(&header(name, 1, 0, size as u32));
		rom.extend_from_slice(&stage);
	}
	rom.resize(0xf000, 0);
	let source = SliceSource(rom.as_slice());
	let image = FlashImage::open(&source, rom.len() as u64, FormatSelection::Auto).unwrap().deny_warnings().unwrap();

	let region = image.boot_region();
	assert_eq!(region.header().declared_boot_size(), None);
	assert_eq!(region.bytes().len(), rom.len());
	assert_eq!(region.smc(), Ok(None));
	assert_eq!(region.chain().kind(), ChainKind::Devkit);
	let offsets: Vec<usize> = region.chain().stages().map(|(_, stage)| stage.offset().get()).collect();
	assert_eq!(offsets, [0x80, 0x41c0, 0x9db0, 0xeb90]);
}

/// Loaders are matched by the IDs the hypervisor checks: a CB of the extra key type is named
/// "Cb", and an optional CC sits between CB_B and CD. Stored names are kept for display.
#[test]
fn retail_loaders_match_by_id_and_cc_is_optional() {
	let chain =
		[(*b"Cb", SPLIT, 0x400), (*b"CB", 0, 0x400), (*b"CC", 0, 0x200), (*b"CD", 0, 0x400), (*b"CE", 0, 0x400)];
	let logical = retail_logical(BOOT_SIZE + 2 * PATCH_SLOT_SIZE, &chain);
	let source = SliceSource(logical.as_slice());
	let image =
		FlashImage::open(&source, logical.len() as u64, FormatSelection::Logical).unwrap().deny_warnings().unwrap();

	assert_eq!(
		layout(&image),
		[
			(StageRole::CbA, 0x8000, 0x400),
			(StageRole::CbB, 0x8400, 0x400),
			(StageRole::Cc, 0x8800, 0x200),
			(StageRole::Cd, 0x8a00, 0x400),
			(StageRole::Ce, 0x8e00, 0x400),
		]
	);
	let region = image.boot_region();
	let BootChain::Retail { cb: RetailCb::Split { cb_a, .. }, cc: Some(cc), .. } = *region.chain() else {
		panic!("expected a split CB and a CC, got {:?}", region.chain());
	};
	assert_eq!((cb_a.name(), cc.name()), (StageName::new(*b"Cb"), StageName::CC));
	assert_eq!(region.chain().stage(StageRole::Cc), Some(cc));
}

/// Both patch slots must lie inside the 0x8000000-byte flash address space
/// (`XeKeysSaveSystemUpdate`).
#[test]
fn patch_slots_must_fit_the_flash_address_space() {
	let mut logical = retail_logical(BOOT_SIZE + 2 * PATCH_SLOT_SIZE, &single_chain());
	let mut slots = |offset: u32, size: u32| {
		logical[0x64..0x68].copy_from_slice(&offset.to_be_bytes());
		logical[0x70..0x74].copy_from_slice(&size.to_be_bytes());
		let source = SliceSource(logical.as_slice());
		let image =
			FlashImage::open(&source, logical.len() as u64, FormatSelection::Logical).unwrap().ignore_warnings();
		image.boot_region().patch_slots().map(|slots| slots.map(|slots| (slots.offset().get(), slots.size().get())))
	};

	assert_eq!(slots(0x7fc_0000, 0x1_0000), Ok(Some((0x7fc_0000, 0x1_0000))));
	assert_eq!(slots(0x7fe_0000, 0x1_0000), Err(ParseError::InvalidPatchSlots { offset: 0x7fe_0000, size: 0x1_0000 }));
	assert_eq!(slots(0x800_0000, 0), Err(ParseError::InvalidPatchSlots { offset: 0x800_0000, size: 0x1_0000 }));
	assert_eq!(slots(0, 0x2_0000), Ok(None));
}
