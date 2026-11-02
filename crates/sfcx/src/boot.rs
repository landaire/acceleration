//! The flash header, the SMC firmware and kernel patch slots that it locates, and the boot chain
//! that follows it: SB, SC, SD and SE on a devkit, CB (or CB_A and CB_B), CD and CE on a retail
//! console.

use std::fmt;
use std::ops::Range;

use winnow::Parser;
use winnow::binary::be_u16;
use winnow::binary::be_u32;
use winnow::error::ModalResult;
use winnow::token::take;

use crate::checked::Checked;
use crate::error::ParseError;
use crate::error::SfcxError;
use crate::image::FlashImage;
use crate::io::AsyncReadAt;
use crate::io::ReadAt;

pub const IMAGE_HEADER_SIZE: usize = 16;
const FLASH_MAGIC: [u8; 2] = [0xff, 0x4f];
/// The flash header area: the image header, the copyright string, and the location fields
/// (`XE_FLASH_HEADER`).
pub const FLASH_HEADER_SIZE: usize = 0x80;
/// Offsets in the flash header of big-endian u32 location fields.
const PATCH_SLOT_OFFSET_FIELD: usize = 0x64;
const KEY_VAULT_OFFSET_FIELD: usize = 0x6c;
const PATCH_SLOT_SIZE_FIELD: usize = 0x70;
const SMC_LENGTH_FIELD: usize = 0x78;
const SMC_OFFSET_FIELD: usize = 0x7c;
/// The key vault that the flash header locates is this long (`HV_KEY_VAULT`).
pub const KEY_VAULT_SIZE: usize = 0x4000;
/// A patch slot size field of zero means this size (`XEKEYS_PATCH_SLOT_SIZE_MIN`).
const DEFAULT_PATCH_SLOT_SIZE: usize = 0x1_0000;
/// The hypervisor places each loader at its predecessor's size rounded up to this
/// (`HV_BOOT_LOADER_ALIGNMENT`).
pub const STAGE_ALIGNMENT: usize = 16;
/// CB flag: a second CB block follows (`HV_BOOT_LOADER_FLAG_SPLIT`).
const SPLIT_FLAG: u16 = 0x0800;
/// The magic bits that name a devkit chain: an `S` where a retail chain has a `C`
/// (`HV_CB_CONSOLE_KEY_MASK`).
const CONSOLE_KEY_MASK: u16 = 0xf000;
const DEVKIT_CONSOLE_KEY: u16 = 0x5000;

/// The loader that a magic names, as the hypervisor matches it (`bootloader.h`): a CB under
/// `HV_CB_MAGIC_MASK`, which ignores the key type and console key bits, and the later loaders
/// under the low 12 bits (`HV_BOOT_LOADER_MAGIC_ID_MASK`). `name` is the name an error expects.
#[derive(Debug, Clone, Copy)]
struct LoaderId {
	mask: u16,
	id: u16,
	name: StageName,
}

impl LoaderId {
	const CB: Self = Self { mask: 0x0f0f, id: 0x0302, name: StageName::CB };
	const SB: Self = Self { name: StageName::SB, ..Self::CB };
	const CB_B: Self = Self { mask: 0x0fff, id: 0x0342, name: StageName::CB };
	const CC: Self = Self { mask: 0x0fff, id: 0x0343, name: StageName::CC };
	const SC: Self = Self { name: StageName::SC, ..Self::CC };
	const CD: Self = Self { mask: 0x0fff, id: 0x0344, name: StageName::CD };
	const SD: Self = Self { name: StageName::SD, ..Self::CD };
	const CE: Self = Self { mask: 0x0fff, id: 0x0345, name: StageName::CE };
	const SE: Self = Self { name: StageName::SE, ..Self::CE };

	const fn matches(self, name: StageName) -> bool {
		u16::from_be_bytes(name.0) & self.mask == self.id
	}
}

/// The two magic bytes of a boot stage header, such as `SB` or `CB`. Displayed as quoted ASCII
/// with other bytes escaped.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct StageName([u8; 2]);

