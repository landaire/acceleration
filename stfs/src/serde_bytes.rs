pub mod fixed {
	use serde::Deserializer;
	use serde::Serializer;
	use serde::de;

	#[cfg(feature = "base64-serde")]
	use base64::Engine;
	#[cfg(feature = "base64-serde")]
	use base64::engine::general_purpose::STANDARD;

	#[cfg(feature = "base64-serde")]
	pub fn serialize<const N: usize, S: Serializer>(bytes: &[u8; N], s: S) -> Result<S::Ok, S::Error> {
		s.serialize_str(&STANDARD.encode(bytes))
	}

	#[cfg(not(feature = "base64-serde"))]
	pub fn serialize<const N: usize, S: Serializer>(bytes: &[u8; N], s: S) -> Result<S::Ok, S::Error> {
		s.serialize_bytes(bytes)
	}

	#[cfg(feature = "base64-serde")]
	pub fn deserialize<'de, const N: usize, D: Deserializer<'de>>(d: D) -> Result<[u8; N], D::Error> {
		let s: &str = de::Deserialize::deserialize(d)?;
		let bytes = STANDARD.decode(s).map_err(de::Error::custom)?;
		bytes.try_into().map_err(|v: Vec<u8>| de::Error::custom(format!("expected {} bytes, got {}", N, v.len())))
	}

	#[cfg(not(feature = "base64-serde"))]
	pub fn deserialize<'de, const N: usize, D: Deserializer<'de>>(d: D) -> Result<[u8; N], D::Error> {
		use de::Visitor;

		struct ByteArrayVisitor<const M: usize>;

		impl<'de, const M: usize> Visitor<'de> for ByteArrayVisitor<M> {
			type Value = [u8; M];

			fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
				write!(f, "{} bytes", M)
			}

			fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<[u8; M], E> {
				v.try_into().map_err(|_| de::Error::custom(format!("expected {} bytes, got {}", M, v.len())))
			}

			fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<[u8; M], A::Error> {
				let mut arr = [0u8; M];
				for (i, byte) in arr.iter_mut().enumerate() {
					*byte = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(i, &self))?;
				}
				Ok(arr)
			}
		}

		d.deserialize_any(ByteArrayVisitor::<N>)
	}
}

pub mod fixed_opt {
	use serde::Deserialize;
	use serde::Deserializer;
	use serde::Serializer;
	use serde::de;

	pub fn serialize<const N: usize, S: Serializer>(bytes: &Option<[u8; N]>, s: S) -> Result<S::Ok, S::Error> {
		match bytes {
			Some(b) => super::fixed::serialize(b, s),
			None => s.serialize_none(),
		}
	}

	pub fn deserialize<'de, const N: usize, D: Deserializer<'de>>(d: D) -> Result<Option<[u8; N]>, D::Error> {
		let opt: Option<HelperProxy<N>> = Deserialize::deserialize(d)?;
		Ok(opt.map(|h| h.0))
	}

	struct HelperProxy<const N: usize>([u8; N]);

	impl<'de, const N: usize> de::Deserialize<'de> for HelperProxy<N> {
		fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
			super::fixed::deserialize(d).map(HelperProxy)
		}
	}
}

pub mod vec {
	use serde::Deserializer;
	use serde::Serializer;
	use serde::de;

	#[cfg(feature = "base64-serde")]
	use base64::Engine;
	#[cfg(feature = "base64-serde")]
	use base64::engine::general_purpose::STANDARD;

	#[cfg(feature = "base64-serde")]
	pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
		s.serialize_str(&STANDARD.encode(bytes))
	}

	#[cfg(not(feature = "base64-serde"))]
	pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
		s.serialize_bytes(bytes)
	}

	#[cfg(feature = "base64-serde")]
	pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
		let s: &str = de::Deserialize::deserialize(d)?;
		STANDARD.decode(s).map_err(de::Error::custom)
	}

	#[cfg(not(feature = "base64-serde"))]
	pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
		use de::Visitor;

		struct ByteVecVisitor;

		impl<'de> Visitor<'de> for ByteVecVisitor {
			type Value = Vec<u8>;

			fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
				write!(f, "byte array")
			}

			fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Vec<u8>, E> {
				Ok(v.to_vec())
			}

			fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<u8>, A::Error> {
				let mut vec = Vec::with_capacity(seq.size_hint().unwrap_or(0));
				while let Some(b) = seq.next_element()? {
					vec.push(b);
				}
				Ok(vec)
			}
		}

		d.deserialize_any(ByteVecVisitor)
	}
}
