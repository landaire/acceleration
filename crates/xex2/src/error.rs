use rootcause::IntoReport;
use thiserror::Error;

use crate::delta::Span;
use crate::header::Sha1Hash;
use crate::opt::ResourceName;
use crate::writer::TargetMachine;

#[derive(Debug, Error)]
pub enum Xex2Error {
	#[error("invalid XEX magic: {}", hex4(.found))]
	InvalidMagic { found: [u8; 4] },
	#[error("header offset {offset:#x} exceeds file size {file_size:#x}")]
	InvalidHeaderOffset { offset: u32, file_size: usize },
	#[error("security offset {offset:#x} exceeds file size {file_size:#x}")]
	InvalidSecurityOffset { offset: u32, file_size: usize },
	#[error("missing required optional header {0:#010x}")]
	MissingOptionalHeader(u32),
	#[error("optional header {key:#010x} has invalid size {size}")]
	InvalidOptionalHeaderSize { key: u32, size: usize },
	#[error("invalid compression format: {0}")]
	InvalidCompressionFormat(u16),
	#[error("invalid encryption type: {0}")]
	InvalidEncryptionType(u16),
	#[error("LZX window size {size:#x} is not a power of two from 0x8000 to 0x200000")]
	InvalidWindowSize { size: u32 },
	#[error("compressed LZX chunk of {size:#x} bytes does not fit in a 64 KB block")]
	CompressedChunkTooLarge { size: usize },
	#[error("bounding path {path:?} must be non-empty printable ASCII")]
	InvalidBoundingPath { path: String },
	#[error("export table at {address:#010x} is not 4-byte aligned")]
	ExportTableUnaligned { address: u32 },
	#[error("export table at {address:#010x} extends outside the image")]
	ExportTableOutsideImage { address: u32 },
	#[error("export table at {address:#010x} occupies non-code page {page:#010x}")]
	ExportTableNotInCode { address: u32, page: u32 },
	#[error("resource name \"{}\" must be 1 to 8 bytes without NUL", .name.escape_ascii())]
	InvalidResourceName { name: Vec<u8> },
	#[error("resource name \"{name}\" appears more than once")]
	DuplicateResourceName { name: ResourceName },
	#[error("resource \"{name}\" at {address:#010x} with size {size:#x} extends outside the image")]
	ResourceOutsideImage { name: ResourceName, address: u32, size: u32 },
	#[error("basefile size mismatch: expected {expected:#x}, got {got:#x}")]
	InvalidBasefileSize { expected: usize, got: usize },
	#[error("page descriptor {descriptor_index} does not match its pages")]
	PageDescriptorMismatch { descriptor_index: usize },
	#[error("RSA signing failed")]
	SigningFailed,
	#[error("patch write at {offset:#x}+{len:#x} exceeds buffer size {buf_len:#x}")]
	PatchOutOfBounds { offset: u64, len: usize, buf_len: usize },
	#[error("patch has overlapping writes")]
	PatchOverlap,
	#[error(
		"requested rebuild transform is not implemented (Basic or Delta compression, or PE replacement on an encrypted source)"
	)]
	RebuildTransformNotImplemented,
	#[error("malformed basefile PE: {0}")]
	MalformedBasefilePe(BasefileDefect),
	#[error("FileFormatInfo does not describe its data format")]
	InvalidFileFormatInfo,
	#[error("image range {start:#x}..{end:#x} is not within the {image_size:#x}-byte image")]
	InvalidImageRange { start: u32, end: u32, image_size: u32 },
	#[error("data block {block_index} at {offset:#x} with size {size:#x} extends past the file")]
	DataBlockOutOfRange { block_index: usize, offset: usize, size: u32 },
	#[error("data block {block_index} has size {size:#x}; blocks are 0x800-byte multiples of at most 0x10000 bytes")]
	DataBlockSize { block_index: usize, size: u32 },
	#[error("data block {block_index} does not match its SHA-1")]
	DataBlockHashMismatch { block_index: usize },
	#[error("Basic run {run_index} has data size {size:#x}, which is not a multiple of 0x800")]
	BasicRunSize { run_index: usize, size: u32 },
	#[error("LZX frame at offset {offset:#x} of data block {block_index} extends past the block")]
	LzxFrameOutOfRange { block_index: usize, offset: usize },
	#[error("LZX decompression failed")]
	Lzx(#[source] xenon_lzx::Error),
	#[error("the {machine:?} master key for image flags {image_flags:#010x} is not known")]
	UnknownMasterKey { machine: TargetMachine, image_flags: u32 },
	#[error("delta patch data applies to a base XEX")]
	DeltaPatchNeedsBase,
	#[error("base XEX has patch module flags {module_flags:#010x}")]
	BaseHasPatchFlags { module_flags: u32 },
	#[error("patch XEX module flags {module_flags:#010x} lack the patch flag")]
	PatchFlagMissing { module_flags: u32 },
	#[error("delta patch descriptor of {len:#x} bytes is shorter than its 0x4c-byte fixed part")]
	DeltaDescriptorTruncated { len: usize },
	#[error("delta patch descriptor declares {declared:#x} bytes; {available:#x} are present")]
	DeltaDescriptorSize { declared: u32, available: usize },
	#[error(
		"delta header copy of {source_span:?} to {target_offset:#x} does not fit the {base_header_size:#x}-byte base and {target_header_size:#x}-byte patched headers"
	)]
	DeltaHeaderRange { source_span: Span, target_offset: u32, base_header_size: u32, target_header_size: u32 },
	#[error(
		"delta image copy of {source_span:?} to {target_offset:#x} does not fit the {image_size:#x}-byte patched image"
	)]
	DeltaImageRange { source_span: Span, target_offset: u32, image_size: u32 },
	#[error("patched headers of {requested:#x} bytes exceed the {limit:#x}-byte limit")]
	DeltaHeaderTooLarge { requested: u32, limit: u32 },
	#[error("could not allocate {size:#x} bytes")]
	AllocationFailed { size: usize },
	#[error("delta record at offset {offset:#x} is invalid")]
	InvalidDeltaRecord { offset: usize },
	#[error("base XEX signature does not match the patch's base signature digest")]
	BaseSignatureDigestMismatch,
	#[error("header size {size:#x} is not a 0x800-byte multiple from 0x800 to 0x10000")]
	HeaderSize { size: u32 },
	#[error("reserved header word is {value:#x}, not zero")]
	ReservedHeaderWord { value: u32 },
	#[error(
		"SecurityInfo of {security_size:#x} bytes at {security_offset:#x} does not fit the {header_size:#x}-byte headers"
	)]
	SecurityInfoOutOfBounds { security_offset: u32, security_size: u32, header_size: u32 },
	#[error("ImageInfo size {size:#x} is outside 0x104..=0x174")]
	ImageInfoSize { size: u32 },
	#[error("header signature does not verify with the retail or development executable key ({salt})")]
	SignatureInvalid { salt: &'static str },
	#[error("header hash {computed} does not match ImageInfo {expected}")]
	HeaderHashMismatch { expected: Sha1Hash, computed: Sha1Hash },
	#[error("image size {image_size:#x} is not a multiple of the {page_size:#x}-byte page")]
	ImageSizeAlignment { image_size: u32, page_size: u32 },
	#[error("load address {address:#010x} is not 64 KiB aligned")]
	LoadAddressAlignment { address: u32 },
	#[error("load address {address:#010x} is outside 0x80000000..0xa0000000")]
	LoadAddressRegion { address: u32 },
	#[error("load address {address:#010x} and the small page flag ({small_pages}) disagree")]
	LoadAddressPageSize { address: u32, small_pages: bool },
	#[error("image of {image_size:#x} bytes at {address:#010x} crosses its 256 MiB region")]
	ImageCrossesRegion { address: u32, image_size: u32 },
	#[error("I/O error")]
	Io(#[source] std::io::Error),
}

/// A structural defect in a basefile PE that prevents deriving its page
/// section types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BasefileDefect {
	#[error("truncated DOS header")]
	TruncatedDosHeader,
	#[error("truncated COFF header")]
	TruncatedCoffHeader,
	#[error("truncated section header")]
	TruncatedSectionHeader,
}

fn hex4(bytes: &[u8; 4]) -> String {
	bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub type Report = rootcause::Report<Xex2Error>;
pub type Result<T, E = Report> = std::result::Result<T, E>;

pub(crate) trait IoResultExt<T> {
	fn io(self) -> Result<T>;
}

impl<T> IoResultExt<T> for std::result::Result<T, std::io::Error> {
	fn io(self) -> Result<T> {
		self.map_err(|e| Xex2Error::Io(e).into_report())
	}
}
