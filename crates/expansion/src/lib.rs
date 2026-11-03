//! Xbox 360 hypervisor expansion packages (`HXPC`).
//!
//! The kernel passes a package to `HvxExpansionInstall` (build 16547
//! `0x30a98`), which checks the RSA signature over the header and the SHA-1
//! digest of the body, then `HvpExpansionLoad` (`0x30818`) copies the kernel
//! section into kernel memory and runs the hypervisor section. This crate
//! builds packages with `Flags` 0, signs them with the devkit executable key
//! and verifies them with a model of those two routines.
//!
//! All header fields are big-endian:
//!
//! | Offset | Field |
//! | --- | --- |
//! | `0x000` | `Magic` (`HXPC`) |
//! | `0x004` | `Flags` |
//! | `0x008` | `Size` |
//! | `0x00c` | `Digest`, SHA-1 of `[0x130, Size)` |
//! | `0x020` | `Iv` |
//! | `0x030` | `Signature` over the RotSumSha digest of `[0, 0x30)` |
//! | `0x130` | `Id` |
//! | `0x134` | `RequiredFlags` |
//! | `0x138` | `MinimumBuild` |
//! | `0x13c` | `MaximumBuild` |
//! | `0x140` | `Reserved[2]` |
//! | `0x148` | `RequiredMask` (u64) |
//! | `0x150` | `KernelSectionOffset`, `KernelSectionSize` |
//! | `0x158` | `HvSectionOffset`, `HvSectionSize` |

use std::fmt;
use std::fs::File;
use std::io;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::path::Path;
use std::path::PathBuf;

use thiserror::Error;
use xecrypt::ConsoleKind;
use xecrypt::RsaKeyKind;
use xecrypt::symmetric::xe_crypt_rot_sum_sha;
use xecrypt::symmetric::xe_crypt_sha;
use xecrypt::xex_sig;
use xecrypt::xex_sig::SALT_EXPANSION;

/// `HXPC`.
pub const MAGIC: u32 = 0x4858_5043;
/// `HXPR`, a package format this crate does not support.
pub const MAGIC_HXPR: u32 = 0x4858_5052;
/// Ids the kernel uses for its own packages.
pub const RESERVED_IDS: [u32; 2] = [0x454d_4120, 0x5056_3033];
/// Hypervisor build the verifier models.
pub const BUILD: u32 = 16547;
/// Size of the fixed header; sections start here.
pub const HEADER_SIZE: usize = 0x160;
/// Start of the region covered by `Digest`.
pub const BODY_OFFSET: usize = 0x130;
/// Install size limit of `ExExpansionInstall`.
pub const MAX_SIZE: usize = 0x10000;
/// Required alignment of the install size.
pub const SIZE_ALIGNMENT: usize = 0x80;
/// Kernel code area available to packages on build 16547 devkit.
pub const KERNEL_CODE_LIMIT: u32 = 0x3000;
/// Kernel data area available to packages.
pub const KERNEL_DATA_LIMIT: u32 = 0x1000;
/// Hypervisor area available to packages on build 16547 devkit.
pub const HV_LIMIT: u32 = 0x4000;
/// Size of a big-endian `XECRYPT_RSAPUB_2048`.
pub const PUBLIC_KEY_SIZE: usize = 0x110;
/// Offset of `HvpKeyBlock.PublicKey39`, the expansion key, in a build 16547
/// devkit hypervisor image.
pub const HV_PUBLIC_KEY_OFFSET: u64 = 0x105b8;

const DESCRIPTOR_SIZE: u32 = 0x10;

fn read_u32(data: &[u8], offset: usize) -> u32 {
	u32::from_be_bytes(data[offset..offset + 4].try_into().expect("4-byte field"))
}