impl StageName {
	pub const SB: Self = Self(*b"SB");
	pub const SC: Self = Self(*b"SC");
	pub const SD: Self = Self(*b"SD");
	pub const SE: Self = Self(*b"SE");
	pub const CB: Self = Self(*b"CB");
	pub const CC: Self = Self(*b"CC");
	pub const CD: Self = Self(*b"CD");
	pub const CE: Self = Self(*b"CE");
	pub const CF: Self = Self(*b"CF");
	pub const CG: Self = Self(*b"CG");

	pub const fn new(bytes: [u8; 2]) -> Self {
		Self(bytes)
	}

	pub const fn bytes(self) -> [u8; 2] {
		self.0
	}
}

impl fmt::Display for StageName {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(formatter, "\"{}\"", self.0.escape_ascii())
	}
}

impl fmt::Debug for StageName {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		fmt::Display::fmt(self, formatter)
	}
}

/// The place of a boot stage in its chain. A split retail CB is two stages that are both named
/// `CB`; the role tells them apart.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum StageRole {
	Sb,
	Sc,
	Sd,
	Se,
	/// A retail CB that is not split.
	Cb,
	/// The first block of a split retail CB.
	CbA,
	/// The second block of a split retail CB.
	CbB,
	/// The optional retail loader between CB and CD.
	Cc,
	Cd,
	Ce,
}

impl StageRole {
	pub const ALL: [Self; 10] =
		[Self::Sb, Self::Sc, Self::Sd, Self::Se, Self::Cb, Self::CbA, Self::CbB, Self::Cc, Self::Cd, Self::Ce];

	/// The unambiguous name of the role: the stage name, or `CB_A` and `CB_B` for a split CB.
	pub const fn label(self) -> &'static str {
		match self {
			Self::Sb => "SB",
			Self::Sc => "SC",
			Self::Sd => "SD",
			Self::Se => "SE",
			Self::Cb => "CB",
			Self::CbA => "CB_A",
			Self::CbB => "CB_B",
			Self::Cc => "CC",
			Self::Cd => "CD",
			Self::Ce => "CE",
		}
	}

	/// The stage name the role's header usually carries. The hypervisor matches loaders by ID
	/// bits only, so a stage may carry another name; [`BootStage::name`] is the stored one.
	pub const fn name(self) -> StageName {
		match self {
			Self::Sb => StageName::SB,
			Self::Sc => StageName::SC,
			Self::Sd => StageName::SD,
			Self::Se => StageName::SE,
			Self::Cb | Self::CbA | Self::CbB => StageName::CB,
			Self::Cc => StageName::CC,
			Self::Cd => StageName::CD,
			Self::Ce => StageName::CE,
		}
	}

	/// Whether a stage named `name` can take this role: the hypervisor's loader ID matches.
	pub const fn matches(self, name: StageName) -> bool {
		let loader = match self {
			Self::Sb | Self::Cb | Self::CbA => LoaderId::CB,
			Self::CbB => LoaderId::CB_B,
			Self::Sc | Self::Cc => LoaderId::CC,
			Self::Sd | Self::Cd => LoaderId::CD,
			Self::Se | Self::Ce => LoaderId::CE,
		};
		loader.matches(name)
	}

	/// The role whose label is `label`, ignoring ASCII case.
	pub fn parse(label: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|role| role.label().eq_ignore_ascii_case(label))
	}
}

impl fmt::Display for StageRole {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(formatter, "\"{}\"", self.label())
	}
}

impl fmt::Debug for StageRole {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		fmt::Display::fmt(self, formatter)
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileOffset(usize);

impl FileOffset {
	pub const fn new(value: usize) -> Self {
		Self(value)
	}

