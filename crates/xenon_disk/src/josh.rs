//! The block that starts with `Josh` at byte 0x800 of a hard disk. The kernel never reads it.
//! Titles maintain it through the XAPI utility drive code that is linked into them
//! (XMountUtilityDrive): the code reads 0x400 bytes at 0x800 of the raw disk and writes back
//! 0x284 of them. The layout here comes from that code in `TakehomeRecorder.lex` on SystemAux,
//! based at 0x98000000: sub_980112e8, sub_98010538, sub_980105b8 and sub_98010be0.
//!
//! The block holds the console signature (`XE_CONSOLE_SIGNATURE`) of the console that last
//! wrote it, over a table of the two utility drive cache partitions, `\Device\Harddisk0\Cache0`
//! and `Cache1`. When the magic, the console signature or the version does not check, the title
//! writes a new block and reformats both caches.

use std::fmt;

use sha1::Digest;
use sha1::Sha1;
use xecrypt::ConsoleSignature;
use xecrypt::ConsoleSignatureVerification;
use xecrypt::console::CONSOLE_SIGNATURE_SIZE;
use xenon_types::TitleId;

use crate::bytes::array;
use crate::error::Hex;
use crate::error::JoshBlockError;

/// Byte offset of the block on the disk.
pub const JOSH_BLOCK_OFFSET: u64 = 0x800;
/// The bytes the title writes back.
pub const JOSH_BLOCK_SIZE: usize = 0x284;
/// The version the title requires.
pub const JOSH_BLOCK_VERSION: u32 = 1;
const MAGIC: [u8; 4] = *b"Josh";
const CONSOLE_SIGNATURE_OFFSET: usize = 0x04;
const CACHE_TABLE_OFFSET: usize = CONSOLE_SIGNATURE_OFFSET + CONSOLE_SIGNATURE_SIZE;
const CACHE_ENTRY_SIZE: usize = 0x24;

/// Which utility drive cache partition an entry or index names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CacheIndex(u32);

impl CacheIndex {
	pub const fn new(value: u32) -> Self {
		Self(value)
	}

	pub const fn get(self) -> u32 {
		self.0
	}

	/// The cache partition the index names, when it is 0 or 1.
	pub const fn partition_name(self) -> Option<&'static str> {
		match self.0 {
			0 => Some("Cache0"),
			1 => Some("Cache1"),
			_ => None,
		}
	}
}

impl fmt::Display for CacheIndex {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

/// The entry of one cache partition. Only byte 1 is known to be read: the title compares it with
/// a value of its own against 0xf0. The other fields are kept as stored; on a development kit an
/// entry in use read `24 f0 00 01`, a zero word, a 20-byte value that looks like a digest, 4 and 2.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CacheEntry {
	pub bytes_00: [u8; 4],
	pub word_04: u32,
	pub bytes_08: [u8; 20],
	pub word_1c: u32,
	pub word_20: u32,
}

impl CacheEntry {
	fn parse(bytes: &[u8]) -> Self {
		Self {
			bytes_00: array(bytes, 0x00),
			word_04: u32::from_be_bytes(array(bytes, 0x04)),
			bytes_08: array(bytes, 0x08),
			word_1c: u32::from_be_bytes(array(bytes, 0x1c)),
			word_20: u32::from_be_bytes(array(bytes, 0x20)),
		}
	}

	/// Byte 1, which the title compares against 0xf0.
	pub const fn compared_byte(&self) -> u8 {
		self.bytes_00[1]
	}

	/// Whether the entry is zero, as the title leaves it when it reformats the cache.
	pub fn is_zero(&self) -> bool {
		*self == Self { bytes_00: [0; 4], word_04: 0, bytes_08: [0; 20], word_1c: 0, word_20: 0 }
	}
}

impl fmt::Debug for CacheEntry {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("CacheEntry")
			.field("bytes_00", &format_args!("{}", Hex(&self.bytes_00)))
			.field("word_04", &format_args!("{:#x}", self.word_04))
			.field("bytes_08", &format_args!("{}", Hex(&self.bytes_08)))
			.field("word_1c", &format_args!("{:#x}", self.word_1c))
			.field("word_20", &format_args!("{:#x}", self.word_20))
			.finish()
	}
}

#[derive(Clone, PartialEq, Eq)]
pub struct JoshBlock {
	pub console_signature: Box<ConsoleSignature>,
	/// Cache0, then Cache1.
	pub caches: [CacheEntry; 2],
	/// [`JOSH_BLOCK_VERSION`] in a block the title accepts.
	pub version: u32,
	/// The cache the title assigned last.
	pub last_assigned: CacheIndex,
	/// The title that owns Cache0, then Cache1.
	pub owners: [Option<TitleId>; 2],
	/// SHA-1 of the cache table, the bytes the console signature signs.
	signed_digest: [u8; 20],
}

impl JoshBlock {
	pub fn parse(bytes: &[u8; JOSH_BLOCK_SIZE]) -> Result<Self, JoshBlockError> {
		let magic: [u8; 4] = array(bytes, 0);
		if magic != MAGIC {
			return Err(JoshBlockError::BadMagic { magic });
		}

		let console_signature = ConsoleSignature::parse(&array(bytes, CONSOLE_SIGNATURE_OFFSET));
		let table = &bytes[CACHE_TABLE_OFFSET..];
		let owner = |offset: usize| Some(u32::from_be_bytes(array(table, offset))).filter(|id| *id != 0).map(TitleId);

		Ok(Self {
			console_signature: Box::new(console_signature),
			caches: [CacheEntry::parse(table), CacheEntry::parse(&table[CACHE_ENTRY_SIZE..])],
			version: u32::from_be_bytes(array(table, 0x48)),
			last_assigned: CacheIndex(u32::from_be_bytes(array(table, 0x4c))),
			owners: [owner(0x50), owner(0x54)],
			signed_digest: Sha1::digest(table).into(),
		})
	}

	/// SHA-1 of the cache table at 0x22c..0x284.
	pub const fn signed_digest(&self) -> &[u8; 20] {
		&self.signed_digest
	}

	/// The title's checks of the console signature over the cache table: the certificate with
	/// its authority key and the signature with the certificate's key (see
	/// [`ConsoleSignature::verify`]).
	pub fn verify_console_signature(&self) -> ConsoleSignatureVerification {
		self.console_signature.verify(&self.signed_digest)
	}
}

impl fmt::Debug for JoshBlock {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let certificate = &self.console_signature.certificate;
		f.debug_struct("JoshBlock")
			.field("console_id", &format_args!("{}", certificate.console_id))
			.field("console_part_number", &certificate.console_part_number)
			.field("console_type", &certificate.console_type)
			.field("manufacturing_date", &certificate.manufacturing_date)
			.field("caches", &self.caches)
			.field("version", &self.version)
			.field("last_assigned", &self.last_assigned)
			.field("owners", &self.owners)
			.field("signed_digest", &format_args!("{}", Hex(&self.signed_digest)))
			.finish()
	}
}