fn write_u32(data: &mut [u8], offset: usize, value: u32) {
	data[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

/// Section that `HvpExpansionLoad` copies into kernel memory.
#[derive(Debug, Clone)]
pub struct KernelSection {
	/// Code copied to `0x90003000 + CodeUsed`.
	pub code: Vec<u8>,
	/// Bytes of zero-filled data copied to `0x90002000 + DataUsed`; rounded
	/// up to 0x10.
	pub data_size: u32,
}

/// Section that `HvpExpansionLoad` copies into hypervisor memory and calls.
#[derive(Debug, Clone)]
pub struct HvSection {
	pub code: Vec<u8>,
	/// Offset of the entry point in `code`; a multiple of 0x10.
	pub entry: u32,
}

/// Contents of a package to build.
#[derive(Debug, Clone, Default)]
pub struct PackageSpec {
	pub id: u32,
	/// 0 for no lower bound.
	pub minimum_build: u32,
	/// 0 for no upper bound; otherwise the first build that rejects the package.
	pub maximum_build: u32,
	pub kernel: Option<KernelSection>,
	pub hv: Option<HvSection>,
}

#[derive(Debug, Error)]
pub enum BuildError {
	#[error("package id {id:#010x} is zero or reserved for the kernel")]
	Id { id: u32 },
	#[error("a package needs a kernel section, a hypervisor section or both")]
	NoSections,
	#[error("the {section} section has no code")]
	EmptyCode { section: SectionKind },
	#[error("hypervisor entry offset {entry:#x} is not a multiple of 0x10")]
	HvEntryAlignment { entry: u32 },
	#[error("hypervisor entry offset {entry:#x} is outside the {code_size:#x}-byte code")]
	HvEntryRange { entry: u32, code_size: usize },
	#[error("kernel code ends at {code_end:#x}, beyond the {limit:#x}-byte kernel code area")]
	KernelCodeLimit { code_end: u64, limit: u32 },
	#[error("kernel data size {data_size:#x} exceeds the {limit:#x}-byte kernel data area")]
	KernelDataLimit { data_size: u32, limit: u32 },
	#[error("hypervisor section length {length:#x} exceeds the {limit:#x}-byte hypervisor area")]
	HvLimit { length: u64, limit: u32 },
	#[error("package size {size:#x} exceeds {limit:#x}", limit = MAX_SIZE)]
	PackageSize { size: usize },
	#[error(transparent)]
	Sign(#[from] SignError),
}

#[derive(Debug, Error)]
pub enum SignError {
	#[error("the package is {length:#x} bytes; the header needs {HEADER_SIZE:#x}")]
	Truncated { length: usize },
	#[error("header Size {size:#x} differs from the {length:#x}-byte package")]
	SizeMismatch { size: u32, length: usize },
	#[error("the devkit executable key is not available")]
	Key(#[from] xecrypt::Error),
}

/// Lay out a package and sign it with [`sign_devkit`].
pub fn build(spec: &PackageSpec) -> Result<Vec<u8>, BuildError> {
	if spec.id == 0 || RESERVED_IDS.contains(&spec.id) {
		return Err(BuildError::Id { id: spec.id });
	}
	if spec.kernel.is_none() && spec.hv.is_none() {
		return Err(BuildError::NoSections);
	}

	let mut package = vec![0u8; HEADER_SIZE];
	write_u32(&mut package, 0x000, MAGIC);
	write_u32(&mut package, 0x130, spec.id);
	write_u32(&mut package, 0x138, spec.minimum_build);
	write_u32(&mut package, 0x13c, spec.maximum_build);

	if let Some(kernel) = &spec.kernel {
		if kernel.code.is_empty() {
			return Err(BuildError::EmptyCode { section: SectionKind::Kernel });
		}
		let code_end = DESCRIPTOR_SIZE as u64 + (kernel.code.len() as u64).next_multiple_of(0x10);
		if code_end > KERNEL_CODE_LIMIT as u64 {
			return Err(BuildError::KernelCodeLimit { code_end, limit: KERNEL_CODE_LIMIT });
		}
		let code_end = code_end as u32;
		let data_size = kernel
			.data_size
			.checked_next_multiple_of(0x10)
			.filter(|size| *size <= KERNEL_DATA_LIMIT)
			.ok_or(BuildError::KernelDataLimit { data_size: kernel.data_size, limit: KERNEL_DATA_LIMIT })?;
		let offset = package.len();
		package.resize(offset + (code_end + data_size) as usize, 0);
		write_u32(&mut package, offset, DESCRIPTOR_SIZE);
		write_u32(&mut package, offset + 4, code_end);
		write_u32(&mut package, offset + 8, data_size);
		let code_offset = offset + DESCRIPTOR_SIZE as usize;
		package[code_offset..code_offset + kernel.code.len()].copy_from_slice(&kernel.code);
		write_u32(&mut package, 0x150, offset as u32);
		write_u32(&mut package, 0x154, code_end + data_size);
	}

	if let Some(hv) = &spec.hv {
		if hv.code.is_empty() {
			return Err(BuildError::EmptyCode { section: SectionKind::Hv });
		}
		if !hv.entry.is_multiple_of(0x10) {
			return Err(BuildError::HvEntryAlignment { entry: hv.entry });
		}
		if hv.entry as usize >= hv.code.len() {
			return Err(BuildError::HvEntryRange { entry: hv.entry, code_size: hv.code.len() });
		}
		let length = DESCRIPTOR_SIZE as u64 + (hv.code.len() as u64).next_multiple_of(0x10);
		if length > HV_LIMIT as u64 {
			return Err(BuildError::HvLimit { length, limit: HV_LIMIT });
		}
		let length = length as u32;
		let offset = package.len();
		package.resize(offset + length as usize, 0);
		write_u32(&mut package, offset, DESCRIPTOR_SIZE + hv.entry);
		write_u32(&mut package, offset + 4, length);
		write_u32(&mut package, offset + 8, 0);
		let code_offset = offset + DESCRIPTOR_SIZE as usize;
		package[code_offset..code_offset + hv.code.len()].copy_from_slice(&hv.code);
		write_u32(&mut package, 0x158, offset as u32);
		write_u32(&mut package, 0x15c, length);
	}

	let size = package.len().next_multiple_of(SIZE_ALIGNMENT);
	if size > MAX_SIZE {
		return Err(BuildError::PackageSize { size });
	}
	package.resize(size, 0);
	write_u32(&mut package, 0x008, size as u32);
	sign_devkit(&mut package)?;
	Ok(package)
}

/// Write `Digest` and `Signature` for a laid-out package whose `Size` equals
/// its length, using the devkit executable key.
pub fn sign_devkit(package: &mut [u8]) -> Result<(), SignError> {
	if package.len() < HEADER_SIZE {
		return Err(SignError::Truncated { length: package.len() });
	}
	let size = read_u32(package, 0x008);
	if size as usize != package.len() {
		return Err(SignError::SizeMismatch { size, length: package.len() });
	}
	let key = RsaKeyKind::Executable.private_key(ConsoleKind::Devkit)?;
	package[0x030..0x130].fill(0);
	let digest = xe_crypt_sha(&package[BODY_OFFSET..]);
	package[0x00c..0x020].copy_from_slice(&digest);
	let header_digest = xe_crypt_rot_sum_sha(&package[..0x030], &[]);
	let signature = xex_sig::sign(&header_digest, SALT_EXPANSION, &key);
	package[0x030..0x130].copy_from_slice(&signature);
	Ok(())
}

/// The devkit executable public key as a big-endian `XECRYPT_RSAPUB_2048`.
pub fn devkit_public_key() -> Result<[u8; PUBLIC_KEY_SIZE], xecrypt::Error> {
	let key = RsaKeyKind::Executable.public_key(ConsoleKind::Devkit)?;
	Ok(xex_sig::xecrypt_public_key_bytes(&key).expect("devkit executable key is 2048-bit with exponent 3"))
}

#[derive(Debug, Error)]
pub enum PublicKeyError {
	#[error("cannot read public key {}", path.display())]
	Io { path: PathBuf, source: io::Error },
	#[error("public key file {} is {length:#x} bytes; expected {PUBLIC_KEY_SIZE:#x} or an offset", path.display())]
	Length { path: PathBuf, length: u64 },
	#[error("public key offset {offset:#x} plus {PUBLIC_KEY_SIZE:#x} exceeds the {length:#x}-byte file {}", path.display())]
	OffsetRange { path: PathBuf, offset: u64, length: u64 },
}

/// Read a big-endian `XECRYPT_RSAPUB_2048` from a file of exactly
/// [`PUBLIC_KEY_SIZE`] bytes, or from `offset` in a larger file such as a
/// hypervisor image ([`HV_PUBLIC_KEY_OFFSET`]).
pub fn load_public_key(path: &Path, offset: Option<u64>) -> Result<[u8; PUBLIC_KEY_SIZE], PublicKeyError> {
	let io_error = |source| PublicKeyError::Io { path: path.to_owned(), source };
	let mut file = File::open(path).map_err(io_error)?;
	let length = file.metadata().map_err(io_error)?.len();
	let offset = match offset {
		Some(offset) => {
			if offset.checked_add(PUBLIC_KEY_SIZE as u64).is_none_or(|end| end > length) {
				return Err(PublicKeyError::OffsetRange { path: path.to_owned(), offset, length });
			}
			offset
		}
		None if length == PUBLIC_KEY_SIZE as u64 => 0,
		None => return Err(PublicKeyError::Length { path: path.to_owned(), length }),
	};
	let mut key = [0u8; PUBLIC_KEY_SIZE];
	file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
	file.read_exact(&mut key).map_err(io_error)?;
	Ok(key)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
	Kernel,
	Hv,
}

impl fmt::Display for SectionKind {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			SectionKind::Kernel => "kernel",
			SectionKind::Hv => "hypervisor",
		})
	}
}

/// A check of `HvxExpansionInstall` or `HvpExpansionLoad` that failed.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum VerifyError {
	#[error("install size {size:#x} is not a multiple of {SIZE_ALIGNMENT:#x}")]
	InstallSizeAlignment { size: usize },
	#[error("install size {size:#x} exceeds {MAX_SIZE:#x}")]
	InstallSizeLimit { size: usize },
	#[error("install size {size:#x} is smaller than the {HEADER_SIZE:#x}-byte header")]
	InstallSizeHeader { size: usize },
	#[error("HXPR packages are not supported")]
	Hxpr,
	#[error("magic {magic:#010x} is not HXPC")]
	Magic { magic: u32 },
	#[error("flags {flags:#x} require the CPU key; only flags 0 are verified")]
	Flags { flags: u32 },
	#[error("public key Cqw {cqw:#x} is not 0x20")]
	PublicKeyCqw { cqw: u32 },
	#[error("public key exponent {exponent:#x} is not 3 or 0x10001")]
	PublicKeyExponent { exponent: u32 },
	#[error("public key modulus is not a usable RSA modulus")]
	PublicKeyModulus,
	#[error("the header signature does not verify")]
	Signature,
	#[error("Size {size:#x} exceeds the install size {install_size:#x}")]
	SizeExceedsInstall { size: u32, install_size: usize },
	#[error("Size {size:#x} is smaller than the {HEADER_SIZE:#x}-byte header")]
	SizeHeader { size: u32 },
	#[error("body digest {actual} does not match Digest {expected}", expected = hex(expected), actual = hex(actual))]
	Digest { expected: [u8; 20], actual: [u8; 20] },
	#[error("package id is zero")]
	IdZero,
	#[error("package id {id:#010x} is reserved for the kernel")]
	IdReserved { id: u32 },
	#[error("RequiredFlags {flags:#x} is not 0")]
	RequiredFlags { flags: u32 },
	#[error("RequiredMask {mask:#x} is not 0")]
	RequiredMask { mask: u64 },
	#[error("MinimumBuild {minimum} is above build {BUILD}")]
	MinimumBuild { minimum: u32 },
	#[error("MaximumBuild {maximum} is not above build {BUILD}")]
	MaximumBuild { maximum: u32 },
	#[error("{section} section at {offset:#x} with size {size:#x} is outside the {package_size:#x}-byte package")]
	SectionRange { section: SectionKind, offset: u32, size: u32, package_size: u32 },
	#[error(
		"kernel CodeStart {code_start:#x}, CodeEnd {code_end:#x} or DataSize {data_size:#x} is not a multiple of 0x10"
	)]
	KernelAlignment { code_start: u32, code_end: u32, data_size: u32 },
	#[error("kernel CodeStart {code_start:#x} is not below CodeEnd {code_end:#x}")]
	KernelCodeOrder { code_start: u32, code_end: u32 },
	#[error("kernel CodeEnd {code_end:#x} plus DataSize {data_size:#x} exceeds the section size {section_size:#x}")]
	KernelSectionSize { code_end: u32, data_size: u32, section_size: u32 },
	#[error("kernel CodeEnd {code_end:#x} exceeds the {KERNEL_CODE_LIMIT:#x}-byte kernel code area")]
	KernelCodeLimit { code_end: u32 },
	#[error("kernel DataSize {data_size:#x} exceeds the {KERNEL_DATA_LIMIT:#x}-byte kernel data area")]
	KernelDataLimit { data_size: u32 },
	#[error("hypervisor EntryOffset {entry_offset:#x} or length {length:#x} is not a multiple of 0x10")]
	HvAlignment { entry_offset: u32, length: u64 },
	#[error("hypervisor EntryOffset {entry_offset:#x} is not below the length {length:#x}")]
	HvEntryRange { entry_offset: u32, length: u64 },
	#[error("hypervisor length {length:#x} exceeds the section size {section_size:#x}")]
	HvSectionSize { length: u64, section_size: u32 },
	#[error("hypervisor length {length:#x} exceeds the {HV_LIMIT:#x}-byte hypervisor area")]
	HvLimit { length: u64 },
}

