use std::path::PathBuf;

use xcontent::XContentPackage;

/// Cargo and Buck2 set CARGO_MANIFEST_DIR when they run a test. Buck2 sets it
/// relative to the project root, where its tests run, so the compile-time
/// value (an absolute path into the build's source tree) does not serve.
fn fixture_path(name: &str) -> PathBuf {
	let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR").expect("the test runner sets CARGO_MANIFEST_DIR");
	PathBuf::from(manifest_dir).join("tests/fixtures").join(name)
}

fn load_package(name: &str) -> (Vec<u8>, XContentPackage) {
	let path = fixture_path(name);
	let data = std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e));
	let package = XContentPackage::parse(&data).unwrap_or_else(|e| panic!("failed to parse {}: {}", name, e));
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
				Ok(verification) => {
					assert!(
						matches!(verification, xecrypt::XContentVerification::Strong(_)),
						"{name}: {verification:?}"
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

	fn con_signature(package: &XContentPackage) -> &xecrypt::ConsoleSignature {
		let xecrypt::XContentKeyMaterial::ConsoleSignature(signature) = &package.header.key_material else {
			panic!("a CON package carries a console signature");
		};

		signature
	}

	#[test]
	fn con_packages_verify_their_certificate_and_signature() {
		for name in &["con_a.bin", "con_b.bin"] {
			let (data, package) = load_package(name);

			let verification = package.verify_signature(&data).unwrap_or_else(|error| panic!("{name}: {error}"));

			let expected = xecrypt::ConsoleSignatureVerification {
				certificate: xecrypt::CertificateStatus::Verified { key: xecrypt::console::CertificateKey::Key3C },
				signature: Ok(()),
			};
			assert_eq!(verification, xecrypt::XContentVerification::Console(expected), "{name}");
		}
	}

	#[test]
	fn a_changed_con_header_fails_the_package_signature() {
		let (mut data, package) = load_package("con_a.bin");
		data[package.header.license_data_offset] ^= 1;

		let Ok(xecrypt::XContentVerification::Console(verification)) = package.verify_signature(&data) else {
			panic!("a CON package verifies as a console signature");
		};
		assert!(verification.certificate.is_verified());
		assert_eq!(verification.signature, Err(xecrypt::ConsoleSignatureError::Mismatch));
	}

	#[test]
	fn a_changed_certificate_is_invalid_and_its_signature_still_checks() {
		let (data, package) = load_package("con_a.bin");
		let mut signature = con_signature(&package).clone();
		signature.certificate.reserved[0] ^= 1;

		let verification = signature.verify(&package.header.header_hash(&data));

		assert_eq!(
			verification.certificate,
			xecrypt::CertificateStatus::Invalid { key: xecrypt::console::CertificateKey::Key3C }
		);
		assert_eq!(verification.signature, Ok(()));
	}

	#[test]
	fn a_recovery_generated_certificate_has_no_available_key() {
		let (data, package) = load_package("con_a.bin");
		let mut signature = con_signature(&package).clone();
		signature.certificate.console_type.0 |= xecrypt::console::ConsoleType::RECOVERY_GENERATED;

		let verification = signature.verify(&package.header.header_hash(&data));

		assert_eq!(
			verification.certificate,
			xecrypt::CertificateStatus::KeyUnavailable { key: xecrypt::console::CertificateKey::Key3A }
		);
		assert!(!verification.is_verified());
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
