use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::BTreeMap;

use rootcause::IntoReport;
use rootcause::Report;
use xex2::Xex2;
use xex2::delta::FILE_HEADER_LIMIT;
use xex2::delta::PatchedModule;
use xex2::opt::ResourceName;

use crate::error::SystemUpdateError;
use crate::package::UpdatePackage;

/// Prefix of the package files that the update installs on the system flash
/// volume. xam strips its seven characters from each name (6717 xam
/// `0x81906f28`).
const FLASH_PREFIX: &str = "$flash_";
/// Suffix of a delta patch. The patch for `<name>.xex` is `<name>.xexp`:
/// every patch of the 6717 update carries the signature digest of the 1888
/// module of that name. The kernel checks the digest when it applies a patch.
const PATCH_SUFFIX: &str = ".xexp";

/// Name of a file on the system flash volume. Lookups ignore ASCII case: the
/// dashboard opens `XenonCLatin.xtt` while the 1888 flash image stores
/// `xenonclatin.xtt`.
#[derive(Debug, Clone)]
pub struct FlashName(String);

impl FlashName {
	pub fn new(name: impl Into<String>) -> Self {
		Self(name.into())
	}

	pub fn as_str(&self) -> &str {
		&self.0
	}
}

impl PartialEq for FlashName {
	fn eq(&self, other: &Self) -> bool {
		self.0.eq_ignore_ascii_case(&other.0)
	}
}

impl Eq for FlashName {}

impl PartialOrd for FlashName {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl Ord for FlashName {
	fn cmp(&self, other: &Self) -> Ordering {
		let lower = |name: &str| name.bytes().map(|byte| byte.to_ascii_lowercase()).collect::<Vec<u8>>();
		lower(&self.0).cmp(&lower(&other.0))
	}
}

/// Contents of one flash file.
#[derive(Debug)]
pub enum FlashFile<'a> {
	/// A file supplied by the caller: a base file the update does not
	/// replace, or an already patched file.
	Supplied(&'a [u8]),
	/// A file copied from the update package.
	Update(Vec<u8>),
	/// A module rebuilt from its base XEX and a delta patch. The console
	/// builds it in memory when it loads the module; no XEX file exists.
	Patched(PatchedModule),
}

impl FlashFile<'_> {
	/// The file's bytes, or `None` for a patched module.
	pub fn bytes(&self) -> Option<&[u8]> {
		match self {
			FlashFile::Supplied(bytes) => Some(bytes),
			FlashFile::Update(bytes) => Some(bytes),
			FlashFile::Patched(_) => None,
		}
	}
}

/// The result of [`FlashFiles::apply_update`].
#[derive(Debug, Default)]
pub struct AppliedUpdate {
	/// Delta patches whose base module is not in the file set. Each stays in
	/// the set under its own name.
	pub patches_without_base: Vec<FlashName>,
}

/// The files of a system flash volume, by name.
#[derive(Debug, Default)]
pub struct FlashFiles<'a> {
	files: BTreeMap<FlashName, FlashFile<'a>>,
}

impl<'a> FlashFiles<'a> {
	pub fn new() -> Self {
		Self { files: BTreeMap::new() }
	}

	/// Add or replace a file.
	pub fn insert(&mut self, name: impl Into<String>, file: FlashFile<'a>) {
		let name = FlashName::new(name);
		self.files.remove(&name);
		self.files.insert(name, file);
	}

	pub fn get(&self, name: &str) -> Option<&FlashFile<'a>> {
		self.files.get(&FlashName::new(name))
	}

	pub fn iter(&self) -> impl Iterator<Item = (&FlashName, &FlashFile<'a>)> {
		self.files.iter()
	}

	/// Install a system update: each `$flash_` file of the package replaces
	/// or adds the file named without the prefix, and each delta patch
	/// (`.xexp`) is applied to its base module, which becomes a patched
	/// module. Patches apply after the full files.
	pub fn apply_update(&mut self, package: &UpdatePackage<'_>) -> Result<AppliedUpdate, Report<SystemUpdateError>> {
		let mut patches = Vec::new();
		for file in package.files() {
			let Some(name) = file.name().strip_prefix(FLASH_PREFIX) else {
				continue;
			};

			let contents = package.read(&file)?;
			if has_patch_suffix(name) {
				patches.push((name.to_string(), contents));
			} else {
				self.insert(name, FlashFile::Update(contents));
			}
		}

		let mut applied = AppliedUpdate::default();
		for (patch_name, patch) in patches {
			let base_name = &patch_name[..patch_name.len() - 1];
			let Some(base) = self.get(base_name) else {
				applied.patches_without_base.push(FlashName::new(patch_name.as_str()));
				self.insert(patch_name, FlashFile::Update(patch));
				continue;
			};

			let Some(base) = base.bytes() else {
				return Err(SystemUpdateError::PatchedBase { name: base_name.to_string() }.into_report());
			};

			let module = xex2::delta::apply_delta_patch(base, &patch, FILE_HEADER_LIMIT).map_err(|report| {
				report.context(SystemUpdateError::DeltaPatch { patch: patch_name.clone(), base: base_name.to_string() })
			})?;
			self.insert(base_name, FlashFile::Patched(module));
		}

		Ok(applied)
	}

	/// The headers and loaded image of a module, with the console's checks:
	/// a XEX file is verified and loaded, a patched module was verified when
	/// it was built.
	pub fn module(&self, name: &str) -> Result<Module<'_>, Report<SystemUpdateError>> {
		let module_error = |report: Report<xex2::error::Xex2Error>| {
			report.context(SystemUpdateError::Module { name: name.to_string() })
		};
		let Some(file) = self.get(name) else {
			return Err(SystemUpdateError::MissingFile { name: name.to_string() }.into_report());
		};

		let bytes = match file {
			FlashFile::Patched(module) => {
				let xex = module.xex().map_err(module_error)?;
				return Ok(Module { name: name.to_string(), xex, image: Cow::Borrowed(&module.image) });
			}
			FlashFile::Supplied(bytes) => *bytes,
			FlashFile::Update(bytes) => bytes.as_slice(),
		};

		let xex = Xex2::parse(bytes).map_err(module_error)?;
		let image = xex.extract_basefile(bytes).map_err(module_error)?;

		Ok(Module { name: name.to_string(), xex, image: Cow::Owned(image) })
	}
}