fn hex(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Kernel section descriptor of a verified package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KernelReport {
	pub offset: u32,
	pub size: u32,
	pub code_start: u32,
	pub code_end: u32,
	pub data_size: u32,
}

/// Hypervisor section descriptor of a verified package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HvReport {
	pub offset: u32,
	pub size: u32,
	pub entry_offset: u32,
	pub size1: u32,
	pub size2: u32,
}

/// Fields of a package that passed every check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
	pub id: u32,
	pub size: u32,
	pub digest: [u8; 20],
	pub minimum_build: u32,
	pub maximum_build: u32,
	pub kernel: Option<KernelReport>,
	pub hv: Option<HvReport>,
}

impl fmt::Display for Report {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		writeln!(f, "id: {:#010x}", self.id)?;
		writeln!(f, "size: {:#x}", self.size)?;
		writeln!(f, "digest: {}", hex(&self.digest))?;
		writeln!(f, "minimum build: {}", self.minimum_build)?;
		writeln!(f, "maximum build: {}", self.maximum_build)?;
		match &self.kernel {
			Some(k) => writeln!(
				f,
				"kernel section: offset {:#x}, size {:#x}, code [{:#x}, {:#x}), data size {:#x}",
				k.offset, k.size, k.code_start, k.code_end, k.data_size
			)?,
			None => writeln!(f, "kernel section: absent")?,
		}
		match &self.hv {
			Some(h) => write!(
				f,
				"hypervisor section: offset {:#x}, size {:#x}, entry {:#x}, length {:#x} + {:#x}",
				h.offset, h.size, h.entry_offset, h.size1, h.size2
			),
			None => write!(f, "hypervisor section: absent"),
		}
	}
}