	pub const fn get(self) -> usize {
		self.0
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteCount(usize);

impl ByteCount {
	pub const fn new(value: usize) -> Self {
		Self(value)
	}

	pub const fn get(self) -> usize {
		self.0
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildNumber(u16);

impl BuildNumber {
	pub const fn get(self) -> u16 {
		self.0
	}
}

/// The 16-byte header of the flash image and of every boot loader (`HV_BOOT_LOADER_HEADER`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageHeader {
	pub magic: [u8; 2],
	pub build: BuildNumber,
	pub qfe: u16,
	pub flags: u16,
	pub entry: u32,
	pub boot_image_size: ByteCount,
}

impl ImageHeader {
	/// Parses the 16-byte flash header and checks its magic.
	pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
		let mut input = bytes.get(..IMAGE_HEADER_SIZE).ok_or(ParseError::HeaderTruncated)?;
		let header = parse_header_fields(&mut input).map_err(|_| ParseError::HeaderTruncated)?;
		if header.magic != FLASH_MAGIC {
			return Err(ParseError::UnsupportedMagic { magic: header.magic });
		}

		Ok(header)
	}

	/// The boot region size of a flash header, or `None` when the field is zero. Shadowboot ROMs
	/// such as freeboot's leave it zero: the kernel passes the file size to the hypervisor
	/// (`ExpTryToBootMediaKernel`), whose shadowboot path reads only the magic, the entry point
	/// and the SB header (`HvpShadowBootStart`).
	pub const fn declared_boot_size(&self) -> Option<ByteCount> {
		match self.boot_image_size.0 {
			0 => None,
			_ => Some(self.boot_image_size),
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootStage {
	name: StageName,
	build: BuildNumber,
	qfe: u16,
	flags: u16,
	entry: u32,
	offset: FileOffset,
	size: ByteCount,
}

impl BootStage {
	pub const fn name(&self) -> StageName {
		self.name
	}

	pub const fn build(&self) -> BuildNumber {
		self.build
	}

	pub const fn qfe(&self) -> u16 {
		self.qfe
	}

	pub const fn flags(&self) -> u16 {
		self.flags
	}

	pub const fn entry(&self) -> u32 {
		self.entry
	}

	pub const fn offset(&self) -> FileOffset {
		self.offset
	}

	pub const fn size(&self) -> ByteCount {
		self.size
	}

	/// Offset where the next loader starts: the end of this one, rounded up to
	/// [`STAGE_ALIGNMENT`].
	pub const fn aligned_end(&self) -> usize {
		self.offset.0 + self.size.0.next_multiple_of(STAGE_ALIGNMENT)
	}
}

/// Whether a boot chain belongs to a development kit or a retail console. The first letter of
/// the stage names is the console key nibble of the CB magic (`HV_CB_CONSOLE_KEY_MASK`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainKind {
	Devkit,
	Retail,
}

impl fmt::Display for ChainKind {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(match self {
			Self::Devkit => "devkit",
			Self::Retail => "retail",
		})
	}
}

/// The CB of a retail chain: one block, or two blocks named `CB` when the first has the split
/// flag (`HV_BOOT_LOADER_FLAG_SPLIT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetailCb {
	Single(BootStage),
	Split { cb_a: BootStage, cb_b: BootStage },
}

/// The boot loaders after the flash header, in flash order. Retail stages are stored encrypted
/// and are kept as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootChain {
	Devkit { sb: BootStage, sc: BootStage, sd: BootStage, se: BootStage },
	Retail { cb: RetailCb, cc: Option<BootStage>, cd: BootStage, ce: BootStage },
}

impl BootChain {
	pub const fn kind(&self) -> ChainKind {
		match self {
			Self::Devkit { .. } => ChainKind::Devkit,
			Self::Retail { .. } => ChainKind::Retail,
		}
	}

	/// The stages with their roles, in flash order.
	pub fn stages(&self) -> impl Iterator<Item = (StageRole, BootStage)> {
		let stages = match *self {
			Self::Devkit { sb, sc, sd, se } => [
				Some((StageRole::Sb, sb)),
				Some((StageRole::Sc, sc)),
				Some((StageRole::Sd, sd)),
				Some((StageRole::Se, se)),
				None,
			],
			Self::Retail { cb, cc, cd, ce } => {
				let (first, second) = match cb {
					RetailCb::Single(cb) => ((StageRole::Cb, cb), None),
					RetailCb::Split { cb_a, cb_b } => ((StageRole::CbA, cb_a), Some((StageRole::CbB, cb_b))),
				};
				[
					Some(first),
					second,
					cc.map(|cc| (StageRole::Cc, cc)),
					Some((StageRole::Cd, cd)),
					Some((StageRole::Ce, ce)),
				]
			}
		};

		stages.into_iter().flatten()
	}

