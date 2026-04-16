//! Bitstream writer mirroring `lzxd::Bitstream`.
//!
//! LZX packs bits into **little-endian 16-bit words**, with bits within each
//! word consumed most-significant-first. The reference specification describes
//! an input bit sequence `a, b, c, ..., x, y, z, A, B, C, D, E, F` producing:
//!
//! ```text
//! [i|j|k|l|m|n|o|p|a|b|c|d|e|f|g|h][y|z|A|B|C|D|E|F|q|r|s|t|u|v|w|x]
//! ```
//!
//! i.e. the top 8 bits of the first 16-bit word come from bits `[i..p]`, the
//! bottom 8 bits from `[a..h]`, and then the word is written LE so its low
//! byte (`[a..h]`) appears first. The writer here reproduces that layout.
//!
//! # Accumulator strategy
//!
//! We buffer up to 64 bits in a `u64` and spill 16-bit words out the top
//! when we run out of room. That lets a single `write_bits` call (<=32 bits)
//! usually stay entirely in-register -- no branching on word-boundary
//! straddles, no per-word `Vec::push`. For an LZX main tree that emits tens
//! of thousands of small codes per block this is the dominant cost center,
//! so the larger accumulator is worth its weight.

use std::io;

/// Write bits into an in-memory buffer, emitting 16-bit LE words as they fill.
pub struct BitWriter {
	buffer: Vec<u8>,
	/// Bits not yet flushed to `buffer`. New bits fill in from the LSB side;
	/// `bits_used` tracks how many are currently live. When `bits_used >= 16`
	/// we spill the highest 16 bits to `buffer` as an LE word.
	cur: u64,
	/// Number of live bits in `cur` (0..=63 at rest; can briefly hit 64
	/// mid-write before the spill loop drains down below 16).
	bits_used: u8,
}

impl BitWriter {
	pub fn new() -> Self {
		Self { buffer: Vec::new(), cur: 0, bits_used: 0 }
	}

	pub fn with_capacity(cap: usize) -> Self {
		Self { buffer: Vec::with_capacity(cap), cur: 0, bits_used: 0 }
	}

	/// Write `n_bits` from `value` (low-order bits). Accepts up to 32 bits.
	#[inline]
	pub fn write_bits(&mut self, value: u32, n_bits: u8) {
		debug_assert!(n_bits <= 32);
		if n_bits == 0 {
			return;
		}
		// `cur` carries bits high-first. Shift the new field into the live
		// region at `(64 - bits_used - n_bits)`; spill full 16-bit words
		// afterwards. With `bits_used <= 48` at entry and `n_bits <= 32`, no
		// shift overflow is possible.
		if self.bits_used + n_bits > 64 {
			// Not expected in normal use: means the caller skipped a spill.
			// Drain 16-bit words until we have headroom, then retry.
			self.drain_words();
			debug_assert!(self.bits_used + n_bits <= 64);
		}
		let shift = 64 - self.bits_used - n_bits;
		let mask = if n_bits == 32 { u32::MAX as u64 } else { (1u64 << n_bits) - 1 };
		self.cur |= (value as u64 & mask) << shift;
		self.bits_used += n_bits;
		if self.bits_used >= 16 {
			self.drain_words();
		}
	}

	/// Spill every complete 16-bit word from the top of `cur` into `buffer`.
	#[inline]
	fn drain_words(&mut self) {
		while self.bits_used >= 16 {
			let word = (self.cur >> 48) as u16;
			let bytes = word.to_le_bytes();
			self.buffer.extend_from_slice(&bytes);
			self.cur <<= 16;
			self.bits_used -= 16;
		}
	}

	/// Single bit, MSB-first into the current word. Slightly nicer at call sites.
	#[inline]
	pub fn write_bit(&mut self, bit: bool) {
		self.write_bits(if bit { 1 } else { 0 }, 1);
	}