/// Model of `HvxExpansionInstall` and `HvpExpansionLoad` (build 16547
/// devkit, empty expansion areas) for a package of `package.len()` install
/// bytes and the big-endian `XECRYPT_RSAPUB_2048` `public_key`.
pub fn verify(package: &[u8], public_key: &[u8; PUBLIC_KEY_SIZE]) -> Result<Report, VerifyError> {
	let install_size = package.len();
	if !install_size.is_multiple_of(SIZE_ALIGNMENT) {
		return Err(VerifyError::InstallSizeAlignment { size: install_size });
	}
	if install_size > MAX_SIZE {
		return Err(VerifyError::InstallSizeLimit { size: install_size });
	}
	if install_size < HEADER_SIZE {
		return Err(VerifyError::InstallSizeHeader { size: install_size });
	}
	match read_u32(package, 0x000) {
		MAGIC => {}
		MAGIC_HXPR => return Err(VerifyError::Hxpr),
		magic => return Err(VerifyError::Magic { magic }),
	}
	let flags = read_u32(package, 0x004);
	if flags != 0 {
		return Err(VerifyError::Flags { flags });
	}

	let cqw = read_u32(public_key, 0x000);
	if cqw != 0x20 {
		return Err(VerifyError::PublicKeyCqw { cqw });
	}
	let exponent = read_u32(public_key, 0x004);
	if !matches!(exponent, 3 | 0x10001) {
		return Err(VerifyError::PublicKeyExponent { exponent });
	}
	let key = xex_sig::xecrypt_public_key(public_key).ok_or(VerifyError::PublicKeyModulus)?;
	let header_digest = xe_crypt_rot_sum_sha(&package[..0x030], &[]);
	let signature: &[u8; 256] = package[0x030..0x130].try_into().expect("signature field");
	if !xex_sig::console_verify(signature, &header_digest, SALT_EXPANSION, &key) {
		return Err(VerifyError::Signature);
	}

	let size = read_u32(package, 0x008);
	if size as usize > install_size {
		return Err(VerifyError::SizeExceedsInstall { size, install_size });
	}
	if (size as usize) < HEADER_SIZE {
		return Err(VerifyError::SizeHeader { size });
	}
	let package = &package[..size as usize];
	let expected: [u8; 20] = package[0x00c..0x020].try_into().expect("digest field");
	let actual = xe_crypt_sha(&package[BODY_OFFSET..]);
	if actual != expected {
		return Err(VerifyError::Digest { expected, actual });
	}

	let id = read_u32(package, 0x130);
	if id == 0 {
		return Err(VerifyError::IdZero);
	}
	if RESERVED_IDS.contains(&id) {
		return Err(VerifyError::IdReserved { id });
	}
	let required_flags = read_u32(package, 0x134);
	if required_flags != 0 {
		return Err(VerifyError::RequiredFlags { flags: required_flags });
	}
	let minimum_build = read_u32(package, 0x138);
	if minimum_build != 0 && BUILD < minimum_build {
		return Err(VerifyError::MinimumBuild { minimum: minimum_build });
	}
	let maximum_build = read_u32(package, 0x13c);
	if maximum_build != 0 && BUILD >= maximum_build {
		return Err(VerifyError::MaximumBuild { maximum: maximum_build });
	}
	let required_mask = u64::from_be_bytes(package[0x148..0x150].try_into().expect("mask field"));
	if required_mask != 0 {
		return Err(VerifyError::RequiredMask { mask: required_mask });
	}

	// (offset, size, bytes) of a present section.
	type Section<'a> = Option<(u32, u32, &'a [u8])>;
	let section = |kind, field: usize| -> Result<Section<'_>, VerifyError> {
		let offset = read_u32(package, field);
		let section_size = read_u32(package, field + 4);
		if section_size == 0 {
			return Ok(None);
		}
		let end = offset as u64 + section_size as u64;
		if section_size < 0xc || end > size as u64 {
			return Err(VerifyError::SectionRange { section: kind, offset, size: section_size, package_size: size });
		}
		Ok(Some((offset, section_size, &package[offset as usize..end as usize])))
	};

	let kernel = match section(SectionKind::Kernel, 0x150)? {
		None => None,
		Some((offset, section_size, data)) => {
			let code_start = read_u32(data, 0);
			let code_end = read_u32(data, 4);
			let data_size = read_u32(data, 8);
			if !(code_start | code_end | data_size).is_multiple_of(0x10) {
				return Err(VerifyError::KernelAlignment { code_start, code_end, data_size });
			}
			if code_start >= code_end {
				return Err(VerifyError::KernelCodeOrder { code_start, code_end });
			}
			if code_end as u64 + data_size as u64 > section_size as u64 {
				return Err(VerifyError::KernelSectionSize { code_end, data_size, section_size });
			}
			if code_end > KERNEL_CODE_LIMIT {
				return Err(VerifyError::KernelCodeLimit { code_end });
			}
			if data_size > KERNEL_DATA_LIMIT {
				return Err(VerifyError::KernelDataLimit { data_size });
			}
			Some(KernelReport { offset, size: section_size, code_start, code_end, data_size })
		}
	};

	let hv = match section(SectionKind::Hv, 0x158)? {
		None => None,
		Some((offset, section_size, data)) => {
			let entry_offset = read_u32(data, 0);
			let size1 = read_u32(data, 4);
			let size2 = read_u32(data, 8);
			let length = size1 as u64 + size2 as u64;
			if !entry_offset.is_multiple_of(0x10) || !length.is_multiple_of(0x10) {
				return Err(VerifyError::HvAlignment { entry_offset, length });
			}
			if entry_offset as u64 >= length {
				return Err(VerifyError::HvEntryRange { entry_offset, length });
			}
			if length > section_size as u64 {
				return Err(VerifyError::HvSectionSize { length, section_size });
			}
			if length > HV_LIMIT as u64 {
				return Err(VerifyError::HvLimit { length });
			}
			Some(HvReport { offset, size: section_size, entry_offset, size1, size2 })
		}
	};

	Ok(Report { id, size, digest: expected, minimum_build, maximum_build, kernel, hv })
}