	pub fn stage(&self, role: StageRole) -> Option<BootStage> {
		self.stages().find_map(|(candidate, stage)| (candidate == role).then_some(stage))
	}

	/// The last stage: SE or CE, which holds the hypervisor and the kernel.
	pub const fn last(&self) -> BootStage {
		match self {
			Self::Devkit { se, .. } => *se,
			Self::Retail { ce, .. } => *ce,
		}
	}
}

/// The location of the encrypted SMC firmware in the boot region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmcImage {
	offset: FileOffset,
	size: ByteCount,
}

impl SmcImage {
	pub const fn offset(&self) -> FileOffset {
		self.offset
	}

	pub const fn size(&self) -> ByteCount {
		self.size
	}
}

/// The two kernel patch slots that the flash header locates. The second slot follows the first
/// (`XeKeysSaveSystemUpdate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchSlotLocations {
	offset: FileOffset,
	size: ByteCount,
}

impl PatchSlotLocations {
	pub const COUNT: usize = 2;

	pub const fn offset(&self) -> FileOffset {
		self.offset
	}

	/// Size of one slot.
	pub const fn size(&self) -> ByteCount {
		self.size
	}

	/// Logical range of slot `index`.
	pub const fn range(&self, index: usize) -> Range<u64> {
		let start = (self.offset.0 + index * self.size.0) as u64;
		start..start + self.size.0 as u64
	}

	/// Logical range of both slots.
	pub const fn ranges(&self) -> Range<u64> {
		let start = self.offset.0 as u64;
		start..start + (Self::COUNT * self.size.0) as u64
	}
}

/// What a kernel patch slot holds: nothing, or the CF update loader and the CG patch after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchSlotContents {
	/// The slot starts with an erased or zero header.
	Empty,
	Loaders {
		cf: BootStage,
		cg: Option<BootStage>,
	},
}

/// A kernel patch slot. Loader offsets are relative to the slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatchSlot {
	offset: FileOffset,
	size: ByteCount,
	contents: PatchSlotContents,
}

impl PatchSlot {
	/// Parses the bytes of a slot that starts at logical `offset`. Errors name logical offsets.
	pub fn parse(offset: FileOffset, bytes: &[u8]) -> Result<Self, ParseError> {
		let size = ByteCount::new(bytes.len());
		let header =
			bytes.get(..IMAGE_HEADER_SIZE).ok_or(ParseError::BootStageHeaderTruncated { offset: offset.get() })?;
		if header.iter().all(|byte| *byte == 0xff) || header.iter().all(|byte| *byte == 0) {
			return Ok(Self { offset, size, contents: PatchSlotContents::Empty });
		}

		let absolute = |error| match error {
			ParseError::BootStageHeaderTruncated { offset: within } => {
				ParseError::BootStageHeaderTruncated { offset: offset.get() + within }
			}
			error => error,
		};
		let cf = parse_boot_stage(bytes, 0).map_err(absolute)?;
		if cf.name != StageName::CF {
			return Err(ParseError::UnexpectedBootStage {
				offset: offset.get(),
				expected: StageName::CF,
				actual: cf.name,
			});
		}
		let next = cf.aligned_end();
		let cg = match bytes.get(next..next + IMAGE_HEADER_SIZE) {
			Some(header) if header.starts_with(&StageName::CG.bytes()) => {
				Some(parse_boot_stage(bytes, next).map_err(absolute)?)
			}
			Some(_) | None => None,
		};

		Ok(Self { offset, size, contents: PatchSlotContents::Loaders { cf, cg } })
	}

	pub const fn offset(&self) -> FileOffset {
		self.offset
	}

	pub const fn size(&self) -> ByteCount {
		self.size
	}

	pub const fn contents(&self) -> PatchSlotContents {
		self.contents
	}
}

/// The flash header and the boot chain of a flash image, limited to the declared boot region.
#[derive(Debug, Clone, Copy)]
pub struct BootRegion<'a> {
	bytes: &'a [u8],
	header: ImageHeader,
	chain: BootChain,
}

