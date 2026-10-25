use std::fs::File;

use memmap2::Mmap;
use system_update::FlashFile;
use system_update::FlashFiles;
use system_update::UpdatePackage;

fn map(path: &std::path::Path) -> Mmap {
	let file = File::open(path).unwrap();
	// SAFETY: the test only reads the mapping.
	unsafe { Mmap::map(&file) }.unwrap()
}

/// Installs a console system update over its base flash files:
/// `SYSTEM_UPDATE_PACKAGE` names the package, `SYSTEM_UPDATE_BASE` a directory
/// of base flash files, and `SYSTEM_UPDATE_XAM_BASEFILE` the expected image
/// of the updated `xam.xex`.
#[test]
#[ignore = "requires SYSTEM_UPDATE_PACKAGE, SYSTEM_UPDATE_BASE and SYSTEM_UPDATE_XAM_BASEFILE"]
fn installs_a_system_update_over_base_files() {
	let path = |name: &str| std::path::PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is set")));
	let package = map(&path("SYSTEM_UPDATE_PACKAGE"));
	let expected_xam = std::fs::read(path("SYSTEM_UPDATE_XAM_BASEFILE")).unwrap();

	let base: Vec<(String, Mmap)> = std::fs::read_dir(path("SYSTEM_UPDATE_BASE"))
		.unwrap()
		.map(|entry| entry.unwrap())
		.filter(|entry| entry.file_type().unwrap().is_file())
		.map(|entry| (entry.file_name().to_string_lossy().into_owned(), map(&entry.path())))
		.collect();
	let mut files = FlashFiles::new();
	for (name, map) in &base {
		files.insert(name.as_str(), FlashFile::Supplied(map));
	}

	files.apply_update(&UpdatePackage::parse(&package).unwrap()).unwrap();

	let xam = files.module("xam.xex").unwrap();
	assert!(xam.image() == expected_xam.as_slice());
	assert!(xam.resource("shrdres").unwrap().is_some());
}
