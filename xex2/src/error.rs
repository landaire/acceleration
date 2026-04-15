use std::fmt;

use rootcause::IntoReport;

#[derive(Debug)]
pub enum Xex2Error {
	InvalidMagic { found: [u8; 4] },
	UnsupportedModuleFlags(u32),
	InvalidHeaderOffset { offset: u32, file_size: usize },
	InvalidSecurityOffset { offset: u32, file_size: usize },
	MissingOptionalHeader(u32),
	InvalidOptionalHeaderSize { key: u32, size: usize },
	InvalidCompressionFormat(u16),
	InvalidEncryptionType(u16),
	DecryptionFailed,
	DecompressionFailed,
	InvalidBasefileSize { expected: usize, got: usize },
	HashMismatch { block_index: usize },
	Io(std::io::Error),
}

impl fmt::Display for Xex2Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::InvalidMagic { found } => {
				write!(f, "invalid XEX magic: {:02x}{:02x}{:02x}{:02x}", found[0], found[1], found[2], found[3])
			}
			Self::UnsupportedModuleFlags(flags) => write!(f, "unsupported module flags: {:#010x}", flags),
			Self::InvalidHeaderOffset { offset, file_size } => {
				write!(f, "header offset {:#x} exceeds file size {:#x}", offset, file_size)
			}
			Self::InvalidSecurityOffset { offset, file_size } => {
				write!(f, "security offset {:#x} exceeds file size {:#x}", offset, file_size)
			}
			Self::MissingOptionalHeader(key) => write!(f, "missing required optional header {:#010x}", key),
			Self::InvalidOptionalHeaderSize { key, size } => {
				write!(f, "optional header {:#010x} has invalid size {}", key, size)
			}
			Self::InvalidCompressionFormat(v) => write!(f, "invalid compression format: {}", v),
			Self::InvalidEncryptionType(v) => write!(f, "invalid encryption type: {}", v),
			Self::DecryptionFailed => write!(f, "decryption failed"),
			Self::DecompressionFailed => write!(f, "decompression failed"),
			Self::InvalidBasefileSize { expected, got } => {
				write!(f, "basefile size mismatch: expected {:#x}, got {:#x}", expected, got)
			}
			Self::HashMismatch { block_index } => write!(f, "hash mismatch at block {}", block_index),
			Self::Io(e) => write!(f, "I/O error: {}", e),
		}
	}
}

impl std::error::Error for Xex2Error {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Io(e) => Some(e),
			_ => None,
		}
	}
}

pub type Report = rootcause::Report<Xex2Error>;
pub type Result<T> = std::result::Result<T, Report>;

pub trait IoResultExt<T> {
	fn io(self) -> Result<T>;
}

impl<T> IoResultExt<T> for std::result::Result<T, std::io::Error> {
	fn io(self) -> Result<T> {
		self.map_err(|e| Xex2Error::Io(e).into_report())
	}
}