impl<'a> BootRegion<'a> {
	/// Parses the boot region of a logical image: the flash header's declared boot size, or all
	/// of `bytes` when it declares none. Bytes past the declared boot region are ignored.
	///
	/// Stages are matched by the loader IDs the hypervisor checks, not by their full names. The
	/// first stage must be a CB; an `S` in its console key bits (`SB`) starts a devkit chain of
	/// SB, SC, SD and SE, and anything else a retail chain of CB (or CB_A and CB_B), an optional
	/// CC, CD and CE (`HvpVerifyFlashBootLoader`, `HvxKeysSaveBootLoader`).
	pub fn parse(bytes: &'a [u8]) -> Result<Self, ParseError> {
		let header = ImageHeader::parse(bytes)?;
		let declared = header.declared_boot_size().map_or(bytes.len(), ByteCount::get);
		let bytes = bytes
			.get(..declared)
			.ok_or(ParseError::BootRegionTruncated { declared: declared as u64, actual: bytes.len() as u64 })?;

		let mut cursor = ChainCursor { bytes, offset: header.entry as usize };
		let first = cursor.peek()?;
		if !LoaderId::CB.matches(first.name) {
			return Err(ParseError::UnknownBootChain { offset: cursor.offset, name: first.name });
		}
		let chain = if u16::from_be_bytes(first.name.0) & CONSOLE_KEY_MASK == DEVKIT_CONSOLE_KEY {
			let sb = cursor.expect(LoaderId::SB)?;
			let sc = cursor.expect(LoaderId::SC)?;
			let sd = cursor.expect(LoaderId::SD)?;
			let se = cursor.expect(LoaderId::SE)?;
			BootChain::Devkit { sb, sc, sd, se }
		} else {
			let cb_a = cursor.expect(LoaderId::CB)?;
			let cb = if cb_a.flags & SPLIT_FLAG != 0 {
				RetailCb::Split { cb_a, cb_b: cursor.expect(LoaderId::CB_B)? }
			} else {
				RetailCb::Single(cb_a)
			};
			let cc = if LoaderId::CC.matches(cursor.peek()?.name) { Some(cursor.expect(LoaderId::CC)?) } else { None };
			let cd = cursor.expect(LoaderId::CD)?;
			let ce = cursor.expect(LoaderId::CE)?;
			BootChain::Retail { cb, cc, cd, ce }
		};

		Ok(Self { bytes, header, chain })
	}

	/// The boot region: the flash header through the declared boot image size.
	pub const fn bytes(&self) -> &'a [u8] {
		self.bytes
	}

	pub const fn header(&self) -> ImageHeader {
		self.header
	}

	/// Offset of the first boot stage, as the flash header declares it.
	pub const fn boot_chain_offset(&self) -> FileOffset {
		FileOffset(self.header.entry as usize)
	}

	pub const fn chain(&self) -> &BootChain {
		&self.chain
	}

	pub fn boot_stage_data(&self, stage: &BootStage) -> &'a [u8] {
		let start = stage.offset.get();
		&self.bytes[start..start + stage.size.get()]
	}

	/// The SMC firmware, or `None` when the flash header gives it no length. The location is
	/// checked on request, so a malformed one does not keep the rest of the image from opening.
	pub fn smc(&self) -> Result<Option<SmcImage>, ParseError> {
		parse_smc(self.bytes)
	}

	/// The encrypted bytes of the SMC firmware; [`crate::smc::decrypt`] gives the firmware.
	pub fn smc_data(&self, smc: &SmcImage) -> &'a [u8] {
		let start = smc.offset.get();
		&self.bytes[start..start + smc.size.get()]
	}

	/// The key vault range (`KeyVaultOffset` at 0x6c, `HV_KEY_VAULT`), or `None` when the flash
	/// header gives no offset. The range is not checked against the boot region.
	pub fn key_vault(&self) -> Result<Option<Range<usize>>, ParseError> {
		let offset = header_field(self.bytes, KEY_VAULT_OFFSET_FIELD)?;
		if offset == 0 {
			return Ok(None);
		}

		Ok(Some(offset..offset + KEY_VAULT_SIZE))
	}

	/// The kernel patch slots, or `None` when the flash header gives them no offset. Checked as
	/// `XeKeysSaveSystemUpdate` checks them, on request: both slots must lie within the
	/// 0x8000000-byte flash address space.
	pub fn patch_slots(&self) -> Result<Option<PatchSlotLocations>, ParseError> {
		const MAXIMUM_SLOT_SIZE: usize = 0x10_0000;
		const FLASH_SIZE: usize = 0x800_0000;

		let offset = header_field(self.bytes, PATCH_SLOT_OFFSET_FIELD)?;
		let size = match header_field(self.bytes, PATCH_SLOT_SIZE_FIELD)? {
			0 => DEFAULT_PATCH_SLOT_SIZE,
			size => size,
		};
		if offset == 0 {
			return Ok(None);
		}
		let valid = size.is_multiple_of(DEFAULT_PATCH_SLOT_SIZE)
			&& size <= MAXIMUM_SLOT_SIZE
			&& offset.is_multiple_of(STAGE_ALIGNMENT)
			&& (FLASH_HEADER_SIZE..FLASH_SIZE).contains(&offset)
			&& size * 2 < FLASH_SIZE - offset;
		if !valid {
			return Err(ParseError::InvalidPatchSlots { offset, size });
		}

		Ok(Some(PatchSlotLocations { offset: FileOffset::new(offset), size: ByteCount::new(size) }))
	}
}

