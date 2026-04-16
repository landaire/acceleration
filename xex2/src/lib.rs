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
//! let xex = Xex2::parse(&data).unwrap();
//!
//! // Read metadata from the parsed header
//! if let Some(exec) = xex.header.execution_info() {
//!     println!("Title ID: {:#010x}", exec.title_id.0);
//! }
//!
//! // Extract the inner PE image
//! let pe = xex.extract_basefile(&data).unwrap();
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
//! - [`patch`] -- describing modifications as byte-level edits
//! - [`writer`] -- planning length-preserving edits + re-signing (small-edit path)
//! - [`rebuild`] -- streaming rebuilds (transformative-edit path)
//!
//! # Features
//!
//! - `serde` (default off) -- enables `Serialize` on all public types.

pub mod basefile;
pub mod builder;
pub mod crypto;
pub mod error;
pub mod hashes;
pub mod header;
pub mod imports;
pub mod opt;
pub mod page_descriptors;
pub mod patch;
pub mod rebuild;
pub mod writer;

use crate::error::Result;
use crate::header::SecurityInfo;
use crate::header::Xex2Header;

/// A parsed XEX2 executable.
///
/// Holds only the parsed header and security info; the raw file bytes are
/// passed back in to methods that need them (e.g.
/// [`extract_basefile`][Self::extract_basefile]).
///
/// # Example
///
/// ```no_run
/// use xex2::Xex2;
///
/// let data = std::fs::read("game.xex").unwrap();
/// let xex = Xex2::parse(&data).unwrap();
/// println!("Load address: {:#010x}", xex.security_info.image_info.load_address.0);
/// ```
pub struct Xex2 {
	pub header: Xex2Header,
	pub security_info: SecurityInfo,
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
	pub fn parse(data: impl AsRef<[u8]>) -> Result<Self> {
		Self::parse_inner(data.as_ref())
	}

	fn parse_inner(data: &[u8]) -> Result<Self> {
		let header = Xex2Header::parse(data)?;
		let security_info = SecurityInfo::parse(data, header.security_offset as usize)?;
		Ok(Xex2 { header, security_info })
	}

	/// Decrypt and decompress the inner PE image.
	///
	/// Tries the retail AES key first, then falls back to the devkit key.
	/// For compressed XEXs, performs basic block decompression or LZX
	/// decompression based on the file format info. The result starts
	/// with the `MZ` PE signature.
	pub fn extract_basefile(&self, data: impl AsRef<[u8]>) -> Result<Vec<u8>> {
		basefile::extract_basefile(data.as_ref(), &self.header, &self.security_info)
	}

	/// Start a streaming rebuild of this XEX.
	///
	/// Consumes `self` so the caller can't accidentally reuse a stale
	/// [`Xex2`] against the newly-written output. After `write_to`, parse the
	/// resulting bytes with [`parse`][Self::parse] if further work is needed.
	///
	/// Configure transforms on the returned [`rebuild::Rebuilder`] and call
	/// [`write_to`][rebuild::Rebuilder::write_to] to stream the result, or
	/// [`as_patch`][rebuild::Rebuilder::as_patch] to get a [`patch::Patch`]
	/// when the rebuild is length-preserving.
	pub fn rebuild<'a>(self, data: &'a [u8]) -> rebuild::Rebuilder<'a> {
		rebuild::Rebuilder::new(self, data)
	}

	/// Apply restriction removals and produce a re-signed XEX.
	///
	/// Consumes `self`; parse the returned bytes if further work is needed.
	/// Convenience wrapper: plans the edits as a [`patch::Patch`] and applies
	/// them to an owned buffer. For streaming output (avoid buffering the full
	/// XEX), use [`rebuild`][Self::rebuild] + `write_to` instead.
	pub fn modify(self, data: impl AsRef<[u8]>, limits: &writer::RemoveLimits) -> Result<Vec<u8>> {
		let data = data.as_ref();
		let plan: writer::EditPlan = limits.into();
		let patch = writer::plan_edits(&self, data, &plan)?;
		let mut out = data.to_vec();
		patch.apply_to_vec(&mut out)?;
		Ok(out)
	}
}
