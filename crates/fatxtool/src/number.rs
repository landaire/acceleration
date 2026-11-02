//! Command-line numbers: decimal, or hexadecimal with a `0x` prefix.

use fatx::ClusterNumber;

use crate::error::NumberError;

pub fn parse_number(text: &str) -> Result<u64, NumberError> {
	let parsed = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
		Some(digits) => u64::from_str_radix(digits, 16),
		None => text.parse(),
	};

	parsed.map_err(|source| NumberError::Invalid { text: text.to_owned(), source })
}

pub fn parse_u32(text: &str) -> Result<u32, NumberError> {
	let value = parse_number(text)?;

	u32::try_from(value).map_err(|_| NumberError::TooLarge { value })
}

pub fn parse_cluster(text: &str) -> Result<ClusterNumber, NumberError> {
	ClusterNumber::new(parse_u32(text)?).ok_or(NumberError::ZeroCluster)
}