impl FlashImage {
	/// Reads and parses the kernel patch slots that the flash header locates, outside the boot
	/// region. An image whose header names no patch slots has none.
	pub fn patch_slots<R>(&self, source: &R) -> Result<Checked<Vec<PatchSlot>>, SfcxError>
	where
		R: ReadAt,
	{
		let Some(locations) = self.boot_region().patch_slots()? else {
			return Ok(Checked::new(Vec::new(), Vec::new()));
		};

		let bytes = self.read(source, locations.ranges())?;
		parse_patch_slots(locations, bytes)
	}

	pub async fn patch_slots_async<R>(&self, source: &R) -> Result<Checked<Vec<PatchSlot>>, SfcxError>
	where
		R: AsyncReadAt,
	{
		let Some(locations) = self.boot_region().patch_slots()? else {
			return Ok(Checked::new(Vec::new(), Vec::new()));
		};

		let bytes = self.read_async(source, locations.ranges()).await?;
		parse_patch_slots(locations, bytes)
	}
}

fn parse_patch_slots(
	locations: PatchSlotLocations,
	bytes: Checked<Vec<u8>>,
) -> Result<Checked<Vec<PatchSlot>>, SfcxError> {
	let (bytes, warnings) = bytes.into_parts();
	let slots = bytes
		.chunks_exact(locations.size.get())
		.enumerate()
		.map(|(index, slot)| PatchSlot::parse(FileOffset::new(locations.range(index).start as usize), slot))
		.collect::<Result<Vec<_>, _>>()?;

	Ok(Checked::new(slots, warnings))
}

/// Walks the boot chain from the flash header's entry point.
struct ChainCursor<'a> {
	bytes: &'a [u8],
	offset: usize,
}

impl ChainCursor<'_> {
	fn peek(&self) -> Result<BootStage, ParseError> {
		parse_boot_stage(self.bytes, self.offset)
	}

	/// Takes the stage at the cursor, which must be the loader `expected`.
	fn expect(&mut self, expected: LoaderId) -> Result<BootStage, ParseError> {
		let stage = self.peek()?;
		if !expected.matches(stage.name) {
			return Err(ParseError::UnexpectedBootStage {
				offset: self.offset,
				expected: expected.name,
				actual: stage.name,
			});
		}
		self.offset = stage.aligned_end();

		Ok(stage)
	}
}