fn has_patch_suffix(name: &str) -> bool {
	name.len() > PATCH_SUFFIX.len() && name[name.len() - PATCH_SUFFIX.len()..].eq_ignore_ascii_case(PATCH_SUFFIX)
}

/// A module's parsed headers and loaded image.
pub struct Module<'s> {
	name: String,
	xex: Xex2,
	image: Cow<'s, [u8]>,
}

impl Module<'_> {
	pub fn xex(&self) -> &Xex2 {
		&self.xex
	}

	/// The loaded image (basefile).
	pub fn image(&self) -> &[u8] {
		&self.image
	}

	/// The resources named in the ResourceInfo header, in header order, with
	/// their bytes in the image.
	pub fn resources(&self) -> Result<Vec<(ResourceName, &[u8])>, Report<SystemUpdateError>> {
		let Some(info) = self.xex.header.resource_info() else {
			return Ok(Vec::new());
		};

		info.resources.iter().map(|entry| Ok((entry.name, self.resource_bytes(entry)?))).collect()
	}

	/// The first resource whose name equals `name` byte for byte, as
	/// `XexGetModuleSection` finds it (kernel 21256 `0x800a4290`).
	pub fn resource(&self, name: &str) -> Result<Option<&[u8]>, Report<SystemUpdateError>> {
		let Some(info) = self.xex.header.resource_info() else {
			return Ok(None);
		};

		match info.resources.iter().find(|entry| entry.name.as_bytes() == name.as_bytes()) {
			Some(entry) => Ok(Some(self.resource_bytes(entry)?)),
			None => Ok(None),
		}
	}

	fn resource_bytes(&self, entry: &xex2::opt::ResourceEntry) -> Result<&[u8], Report<SystemUpdateError>> {
		let load_address = self.xex.security_info.image_info.load_address.0;
		let range = entry
			.address
			.0
			.checked_sub(load_address)
			.map(|offset| offset as usize..offset as usize + entry.size as usize)
			.filter(|range| range.end <= self.image.len());
		let Some(range) = range else {
			return Err(SystemUpdateError::ResourceOutsideImage {
				module: self.name.clone(),
				resource: entry.name.to_string(),
				address: entry.address.0,
				size: entry.size,
			}
			.into_report());
		};

		Ok(&self.image[range])
	}
}

#[cfg(test)]
mod tests {
	use xex2::builder::Xex2Builder;
	use xex2::header::VirtualAddress;
	use xex2::opt::ResourceEntry;

	use super::*;

	const LOAD_ADDRESS: u32 = 0x8200_0000;

	fn module_with_resource() -> Vec<u8> {
		let mut image = vec![0u8; 0x1_0000];
		image[..2].copy_from_slice(b"MZ");
		image[0x2000..0x2010].copy_from_slice(b"shared resources");

		let resource = ResourceEntry {
			name: ResourceName::new("shrdres").unwrap(),
			address: VirtualAddress(LOAD_ADDRESS + 0x2000),
			size: 0x10,
		};
		Xex2Builder::new(image)
			.entry_point(VirtualAddress(LOAD_ADDRESS + 0x1000))
			.load_address(VirtualAddress(LOAD_ADDRESS))
			.resources(vec![resource])
			.build()
			.unwrap()
	}

	#[test]
	fn lookups_ignore_ascii_case() {
		let mut files = FlashFiles::new();
		files.insert("xenonclatin.xtt", FlashFile::Supplied(b"font"));

		assert_eq!(files.get("XenonCLatin.xtt").and_then(FlashFile::bytes), Some(&b"font"[..]));

		files.insert("XENONCLATIN.XTT", FlashFile::Update(b"new font".to_vec()));
		assert_eq!(files.iter().count(), 1);
		assert_eq!(files.get("xenonclatin.xtt").and_then(FlashFile::bytes), Some(&b"new font"[..]));
	}

	#[test]
	fn module_resources_are_read_from_the_loaded_image() {
		let xex = module_with_resource();
		let mut files = FlashFiles::new();
		files.insert("xam.xex", FlashFile::Supplied(&xex));

		let module = files.module("XAM.XEX").unwrap();

		assert_eq!(module.resource("shrdres").unwrap(), Some(&b"shared resources"[..]));
		assert_eq!(module.resource("SHRDRES").unwrap(), None);
		let names: Vec<String> = module.resources().unwrap().iter().map(|(name, _)| name.to_string()).collect();
		assert_eq!(names, ["shrdres"]);
	}

	#[test]
	fn patch_suffix_ignores_case_and_needs_a_stem() {
		assert!(has_patch_suffix("xam.XEXP"));
		assert!(!has_patch_suffix(".xexp"));
		assert!(!has_patch_suffix("XenonCLatin.xttp"));
	}
}
