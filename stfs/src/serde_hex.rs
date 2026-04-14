pub mod fixed {
	use serde::de;
	use serde::Deserializer;
	use serde::Serializer;

	pub fn serialize<const N: usize, S: Serializer>(bytes: &[u8; N], s: S) -> Result<S::Ok, S::Error> {
		let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
		s.serialize_str(&hex)
	}

	pub fn deserialize<'de, const N: usize, D: Deserializer<'de>>(d: D) -> Result<[u8; N], D::Error> {
		let s: &str = de::Deserialize::deserialize(d)?;
		if s.len() != N * 2 {
			return Err(de::Error::custom(format!("expected {} hex chars, got {}", N * 2, s.len())));
		}
		let mut arr = [0u8; N];
		for (i, byte) in arr.iter_mut().enumerate() {
			*byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(de::Error::custom)?;
		}
		Ok(arr)
	}
}
