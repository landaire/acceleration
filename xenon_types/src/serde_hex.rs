use serde::de;
use serde::Deserializer;
use serde::Serializer;

fn serialize_hex<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
	let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
	s.serialize_str(&hex)
}

fn deserialize_hex<'de, D: Deserializer<'de>>(d: D, expected_len: usize) -> Result<Vec<u8>, D::Error> {
	let s: &str = de::Deserialize::deserialize(d)?;
	if s.len() != expected_len * 2 {
		return Err(de::Error::custom(format!("expected {} hex chars, got {}", expected_len * 2, s.len())));
	}
	let mut bytes = Vec::with_capacity(expected_len);
	for i in 0..expected_len {
		let byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(de::Error::custom)?;
		bytes.push(byte);
	}
	Ok(bytes)
}

macro_rules! hex_serde_mod {
	($name:ident, $len:expr) => {
		pub mod $name {
			use serde::Deserializer;
			use serde::Serializer;

			pub fn serialize<S: Serializer>(bytes: &[u8; $len], s: S) -> Result<S::Ok, S::Error> {
				super::serialize_hex(bytes, s)
			}

			pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; $len], D::Error> {
				let bytes = super::deserialize_hex(d, $len)?;
				bytes.try_into().map_err(|_| serde::de::Error::custom(concat!("expected ", stringify!($len), " bytes")))
			}
		}
	};
}

hex_serde_mod!(fixed5, 5);
hex_serde_mod!(fixed8, 8);
hex_serde_mod!(fixed20, 20);