#[cfg(test)]
mod tests {
	use super::*;

	fn spec() -> PackageSpec {
		PackageSpec {
			id: 0x4f58_4258,
			minimum_build: 0,
			maximum_build: 0,
			kernel: Some(KernelSection { code: (0..0x24u8).collect(), data_size: 0x18 }),
			hv: Some(HvSection { code: (0x80..0xa0u8).collect(), entry: 0x10 }),
		}
	}

	fn key() -> [u8; PUBLIC_KEY_SIZE] {
		devkit_public_key().unwrap()
	}

	fn resign(package: &mut [u8], offset: usize, value: u32) {
		write_u32(package, offset, value);
		sign_devkit(package).unwrap();
	}

	#[test]
	fn built_package_verifies() {
		let package = build(&spec()).unwrap();
		let report = verify(&package, &key()).unwrap();
		assert_eq!(package.len(), 0x200);
		let kernel = report.kernel.unwrap();
		assert_eq!(&package[0x170..0x194], &spec().kernel.unwrap().code[..]);
		assert_eq!((kernel.code_start, kernel.code_end, kernel.data_size), (0x10, 0x40, 0x20));
		let hv = report.hv.unwrap();
		assert_eq!(hv.offset, 0x1c0);
		assert_eq!(&package[0x1d0..0x1f0], &spec().hv.unwrap().code[..]);
		assert_eq!((hv.entry_offset, hv.size1, hv.size2), (0x20, 0x30, 0));
	}

