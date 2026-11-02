//! Fixed-length text fields of on-disk structures.

use std::fmt;

use serde::Serialize;
use serde::Serializer;

/// A fixed-length ASCII field, padded with spaces or NULs. Display trims the padding and
/// escapes bytes that are not printable ASCII; serialization writes the displayed text.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FixedText<const N: usize>([u8; N]);

impl<const N: usize> FixedText<N> {
	pub const fn new(bytes: [u8; N]) -> Self {
		Self(bytes)
	}

	pub const fn bytes(&self) -> &[u8; N] {
		&self.0
	}

	/// The text before the padding.
	pub fn trimmed(&self) -> &[u8] {
		let end = self.0.iter().rposition(|byte| *byte != b' ' && *byte != 0).map_or(0, |last| last + 1);
		let start = self.0[..end].iter().position(|byte| *byte != b' ').unwrap_or(end);

		&self.0[start..end]
	}

	/// Whether the text before the padding is not empty and is all printable ASCII.
	pub fn is_printable(&self) -> bool {
		let text = self.trimmed();

		!text.is_empty() && text.iter().all(|byte| (0x20..0x7f).contains(byte))
	}
}

impl<const N: usize> fmt::Display for FixedText<N> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.trimmed().escape_ascii())
	}
}

impl<const N: usize> fmt::Debug for FixedText<N> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{:?}", self.0.escape_ascii().to_string())
	}
}

impl<const N: usize> Serialize for FixedText<N> {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serializer.collect_str(self)
	}
}
