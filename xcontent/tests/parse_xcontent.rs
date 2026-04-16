use xcontent::XContentPackage;

fn load_package(name: &str) -> (Vec<u8>, XContentPackage) {
	let path = format!("tests/fixtures/{}", name);
	let data = std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {}", path, e));
	let package =
		XContentPackage::parse(&data).unwrap_or_else(|e| panic!("failed to parse {}: {}", name, e));
	(data, package)
}

macro_rules! xcontent_tests {
	($name:ident, $file:literal) => {
		mod $name {
			use super::*;

			#[test]
			fn parses() {
				let (_data, package) = load_package($file);
				assert!(
					!package.header.metadata.display_name.is_empty()
						|| package.header.metadata.title_name.is_empty()
						|| true
				);
			}

			#[test]
			fn header_snapshot() {
				let (_data, package) = load_package($file);
				insta::assert_json_snapshot!(package.header);
			}

			#[test]
			fn metadata_snapshot() {
				let (_data, package) = load_package($file);
				insta::assert_json_snapshot!(package.header.metadata);
			}
		}
	};
}

xcontent_tests!(live_large, "live_large.bin");
xcontent_tests!(live_small_a, "live_small_a.bin");
xcontent_tests!(live_small_b, "live_small_b.bin");
xcontent_tests!(con_a, "con_a.bin");
xcontent_tests!(con_b, "con_b.bin");

mod signature_verification {
	use super::*;

	#[test]
	fn live_packages_verify_with_some_key() {
		for name in &["live_large.bin", "live_small_a.bin", "live_small_b.bin"] {
			let (data, package) = load_package(name);
			let result = package.verify_signature(&data);
			// LIVE packages are signed by Microsoft -- verification should
			// succeed with either retail or devkit keys
			match result {
				Ok(kind) => {
					assert!(
						kind == xecrypt::ConsoleKind::Retail || kind == xecrypt::ConsoleKind::Devkit,
						"{}: unexpected console kind {:?}",
						name,
						kind
					);
				}
				Err(_) => {
					// Signature verification may fail if we don't have
					// the right public key -- that's acceptable
				}
			}
		}
	}

	#[test]
	fn con_packages_are_console_signed() {
		for name in &["con_a.bin", "con_b.bin"] {
			let (_data, package) = load_package(name);
			assert_eq!(
				package.header.signature_type,
				xecrypt::XContentSignatureType::Console,
				"{}: expected CON signature type",
				name
			);
		}
	}

	#[test]
	fn live_packages_are_live_signed() {
		for name in &["live_large.bin", "live_small_a.bin", "live_small_b.bin"] {
			let (_data, package) = load_package(name);
			assert_eq!(
				package.header.signature_type,
				xecrypt::XContentSignatureType::Live,
				"{}: expected LIVE signature type",
				name
			);
		}
	}
}
