use serde::Serialize;
use stfs::BytesStfsWrapper;

fn open_fixture(name: &str) -> BytesStfsWrapper<Vec<u8>> {
	let path = format!("tests/fixtures/{}", name);
	let data = std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {}", path, e));
	BytesStfsWrapper::open(data).unwrap_or_else(|e| panic!("failed to parse {}: {}", name, e))
}

#[derive(Serialize)]
struct WalkEntry {
	path: String,
	size: usize,
}

#[derive(Serialize)]
struct WalkResult {
	files: Vec<WalkEntry>,
}

fn walk_snapshot(wrapper: &BytesStfsWrapper<Vec<u8>>) -> WalkResult {
	let files = wrapper
		.package()
		.file_table
		.walk_files()
		.into_iter()
		.map(|(path, entry)| WalkEntry { path, size: entry.file_size })
		.collect();
	WalkResult { files }
}

macro_rules! fixture_tests {
	($name:ident, $file:literal) => {
		mod $name {
			use super::*;

			#[test]
			fn header() {
				let wrapper = open_fixture($file);
				insta::assert_toml_snapshot!(wrapper.package().header);
			}

			#[test]
			fn file_table() {
				let wrapper = open_fixture($file);
				insta::assert_toml_snapshot!(wrapper.package().file_table);
			}

			#[test]
			fn walk_files() {
				let wrapper = open_fixture($file);
				insta::assert_toml_snapshot!(walk_snapshot(&wrapper));
			}

			#[test]
			fn extract_all() {
				let wrapper = open_fixture($file);
				for (path, entry) in wrapper.package().file_table.walk_files() {
					let mut buf = Vec::new();
					wrapper
						.extract_file(&mut buf, &entry)
						.unwrap_or_else(|e| panic!("failed to extract {}: {}", path, e));
					assert_eq!(buf.len(), entry.file_size, "extracted size mismatch for {}", path);
				}
			}
		}
	};
}

fixture_tests!(live_532k, "live_532k.stfs");
fixture_tests!(live_256k, "live_256k.stfs");
fixture_tests!(live_120k_a, "live_120k_a.stfs");
fixture_tests!(live_120k_b, "live_120k_b.stfs");

#[cfg(feature = "vfs")]
mod vfs_tests {
	use stfs::io::SliceReader;
	use stfs::stfs_vfs::StfsVfs;
	use stfs::StfsPackage;
	use vfs::FileSystem;

	fn open_vfs_fixture(name: &str) -> impl FileSystem {
		let path = format!("tests/fixtures/{}", name);
		let data = std::fs::read(&path).unwrap();
		let reader = SliceReader(&data);
		let package = StfsPackage::open(&reader).unwrap();
		// We need owned data for the VFS, re-open with owned slice
		let data = std::fs::read(&path).unwrap();
		StfsVfs::new(data, package)
	}

	#[test]
	fn vfs_read_dir_root() {
		let vfs = open_vfs_fixture("live_532k.stfs");
		let mut entries: Vec<String> = vfs.read_dir("").unwrap().collect();
		entries.sort();
		assert!(!entries.is_empty());
		// Should contain top-level files and directories
		assert!(entries.contains(&"pack.pak".to_string()));
		assert!(entries.contains(&"marketplace.png".to_string()));
	}

	#[test]
	fn vfs_open_and_read_file() {
		use std::io::Read;
		let vfs = open_vfs_fixture("live_532k.stfs");
		let mut file = vfs.open_file("pack2.xus").unwrap();
		let mut buf = Vec::new();
		file.read_to_end(&mut buf).unwrap();
		assert_eq!(buf.len(), 712);
	}

	#[test]
	fn vfs_metadata() {
		let vfs = open_vfs_fixture("live_532k.stfs");
		let meta = vfs.metadata("pack.pak").unwrap();
		assert_eq!(meta.len, 369777);
		assert_eq!(meta.file_type, vfs::VfsFileType::File);
	}

	#[test]
	fn vfs_exists() {
		let vfs = open_vfs_fixture("live_532k.stfs");
		assert!(vfs.exists("pack.pak").unwrap());
		assert!(vfs.exists("de-de").unwrap());
		assert!(vfs.exists("de-de/pack2.xus").unwrap());
		assert!(!vfs.exists("nonexistent").unwrap());
	}

	#[test]
	fn vfs_subdirectory() {
		let vfs = open_vfs_fixture("live_532k.stfs");
		let entries: Vec<String> = vfs.read_dir("de-de").unwrap().collect();
		assert_eq!(entries, vec!["pack2.xus"]);
	}
}