	#[test]
	fn header_fields_snapshot() {
		let spec = PackageSpec {
			id: 0x1234_5678,
			minimum_build: 16547,
			maximum_build: 17000,
			kernel: Some(KernelSection { code: vec![0x60, 0, 0, 0], data_size: 0x10 }),
			hv: Some(HvSection { code: vec![0x4e, 0x80, 0, 0x20], entry: 0 }),
		};
		let package = build(&spec).unwrap();
		insta::assert_snapshot!(verify(&package, &key()).unwrap().to_string());
	}

	#[test]
	fn header_byte_flip_fails_signature() {
		let mut package = build(&spec()).unwrap();
		package[0x10] ^= 1;
		assert_eq!(verify(&package, &key()), Err(VerifyError::Signature));
	}

	#[test]
	fn signature_byte_flip_fails_signature() {
		let mut package = build(&spec()).unwrap();
		package[0x80] ^= 1;
		assert_eq!(verify(&package, &key()), Err(VerifyError::Signature));
	}

	#[test]
	fn body_byte_flip_fails_digest() {
		let mut package = build(&spec()).unwrap();
		package[0x180] ^= 1;
		assert!(matches!(verify(&package, &key()), Err(VerifyError::Digest { .. })));
	}

	#[test]
	fn wrong_key_fails_signature() {
		let live = RsaKeyKind::Live.public_key(ConsoleKind::Devkit).unwrap();
		let live = xex_sig::xecrypt_public_key_bytes(&live).unwrap();
		let package = build(&spec()).unwrap();
		assert_eq!(verify(&package, &live), Err(VerifyError::Signature));
	}

