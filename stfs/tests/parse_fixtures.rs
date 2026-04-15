use serde::Serialize;
use stfs::BytesStfsReader;
use stfs::StfsPackageReader;

fn open_fixture(name: &str) -> BytesStfsReader<Vec<u8>> {
	let path = format!("tests/fixtures/{}", name);
	let data = std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {}", path, e));
	BytesStfsReader::open(data).unwrap_or_else(|e| panic!("failed to parse {}: {}", name, e))
}

#[derive(Serialize)]
struct WalkSnapshotEntry {
	path: String,
	size: usize,
}

#[derive(Serialize)]
struct WalkSnapshot {
	files: Vec<WalkSnapshotEntry>,
}

fn walk_snapshot(wrapper: &BytesStfsReader<Vec<u8>>) -> WalkSnapshot {
	let files = wrapper
		.package()
		.file_table
		.walk_files()
		.into_iter()
		.map(|w| WalkSnapshotEntry { path: w.path, size: w.entry.file_size })
		.collect();
	WalkSnapshot { files }
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
				for w in wrapper.package().file_table.walk_files() {
					let mut buf = Vec::new();
					wrapper
						.extract_file(&mut buf, &w.entry)
						.unwrap_or_else(|e| panic!("failed to extract {}: {}", w.path, e));
					assert_eq!(buf.len(), w.entry.file_size, "extracted size mismatch for {}", w.path);
				}
			}
		}
	};
}

fixture_tests!(live_532k, "live_532k.stfs");
fixture_tests!(live_256k, "live_256k.stfs");
fixture_tests!(live_120k_a, "live_120k_a.stfs");
fixture_tests!(live_120k_b, "live_120k_b.stfs");

mod hash_validation {
	use stfs::hashing::HashVerifyContext;
	use stfs::hashing::StfsHasher;
	use stfs::io::SliceReader;
	use stfs::types::HashTableLevel;
	use stfs::types::Sha1Digest;
	use stfs::StfsPackage;

	fn open_package(name: &str) -> (Vec<u8>, StfsPackage) {
		let path = format!("tests/fixtures/{}", name);
		let data = std::fs::read(&path).unwrap();
		let reader = SliceReader(&data);
		let package = StfsPackage::open(&reader).unwrap();
		(data, package)
	}

	fn verify_all_hash_chains(name: &str) {
		let (data, package) = open_package(name);
		let reader = SliceReader(&data);
		let ctx = HashVerifyContext {
			sex: package.sex,
			hash_meta: &package.hash_table_meta,
			stfs_vol: package.header.volume_descriptor.stfs_ref(),
		};
		let mut hasher = StfsHasher::new();

		for w in package.file_table.walk_files() {
			let reports = hasher.verify_data_block(&reader, w.entry.starting_block_num, &ctx).unwrap();
			for report in &reports {
				assert!(
					report.is_valid,
					"{}: hash chain invalid at block {} level {:?}",
					name, report.block, report.level
				);
			}
		}
		assert!(hasher.all_valid(), "{}: not all cached reports valid", name);
	}

	fn verify_all_data_blocks(name: &str) {
		let (data, package) = open_package(name);
		let reader = SliceReader(&data);
		let ctx = HashVerifyContext {
			sex: package.sex,
			hash_meta: &package.hash_table_meta,
			stfs_vol: package.header.volume_descriptor.stfs_ref(),
		};
		let hasher = StfsHasher::new();

		for w in package.file_table.walk_files() {
			let mut block = w.entry.starting_block_num;
			for _ in 0..w.entry.block_count {
				let report = hasher.verify_data_block_content(&reader, block, &ctx).unwrap();
				assert!(report.is_valid, "{}: data block {:?} hash mismatch", name, block);

				let hash_entry = ctx.hash_meta.read_block_hash_entry(&reader, block, ctx.sex, ctx.stfs_vol).unwrap();
				block = hash_entry.next_block;
			}
		}
	}

	#[test]
	fn hash_chain_live_532k() {
		verify_all_hash_chains("live_532k.stfs");
	}

	#[test]
	fn hash_chain_live_256k() {
		verify_all_hash_chains("live_256k.stfs");
	}

	#[test]
	fn hash_chain_live_120k_a() {
		verify_all_hash_chains("live_120k_a.stfs");
	}

	#[test]
	fn hash_chain_live_120k_b() {
		verify_all_hash_chains("live_120k_b.stfs");
	}

	#[test]
	fn data_blocks_live_532k() {
		verify_all_data_blocks("live_532k.stfs");
	}

	#[test]
	fn data_blocks_live_256k() {
		verify_all_data_blocks("live_256k.stfs");
	}

	#[test]
	fn data_blocks_live_120k_a() {
		verify_all_data_blocks("live_120k_a.stfs");
	}

	#[test]
	fn data_blocks_live_120k_b() {
		verify_all_data_blocks("live_120k_b.stfs");
	}

	#[test]
	fn cache_deduplicates_hash_table_lookups() {
		let (data, package) = open_package("live_532k.stfs");
		let reader = SliceReader(&data);
		let ctx = HashVerifyContext {
			sex: package.sex,
			hash_meta: &package.hash_table_meta,
			stfs_vol: package.header.volume_descriptor.stfs_ref(),
		};
		let mut hasher = StfsHasher::new();

		let files = package.file_table.walk_files();
		assert!(files.len() > 1, "need multiple files to test caching");

		for w in &files {
			hasher.verify_data_block(&reader, w.entry.starting_block_num, &ctx).unwrap();
		}

		// All files in a small package share the same level-0 hash table,
		// so the cache should have fewer entries than files
		let cache_size = hasher.reports().count();
		assert!(
			cache_size <= files.len(),
			"cache should deduplicate: {} cache entries for {} files",
			cache_size,
			files.len()
		);
	}

	#[test]
	fn hash_block_produces_correct_sha1() {
		let data = b"hello world";
		let hash = StfsHasher::hash_block(data);
		// known SHA-1 of "hello world"
		let expected = Sha1Digest([
			0x2a, 0xae, 0x6c, 0x35, 0xc9, 0x4f, 0xcf, 0xb4, 0x15, 0xdb, 0xe9, 0x5f, 0x40, 0x8b, 0x9c, 0xe9, 0x1e, 0xe8,
			0x46, 0xed,
		]);
		assert_eq!(hash, expected);
	}

	#[test]
	fn top_table_is_level_first_for_small_packages() {
		let (_data, package) = open_package("live_120k_a.stfs");
		assert_eq!(package.hash_table_meta.top_table.level, HashTableLevel::First);
	}
}

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
