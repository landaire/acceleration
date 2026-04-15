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