	/// Write a 24-bit value in LZX's big-endian block-size framing (top 16
	/// bits, then bottom 8 bits, both through the bitstream).
	#[inline]
	pub fn write_u24_be(&mut self, value: u32) {
		debug_assert!(value < (1 << 24));
		self.write_bits((value >> 8) & 0xFFFF, 16);
		self.write_bits(value & 0xFF, 8);
	}

	/// Write a 32-bit value in little-endian, through the bitstream, matching
	/// `Bitstream::read_u32_le` on the reader side.
	#[inline]
	pub fn write_u32_le(&mut self, value: u32) {
		let lo = value & 0xFFFF;
		let hi = value >> 16;
		// Reader does `lo = read_bits(16).to_le_bytes(); hi = read_bits(16).to_le_bytes();`
		// and rebuilds via `from_le_bytes([lo[0], lo[1], hi[0], hi[1]])`. So
		// writing `lo` then `hi` via `write_bits(..., 16)` produces the
		// expected LE byte order on disk.
		self.write_bits(lo, 16);
		self.write_bits(hi, 16);
	}

	/// Align to the next 16-bit word boundary. Matches `Bitstream::align`:
	/// if `bits_used == 0`, consume one full padding word; otherwise pad the
	/// current partial word with zeros.
	pub fn align(&mut self) {
		if self.bits_used == 0 {
			self.buffer.push(0);
			self.buffer.push(0);
		} else {
			// Pad up to the next 16-bit boundary with zeros (the high-bits
			// of `cur` already carry the written data), then spill.
			let pad = 16 - (self.bits_used % 16);
			self.bits_used += pad;
			self.drain_words();
		}
	}

	/// Append raw bytes. Caller is responsible for having aligned first.
	pub fn write_raw(&mut self, bytes: &[u8]) {
		debug_assert_eq!(self.bits_used, 0, "write_raw requires word-aligned stream");
		self.buffer.extend_from_slice(bytes);
	}

	/// Consume and return the final byte stream. Pads any trailing partial
	/// word with zeros to make 16-bit words whole.
	pub fn finish(mut self) -> Vec<u8> {
		if self.bits_used != 0 {
			// Pad to the next 16-bit boundary with zeros.
			let pad = 16 - (self.bits_used % 16);
			self.bits_used += pad;
			self.drain_words();
		}
		self.buffer
	}
}

impl Default for BitWriter {
	fn default() -> Self {
		Self::new()
	}
}

impl io::Write for BitWriter {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		self.write_raw(buf);
		Ok(buf.len())
	}

	fn flush(&mut self) -> io::Result<()> {
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn bits_pack_into_expected_byte_count() {
		let mut w = BitWriter::new();
		w.write_bits(0b1, 1);
		w.write_bits(0b10, 2);
		w.write_bits(0b111, 3);
		w.write_bits(0b1010, 4);
		w.write_bits(0b1, 1);
		w.write_bits(0xABCD, 16);
		w.write_bits(0x1234, 16);
		let bytes = w.finish();
		// 1 + 2 + 3 + 4 + 1 + 16 + 16 = 43 bits = 2 full 16-bit words + 11 bits;
		// the partial word pads to 48 bits = 6 bytes.
		assert_eq!(bytes.len(), 6);
	}

	#[test]
	fn u32_le_roundtrips() {
		let mut w = BitWriter::new();
		w.write_u32_le(0x12345678);
		let bytes = w.finish();
		// LZX stores u32 as two LE u16 words, each output LE. Result should
		// be the u32 in little-endian byte order: [0x78, 0x56, 0x34, 0x12].
		assert_eq!(bytes, vec![0x78, 0x56, 0x34, 0x12]);
	}

	#[test]
	fn u24_be_roundtrips() {
		let mut w = BitWriter::new();
		w.write_u24_be(0xABCDEF);
		let bytes = w.finish();
		// 24 bits fits in 1.5 words -> 2 words padded.
		assert_eq!(bytes.len(), 4);
	}
}
