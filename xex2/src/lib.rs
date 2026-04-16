//! Parser and extractor for Xbox 360 XEX2 executables.
//!
//! XEX2 is the executable format used by the Xbox 360. An XEX file wraps a
//! Windows PE image with added metadata, optional compression (basic or LZX),
//! and optional AES-128-CBC encryption. The kernel verifies an RSA signature
//! over the security info before loading.
//!
//! # Quick start
//!
//! ```no_run
//! use xex2::Xex2;
//!
//! let data = std::fs::read("game.xex").unwrap();
//! let xex = Xex2::parse(data).unwrap();
//!
//! // Read metadata from the parsed header
//! if let Some(exec) = xex.header.execution_info() {
//!     println!("Title ID: {:#010x}", exec.title_id.0);
//! }
//!
//! // Extract the inner PE image
//! let pe = xex.extract_basefile().unwrap();
//! std::fs::write("game.pe", pe).unwrap();
//! ```
//!
//! # Modules
//!
//! - [`header`] -- the raw XEX2 header, security info, and optional header keys
//! - [`opt`] -- typed accessors for optional headers (system flags, game ratings,
//!   date range, etc.)
//! - [`imports`] -- import library parsing
//! - [`basefile`] -- decryption and decompression of the inner PE image
//! - [`writer`] -- modifying an XEX (removing restrictions, re-signing)
//!
//! # Features
//!
//! - `serde` (default off) -- enables `Serialize` on all public types.

pub mod basefile;
pub mod crypto;
pub mod error;
pub mod header;
pub mod imports;
pub mod opt;
pub mod writer;

use crate::error::Result;
use crate::header::SecurityInfo;
use crate::header::Xex2Header;

/// A parsed XEX2 executable.
///
/// Owns the raw file bytes so accessors that need to read into the data section
/// (e.g. [`extract_basefile`][Self::extract_basefile]) can return without
/// additional I/O.
///
/// # Example
///
/// ```no_run
/// use xex2::Xex2;
///
/// let data = std::fs::read("game.xex").unwrap();
/// let xex = Xex2::parse(data).unwrap();
/// println!("Load address: {:#010x}", xex.security_info.image_info.load_address.0);
/// ```
pub struct Xex2 {
	pub header: Xex2Header,
	pub security_info: SecurityInfo,
	raw: Vec<u8>,
}

impl Xex2 {
	/// Parse an XEX2 file from its raw bytes.
	///
	/// Validates the `XEX2` magic, parses the header and security info.
	/// Does not verify the RSA signature or decrypt the image -- that happens
	/// lazily via [`extract_basefile`][Self::extract_basefile].
	///
	/// # Errors
	///
	/// Returns an error if the magic is wrong, the header is truncated, or
	/// any offset in the header points outside the file.
	pub fn parse(data: Vec<u8>) -> Result<Self> {
		let header = Xex2Header::parse(&data)?;
		let security_info = SecurityInfo::parse(&data, header.security_offset as usize)?;
		Ok(Xex2 { header, security_info, raw: data })
	}

	/// The raw XEX file bytes.
	pub fn raw(&self) -> &[u8] {
		&self.raw
	}

	/// Decrypt and decompress the inner PE image.
	///
	/// Tries the retail AES key first, then falls back to the devkit key.
	/// For compressed XEXs, performs basic block decompression or LZX
	/// decompression based on the file format info. The result starts
	/// with the `MZ` PE signature.
	pub fn extract_basefile(&self) -> Result<Vec<u8>> {
		basefile::extract_basefile(&self.raw, &self.header, &self.security_info)
	}

	/// Apply restriction removals and produce a re-signed XEX.
	///
	/// Modifies the specified fields in ImageInfo, recomputes the RotSumSha
	/// hash, and signs it with the devkit PIRS private key. The result is a
	/// valid devkit-signed XEX (retail consoles won't load it without further
	/// patching, but devkit/JTAG/RGH consoles will).
	///
	/// Use [`writer::RemoveLimits::all`] to enable every restriction removal,
	/// or set specific fields manually.
	pub fn modify(&self, limits: &writer::RemoveLimits) -> Result<Vec<u8>> {
		writer::modify_xex(
			self,
			writer::TargetEncryption::Unchanged,
			writer::TargetCompression::Unchanged,
			writer::TargetMachine::Unchanged,
			limits,
		)
	}
}