	#[test]
	fn malformed_public_key_is_rejected() {
		let package = build(&spec()).unwrap();
		let mut key = key();
		key[3] = 0x1f;
		assert_eq!(verify(&package, &key), Err(VerifyError::PublicKeyCqw { cqw: 0x1f }));
		key[3] = 0x20;
		key[7] = 5;
		assert_eq!(verify(&package, &key), Err(VerifyError::PublicKeyExponent { exponent: 5 }));
	}

	#[test]
	fn zero_and_reserved_ids_are_rejected() {
		let mut package = build(&spec()).unwrap();
		resign(&mut package, 0x130, 0);
		assert_eq!(verify(&package, &key()), Err(VerifyError::IdZero));
		resign(&mut package, 0x130, 0x454d_4120);
		assert_eq!(verify(&package, &key()), Err(VerifyError::IdReserved { id: 0x454d_4120 }));
		assert!(matches!(build(&PackageSpec { id: 0, ..spec() }), Err(BuildError::Id { id: 0 })));
	}

	#[test]
	fn misaligned_sections_are_rejected() {
		let mut package = build(&spec()).unwrap();
		resign(&mut package, 0x164, 0x38);
		assert_eq!(
			verify(&package, &key()),
			Err(VerifyError::KernelAlignment { code_start: 0x10, code_end: 0x38, data_size: 0x20 })
		);

		let mut package = build(&spec()).unwrap();
		resign(&mut package, 0x1c0, 0x18);
		assert_eq!(verify(&package, &key()), Err(VerifyError::HvAlignment { entry_offset: 0x18, length: 0x30 }));
	}