/// The SMC location fields of the flash header. A zero length means the image has no SMC
/// firmware; otherwise the firmware must lie after the flash header and within the boot region.
fn parse_smc(bytes: &[u8]) -> Result<Option<SmcImage>, ParseError> {
	let size = header_field(bytes, SMC_LENGTH_FIELD)?;
	let offset = header_field(bytes, SMC_OFFSET_FIELD)?;
	if size == 0 {
		return Ok(None);
	}
	let fits = offset >= FLASH_HEADER_SIZE && offset.checked_add(size).is_some_and(|end| end <= bytes.len());
	if !fits {
		return Err(ParseError::SmcOutOfBounds { offset, size });
	}
	Ok(Some(SmcImage { offset: FileOffset::new(offset), size: ByteCount::new(size) }))
}

/// A big-endian u32 location field of the flash header.
fn header_field(bytes: &[u8], at: usize) -> Result<usize, ParseError> {
	let word = bytes.get(at..at + 4).ok_or(ParseError::HeaderTruncated)?;
	Ok(u32::from_be_bytes([word[0], word[1], word[2], word[3]]) as usize)
}

fn parse_header_fields(input: &mut &[u8]) -> ModalResult<ImageHeader> {
	let magic_slice = take(2usize).parse_next(input)?;
	let magic = [magic_slice[0], magic_slice[1]];
	let build = BuildNumber(be_u16.parse_next(input)?);
	let qfe = be_u16.parse_next(input)?;
	let flags = be_u16.parse_next(input)?;
	let entry = be_u32.parse_next(input)?;
	let boot_image_size = ByteCount::new(be_u32.parse_next(input)? as usize);
	Ok(ImageHeader { magic, build, qfe, flags, entry, boot_image_size })
}

fn parse_boot_stage(bytes: &[u8], offset: usize) -> Result<BootStage, ParseError> {
	let mut input = offset
		.checked_add(IMAGE_HEADER_SIZE)
		.and_then(|end| bytes.get(offset..end))
		.ok_or(ParseError::BootStageHeaderTruncated { offset })?;
	let parsed = parse_header_fields(&mut input).map_err(|_| ParseError::BootStageHeaderTruncated { offset })?;
	let name = StageName(parsed.magic);
	let size = parsed.boot_image_size.get();
	if size < IMAGE_HEADER_SIZE {
		return Err(ParseError::InvalidBootStageSize { name, size });
	}
	offset.checked_add(size).filter(|end| *end <= bytes.len()).ok_or(ParseError::BootStageOutOfBounds { name })?;

	Ok(BootStage {
		name,
		build: parsed.build,
		qfe: parsed.qfe,
		flags: parsed.flags,
		entry: parsed.entry,
		offset: FileOffset::new(offset),
		size: parsed.boot_image_size,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn stage_names_display_as_quoted_ascii() {
		assert_eq!(StageName::SB.to_string(), "\"SB\"");
		assert_eq!(StageName::new([0x43, 0x00]).to_string(), "\"C\\x00\"");
		assert_eq!(StageName::new(*b"\"\xff").to_string(), "\"\\\"\\xff\"");
		assert_eq!(StageRole::CbB.to_string(), "\"CB_B\"");
		assert_eq!(StageRole::parse("cb_a"), Some(StageRole::CbA));
	}

	#[test]
	fn boot_stage_errors_name_stages_in_quotes() {
		let unexpected =
			ParseError::UnexpectedBootStage { offset: 0x8000, expected: StageName::SB, actual: StageName::CB };
		assert_eq!(unexpected.to_string(), "expected boot stage \"SB\" at 0x8000, got \"CB\"");
		let size = ParseError::InvalidBootStageSize { name: StageName::CD, size: 4 };
		assert_eq!(size.to_string(), "boot stage \"CD\" has an invalid size 0x4");
		let bounds = ParseError::BootStageOutOfBounds { name: StageName::new([0x01, b'E']) };
		assert_eq!(bounds.to_string(), "boot stage \"\\x01E\" extends past the boot region");
		let unknown = ParseError::UnknownBootChain { offset: 0x8000, name: StageName::new(*b"XB") };
		assert_eq!(
			unknown.to_string(),
			"boot stage \"XB\" at 0x8000 starts neither a devkit chain (\"SB\") nor a retail chain (\"CB\")"
		);
	}
}
