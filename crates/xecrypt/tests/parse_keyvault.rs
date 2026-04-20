use xecrypt::keyvault::KeyVault;

fn load_kv(name: &str) -> KeyVault {
	let path = format!("tests/fixtures/{}", name);
	let data = std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {}", path, e));
	KeyVault::parse(&data).unwrap_or_else(|e| panic!("failed to parse {}: {}", name, e))
}

macro_rules! kv_tests {
	($name:ident, $file:literal) => {
		mod $name {
			use super::*;

			#[test]
			fn parses() {
				let kv = load_kv($file);
				assert!(!kv.keys.console_serial_number.is_empty());
			}

			#[test]
			fn snapshot() {
				let kv = load_kv($file);
				insta::assert_toml_snapshot!(kv);
			}
		}
	};
}

kv_tests!(kv_01, "kv_01.bin");
kv_tests!(kv_02, "kv_02.bin");
kv_tests!(kv_03, "kv_03.bin");
kv_tests!(kv_04, "kv_04.bin");
kv_tests!(kv_05, "kv_05.bin");
kv_tests!(kv_06, "kv_06.bin");
kv_tests!(kv_07, "kv_07.bin");
kv_tests!(kv_08, "kv_08.bin");
kv_tests!(kv_09, "kv_09.bin");

mod zerocopy {
	use xecrypt::keyvault::KeyVault;
	use xecrypt::keyvault::KeyVaultRef;

	#[test]
	fn ref_matches_owned() {
		let data = std::fs::read("tests/fixtures/kv_02.bin").unwrap();
		let kv_ref = KeyVaultRef::parse(&data).unwrap();
		let kv_owned = KeyVault::parse(&data).unwrap();

		assert_eq!(kv_ref.console_serial(), kv_owned.console_serial());
		assert_eq!(kv_ref.dvd_key(), kv_owned.dvd_key());
		assert_eq!(kv_ref.game_region(), kv_owned.game_region());
		assert_eq!(*kv_ref.console_id(), *kv_owned.console_id());
		assert_eq!(kv_ref.console_certificate.console_part_number, kv_owned.console_certificate.console_part_number);
		assert_eq!(kv_ref.console_type(), *kv_owned.console_type());
		assert_eq!(kv_ref.is_devkit(), kv_owned.is_devkit());
		assert_eq!(kv_ref.is_retail(), kv_owned.is_retail());
		assert_eq!(kv_ref.revision(), kv_owned.revision());
		assert_eq!(kv_ref.keys.console_private_key, kv_owned.keys.console_private_key.as_slice());
		assert_eq!(kv_ref.keys.xeika_private_key, kv_owned.keys.xeika_private_key.as_slice());
		assert_eq!(kv_ref.keys.cardea_private_key, kv_owned.keys.cardea_private_key.as_slice());
		assert_eq!(kv_ref.config.manufacturing_mode, kv_owned.config.manufacturing_mode);
		assert_eq!(kv_ref.config.restricted_privileges, kv_owned.config.restricted_privileges);
	}

	#[test]
	fn ref_works_with_truncated() {
		let data = std::fs::read("tests/fixtures/kv_03.bin").unwrap();
		let kv_ref = KeyVaultRef::parse(&data).unwrap();
		assert!(!kv_ref.console_serial().is_empty());
		assert_eq!(kv_ref.console_serial(), "804287655006");
	}
}