	#[test]
	fn install_size_must_be_a_multiple_of_0x80() {
		let mut package = build(&spec()).unwrap();
		package.extend_from_slice(&[0; 0x10]);
		assert_eq!(verify(&package, &key()), Err(VerifyError::InstallSizeAlignment { size: 0x210 }));
	}

	#[test]
	fn build_range_excludes_build_16547() {
		let package = build(&PackageSpec { maximum_build: BUILD, ..spec() }).unwrap();
		assert_eq!(verify(&package, &key()), Err(VerifyError::MaximumBuild { maximum: BUILD }));
		let package = build(&PackageSpec { minimum_build: BUILD + 1, ..spec() }).unwrap();
		assert_eq!(verify(&package, &key()), Err(VerifyError::MinimumBuild { minimum: BUILD + 1 }));
	}

	#[test]
	fn hxpr_and_flags_are_reported() {
		let mut package = build(&spec()).unwrap();
		write_u32(&mut package, 0, MAGIC_HXPR);
		assert_eq!(verify(&package, &key()), Err(VerifyError::Hxpr));
		write_u32(&mut package, 0, MAGIC);
		write_u32(&mut package, 4, 2);
		assert_eq!(verify(&package, &key()), Err(VerifyError::Flags { flags: 2 }));
	}
}
