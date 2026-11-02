//! The copyright text of the flash header. `XE_FLASH_HEADER` reserves 0x10..0x64; images keep
//! the text in 0x10..0x60 and a separate word at 0x60, which is left as it is. Nothing on the
//! shadowboot path reads the text: `HvpShadowBootStart` checks only the magic and the entry point.

use std::fmt;
use std::ops::Range;
use std::str::FromStr;

use thiserror::Error;

const FIELD: Range<usize> = 0x10..0x60;
/// The text and its terminating NUL fill the field at most.
const MAXIMUM_LENGTH: usize = FIELD.end - FIELD.start - 1;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CopyrightError {
	#[error("copyright text contains {character:?}, which is not a Latin-1 character")]
	NotLatin1 { character: char },
	#[error("copyright text is {length} bytes; the flash header holds at most {MAXIMUM_LENGTH}")]
	TooLong { length: usize },
}

/// Copyright text in Latin-1, as the flash header stores it. Microsoft's own text uses 0xa9 for
/// the copyright sign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copyright(Vec<u8>);

impl Copyright {
	pub fn bytes(&self) -> &[u8] {
		&self.0
	}

	/// Writes the text, its NUL and zero fill into the field of `header`, which must hold the
	/// 0x80-byte flash header.
	pub(crate) fn write(&self, header: &mut [u8]) {
		let field = &mut header[FIELD];
		field.fill(0);
		field[..self.0.len()].copy_from_slice(&self.0);
	}
}

impl fmt::Display for Copyright {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		// Latin-1 bytes are the first 256 code points.
		self.0.iter().try_for_each(|byte| fmt::Write::write_char(f, char::from(*byte)))
	}
}

impl FromStr for Copyright {
	type Err = CopyrightError;

	fn from_str(text: &str) -> Result<Self, Self::Err> {
		let bytes = text
			.chars()
			.map(|character| u8::try_from(u32::from(character)).map_err(|_| CopyrightError::NotLatin1 { character }))
			.collect::<Result<Vec<u8>, _>>()?;
		if bytes.len() > MAXIMUM_LENGTH {
			return Err(CopyrightError::TooLong { length: bytes.len() });
		}

		Ok(Self(bytes))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn text_is_latin1_and_bounded() {
		let copyright: Copyright = "\u{a9} 2026 Example".parse().unwrap();
		assert_eq!(copyright.bytes()[0], 0xa9);
		assert_eq!(copyright.bytes().len(), 14);
		assert_eq!("\u{2122}".parse::<Copyright>(), Err(CopyrightError::NotLatin1 { character: '\u{2122}' }));
		assert_eq!("x".repeat(0x50).parse::<Copyright>(), Err(CopyrightError::TooLong { length: 0x50 }));
		assert!("x".repeat(0x4f).parse::<Copyright>().is_ok());
	}
}
