//! File names and volume paths as the driver accepts them for a new or renamed entry.

use std::fmt;

use thiserror::Error;

use crate::entry::InvalidEntryReason;
use crate::entry::MAXIMUM_FILE_NAME_LENGTH;
use crate::entry::validate_name;

/// fatxp.h FATX_MAXIMUM_PATH_NAME_LENGTH: the driver refuses to create or rename an entry
/// whose path, from the separator before its first name, is longer (create.c:560,
/// fileinfo.c:356).
pub const MAXIMUM_PATH_LENGTH: usize = 250;

/// Why the driver would refuse a name.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
	#[error("the name is empty")]
	Empty,
	#[error("the name is {length} bytes; at most {MAXIMUM_FILE_NAME_LENGTH} fit")]
	TooLong { length: usize },
	#[error("the name is . or ..")]
	DotName,
	#[error("the name holds the illegal character {character:?}")]
	IllegalCharacter { character: char },
	/// A stored name holds one byte per character, read as Latin-1.
	#[error("the name holds {character:?}, which has no one-byte form")]
	NotLatin1 { character: char },
}

/// A name the driver accepts: 1 to 42 one-byte characters without control characters or
/// `" * + , / : ; < = > ? \ |`, and not `.` or `..` (fatsup.c FatxIsValidFatFileName).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct FileName {
	bytes: Vec<u8>,
}

impl FileName {
	/// The name of `name`'s characters as Latin-1 bytes, the form the reader shows.
	pub fn parse(name: &str) -> Result<Self, NameError> {
		let bytes = name
			.chars()
			.map(|character| u8::try_from(character).map_err(|_| NameError::NotLatin1 { character }))
			.collect::<Result<Vec<u8>, NameError>>()?;
		if bytes.len() > MAXIMUM_FILE_NAME_LENGTH {
			return Err(NameError::TooLong { length: bytes.len() });
		}

		validate_name(&bytes).map_err(|reason| match reason {
			InvalidEntryReason::DotName => NameError::DotName,
			InvalidEntryReason::IllegalCharacter { byte } => {
				NameError::IllegalCharacter { character: char::from(byte) }
			}
			_ => NameError::Empty,
		})?;

		Ok(Self { bytes })
	}

	pub fn as_bytes(&self) -> &[u8] {
		&self.bytes
	}

	pub fn len(&self) -> usize {
		self.bytes.len()
	}

	pub fn is_empty(&self) -> bool {
		self.bytes.is_empty()
	}

	/// Whether `stored` names the same entry. create.c FatxLookupElementNameInDirectory
	/// compares names with RtlCompareStringN, which ignores case through RtlUpperChar.
	pub fn matches(&self, stored: &[u8]) -> bool {
		self.bytes.len() == stored.len() && self.bytes.iter().zip(stored).all(|(a, b)| upper(*a) == upper(*b))
	}
}

impl fmt::Display for FileName {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.bytes.iter().try_for_each(|byte| write!(f, "{}", char::from(*byte)))
	}
}

impl fmt::Debug for FileName {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "FileName({:?})", self.to_string())
	}
}

/// A path in a volume: the names from the root directory, which has none.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct VolumePath {
	names: Vec<FileName>,
}

impl VolumePath {
	pub const fn root() -> Self {
		Self { names: Vec::new() }
	}

	/// The names of `path` between `/` separators. Leading, trailing and repeated separators
	/// are ignored.
	pub fn parse(path: &str) -> Result<Self, PathError> {
		let names = path
			.split('/')
			.filter(|name| !name.is_empty())
			.map(|name| FileName::parse(name).map_err(|reason| PathError { name: name.to_owned(), reason }))
			.collect::<Result<Vec<_>, _>>()?;

		Ok(Self { names })
	}

	pub fn names(&self) -> &[FileName] {
		&self.names
	}

	pub fn is_root(&self) -> bool {
		self.names.is_empty()
	}

	/// The parent directory and the last name; `None` for the root.
	pub fn split_last(&self) -> Option<(Self, &FileName)> {
		let (last, parent) = self.names.split_last()?;

		Some((Self { names: parent.to_vec() }, last))
	}

	/// The driver's PathNameLength of the entry: a separator and the name for each level.
	pub fn length(&self) -> usize {
		self.names.iter().map(|name| name.len() + 1).sum()
	}

	/// Whether `self` is `other` or inside it, comparing names as the driver does.
	pub fn starts_with(&self, other: &Self) -> bool {
		other.names.len() <= self.names.len()
			&& other.names.iter().zip(&self.names).all(|(prefix, name)| prefix.matches(name.as_bytes()))
	}
}

impl fmt::Display for VolumePath {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if self.names.is_empty() {
			return f.write_str("/");
		}

		self.names.iter().try_for_each(|name| write!(f, "/{name}"))
	}
}

/// A path with a name the driver would refuse.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("name {name:?} is invalid: {reason}")]
pub struct PathError {
	pub name: String,
	#[source]
	pub reason: NameError,
}

/// RtlUpperChar for the one-byte character `byte`. ASCII letters upcase directly; the rest go
/// through the Unicode case table Nls844UnicodeCaseTable (rtl/string.c:36-176, NLS_UPCASE).
/// Evaluating that table for 0x80 to 0xff gives: 0xe0 to 0xfe except 0xf7 move down by 0x20,
/// and 0xff, whose upper case form U+0178 has no one-byte form, becomes `?`.
fn upper(byte: u8) -> u8 {
	match byte {
		b'a'..=b'z' | 0xe0..=0xf6 | 0xf8..=0xfe => byte - 0x20,
		0xff => b'?',
		_ => byte,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn driver_name_rules() {
		assert_eq!(FileName::parse(""), Err(NameError::Empty));
		assert_eq!(FileName::parse("."), Err(NameError::DotName));
		assert_eq!(FileName::parse(".."), Err(NameError::DotName));
		assert_eq!(FileName::parse("a:b"), Err(NameError::IllegalCharacter { character: ':' }));
		assert_eq!(FileName::parse("a\u{1}"), Err(NameError::IllegalCharacter { character: '\u{1}' }));
		assert_eq!(FileName::parse(&"x".repeat(43)), Err(NameError::TooLong { length: 43 }));
		assert_eq!(FileName::parse("\u{263a}"), Err(NameError::NotLatin1 { character: '\u{263a}' }));

		assert_eq!(FileName::parse(&"x".repeat(42)).unwrap().len(), 42);
		assert_eq!(FileName::parse("...").unwrap().as_bytes(), b"...");
		assert_eq!(FileName::parse("caf\u{e9}").unwrap().as_bytes(), b"caf\xe9");
	}

	#[test]
	fn names_match_without_case() {
		let name = FileName::parse("Caf\u{e9}.bin").unwrap();

		assert!(name.matches(b"CAF\xc9.BIN"));
		assert!(!name.matches(b"Cafe.bin"));
		// The multiplication sign 0xd7 and the division sign 0xf7 are not a case pair.
		assert!(!FileName::parse("\u{f7}").unwrap().matches(b"\xd7"));
	}

	#[test]
	fn paths_ignore_extra_separators() {
		let path = VolumePath::parse("//Content/0000/").unwrap();

		assert_eq!(path.to_string(), "/Content/0000");
		assert_eq!(path.length(), "/Content/0000".len());
		assert!(VolumePath::parse("/").unwrap().is_root());
		assert!(path.starts_with(&VolumePath::parse("/content").unwrap()));
		assert!(!VolumePath::parse("/Contents").unwrap().starts_with(&VolumePath::parse("/Content").unwrap()));
	}
}
