#![deny(missing_docs)]

//! LZX compression encoder.
//!
//! Produces byte streams that round-trip through the [lzxd] decoder. See the
//! [MS-PATCH LZX-DELTA format spec][spec] for the on-disk layout.
//!
//! [lzxd]: https://docs.rs/lzxd
//! [spec]: https://docs.microsoft.com/en-us/openspecs/exchange_server_protocols/ms-patch/cc78752a-b4af-4eee-88cb-01f4d8a4c2bf
//!
//! # Two entry points
//!
//! - [`EncoderWriter`]: a `std::io::Write` sink. Buffers input into 32 KB
//!   slabs, compresses each slab, and forwards `u16 BE size | compressed`
//!   frames to an inner sink. Call [`EncoderWriter::finish`] at the end.
//! - [`Encoder`]: the chunk-at-a-time primitive. Use this when you want to
//!   control the framing yourself (an XEX "Normal" data region, for
//!   example, wraps chunks in its own block header).
//!
//! # Strategy
//!
//! [`Strategy`] controls how a chunk is encoded:
//!
//! - [`Strategy::Greedy`] (default): verbatim block + hash-chain match finder
//!   with a greedy parser. Cross-chunk match history is kept in the
//!   [`Encoder`] so matches can span chunk boundaries. On representative
//!   XEX PE payloads this lands around 1.8:1 (~55% of input).
//! - [`Strategy::LiteralOnly`]: verbatim block with no LZ77 matching. Useful
//!   as a Huffman-only baseline.
//! - [`Strategy::Uncompressed`]: type-3 blocks only. No compression, fastest.
//!
//! # Not implemented
//!
//! - Aligned-offset blocks (block type 2). A verbatim block always works in
//!   their place, so output is valid without them.
//!
//! # Example: `Write` sink
//!
//! ```no_run
//! use lzxc::{EncoderWriter, WindowSize};
//! use std::io::Write;
//!
//! let mut out: Vec<u8> = Vec::new();
//! let mut enc = EncoderWriter::new(&mut out, WindowSize::KB64);
//! enc.write_all(b"hello world, hello world, hello world!").unwrap();
//! enc.finish().unwrap();
//! // `out` is now a sequence of `u16 BE size | compressed chunk` frames.
//! ```
//!
//! # Example: chunk-at-a-time
//!
//! ```no_run
//! use lzxc::{Encoder, WindowSize, MAX_CHUNK_SIZE};
//!
//! let input: &[u8] = b"...some PE image...";
//! let mut enc = Encoder::new(WindowSize::KB64);
//! for slab in input.chunks(MAX_CHUNK_SIZE) {
//!     let compressed = enc.encode_chunk(slab);
//!     // `compressed` decodes with `lzxd::Lzxd::decompress_next`.
//!     # let _ = compressed;
//! }
//! ```

mod bitstream;
mod huffman;
mod match_finder;
mod pretree;
mod verbatim;

use bitstream::BitWriter;

/// LZX window sizes. Mirrors the sizes in the [lzxd] decoder crate so
/// callers can pass the same choice to both sides of a round-trip.
///
/// [lzxd]: https://docs.rs/lzxd
///
/// # Example
///
/// ```
/// use lzxc::WindowSize;
///
/// assert_eq!(WindowSize::KB64.bytes(), 0x10000);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)] // variant names are self-explanatory.
pub enum WindowSize {
	KB32,
	KB64,
	KB128,
	KB256,
	KB512,
	MB1,
	MB2,
	MB4,
	MB8,
	MB16,
	MB32,
}

impl WindowSize {
	/// Window size in bytes. Matches the `WINDOW_SIZE` field stored in XEX
	/// `FileFormatInfo` and the value the LZX spec lists for each size.
	pub fn bytes(self) -> u32 {
		match self {
			WindowSize::KB32 => 0x8000,
			WindowSize::KB64 => 0x10000,
			WindowSize::KB128 => 0x20000,
			WindowSize::KB256 => 0x40000,
			WindowSize::KB512 => 0x80000,
			WindowSize::MB1 => 0x100000,
			WindowSize::MB2 => 0x200000,
			WindowSize::MB4 => 0x400000,
			WindowSize::MB8 => 0x800000,
			WindowSize::MB16 => 0x1000000,
			WindowSize::MB32 => 0x2000000,
		}
	}

	/// Number of position slots this window uses (for the main-tree
	/// alphabet: 256 literal symbols + `8 * position_slots` match symbols).
	pub fn position_slots(self) -> usize {
		match self {
			WindowSize::KB32 => 30,
			WindowSize::KB64 => 32,
			WindowSize::KB128 => 34,
			WindowSize::KB256 => 36,
			WindowSize::KB512 => 38,
			WindowSize::MB1 => 42,
			WindowSize::MB2 => 50,
			WindowSize::MB4 => 66,
			WindowSize::MB8 => 98,
			WindowSize::MB16 => 162,
			WindowSize::MB32 => 290,
		}
	}

	/// The largest match offset the encoder will emit for this window size.
	///
	/// The last position slot tops out at `BASE_POSITION[position_slots] - 1`
	/// in formatted-offset space, which (once you subtract the +2 that
	/// formats the offset) means `BASE_POSITION[position_slots] - 3` raw.
	pub fn max_match_offset(self) -> usize {
		// BASE_POSITION recurrence from the spec:
		//   BASE[s] = BASE[s-1] + (1 << FOOTER_BITS[s-1])
		//   FOOTER_BITS[s] = 0 for s < 4, 17 for s >= 36, else (s-2)/2.
		const fn footer_bits(s: usize) -> u32 {
			if s < 4 {
				0
			} else if s >= 36 {
				17
			} else {
				(s as u32 - 2) / 2
			}
		}
		const BASE: [u32; 291] = {
			let mut t = [0u32; 291];
			let mut i = 1usize;
			while i < 291 {
				t[i] = t[i - 1].wrapping_add(1u32 << footer_bits(i - 1));
				i += 1;
			}
			t
		};
		let top = BASE[self.position_slots()];
		(top as usize).saturating_sub(3)
	}
}

/// Maximum input size passed to a single [`Encoder::encode_chunk`] call (32 KB).
pub const MAX_CHUNK_SIZE: usize = 32 * 1024;

/// Block type 3 = uncompressed block (MS-PATCH section 2.6).
const BLOCK_TYPE_UNCOMPRESSED: u32 = 0b011;

/// How a chunk gets encoded. Set via [`Encoder::with_strategy`] /
/// [`EncoderWriter::with_strategy`]; the default is [`Strategy::Greedy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
	/// Type-3 blocks only. Fastest, no compression.
	Uncompressed,
	/// Verbatim blocks with Huffman-coded literals and no LZ77 matching.
	/// Compresses biased data (text); roughly neutral on random data.
	LiteralOnly,
	/// Verbatim blocks with a 3-byte-hash greedy match finder. Best
	/// compression among the currently implemented strategies.
	Greedy,
}

/// LZX encoder: produces one compressed chunk per call.
///
/// Use this directly when you want to control chunk framing yourself. For a
/// `std::io::Write` sink with built-in `u16 BE size | chunk` framing, use
/// [`EncoderWriter`].
///
/// The encoder is **stateful**: match-finder history, the R0/R1/R2
/// repeat-offset queue, previous-tree path lengths, and the one-shot
/// per-stream preamble all persist across chunks so the concatenated output
/// decodes as a single LZX stream.
///
/// # Example
///
/// ```no_run
/// use lzxc::{Encoder, WindowSize, MAX_CHUNK_SIZE};
///
/// let input: &[u8] = b"...";
/// let mut enc = Encoder::new(WindowSize::KB64);
/// for slab in input.chunks(MAX_CHUNK_SIZE) {
///     let compressed = enc.encode_chunk(slab);
///     // compressed decodes with `lzxd::Lzxd::decompress_next`.
///     # let _ = compressed;
/// }
/// ```
pub struct Encoder {
	window_size: WindowSize,
	/// If `Some`, enables E8 preprocessing with the given translation bound
	/// (bytes from the start of the decompressed stream where fixups apply).
	e8_translation_size: Option<u32>,
	first_chunk: bool,
	/// Absolute byte offset of the next chunk in the decompressed stream.
	/// Increments by `input.len()` after each `encode_chunk` call; used as
	/// the base for E8 `current_pointer` when preprocessing is enabled.
	input_offset: u64,
	strategy: Strategy,
	verbatim_state: verbatim::VerbatimState,
	match_finder: match_finder::MatchFinder,
	/// Token scratch buffer reused across chunks. The match finder writes
	/// into it on `Greedy`; `LiteralOnly` repopulates it directly.
	tokens: Vec<verbatim::Token>,
	/// Working buffer for E8 preprocessing. Sized up to a chunk once and
	/// reused.
	e8_scratch: Vec<u8>,
}

impl Encoder {
	/// Construct an encoder for the given window size. Default strategy is
	/// [`Strategy::Greedy`]; E8 preprocessing is off.
	pub fn new(window_size: WindowSize) -> Self {
		Self {
			window_size,
			e8_translation_size: None,
			first_chunk: true,
			input_offset: 0,
			strategy: Strategy::Greedy,
			verbatim_state: verbatim::VerbatimState::new(window_size),
			match_finder: match_finder::MatchFinder::new(window_size.max_match_offset()),
			tokens: Vec::new(),
			e8_scratch: Vec::new(),
		}
	}

	/// Enable E8 preprocessing for x86 CALL targets. `translation_size`
	/// bounds how far into the decompressed stream operand rewrites apply.
	/// Typically set to the full decompressed file size.
	///
	/// Has no effect on PPC payloads (Xbox 360 XEX doesn't use it); leave
	/// off unless you're compressing x86 code.
	pub fn set_e8_translation(&mut self, translation_size: u32) {
		self.e8_translation_size = Some(translation_size);
	}

	/// Builder-style counterpart to [`set_e8_translation`][Self::set_e8_translation].
	pub fn with_e8_translation(mut self, translation_size: u32) -> Self {
		self.set_e8_translation(translation_size);
		self
	}

	/// Select which [`Strategy`] to use for subsequent chunks.
	pub fn set_strategy(&mut self, strategy: Strategy) {
		self.strategy = strategy;
	}

	/// Builder-style counterpart to [`set_strategy`][Self::set_strategy].
	pub fn with_strategy(mut self, strategy: Strategy) -> Self {
		self.set_strategy(strategy);
		self
	}

	/// Encode one chunk (at most [`MAX_CHUNK_SIZE`] bytes). The returned
	/// bytes correspond to exactly one `lzxd::Lzxd::decompress_next` call on
	/// the decode side with `output_len == input.len()`.
	///
	/// # Panics
	///
	/// Panics if `input.len() > MAX_CHUNK_SIZE`.
	pub fn encode_chunk(&mut self, input: &[u8]) -> Vec<u8> {
		assert!(input.len() <= MAX_CHUNK_SIZE);
		let mut w = BitWriter::with_capacity(input.len() + 64);

		if self.first_chunk {
			self.first_chunk = false;
			// Per-stream preamble: one flag bit (E8 translation) optionally
			// followed by 32 bits of translation size. Decoder reads this
			// exactly once on the first chunk.
			match self.e8_translation_size {
				Some(size) => {
					w.write_bit(true);
					w.write_bits(size, 32);
				}
				None => w.write_bit(false),
			}
		}

		// E8 preprocessing runs on a per-chunk copy of `input` so the caller's
		// buffer isn't mutated. Only applied when enabled; the transformation
		// uses `input_offset` as the chunk's base position in the decompressed
		// stream, matching `lzxd`'s postprocess formula.
		let effective_input: &[u8] = if let Some(size) = self.e8_translation_size {
			self.e8_scratch.clear();
			self.e8_scratch.extend_from_slice(input);
			e8_preprocess_in_place(&mut self.e8_scratch, self.input_offset, size as i32);
			&self.e8_scratch
		} else {
			input
		};

		let has_tokens = match self.strategy {
			Strategy::Uncompressed => {
				self.tokens.clear();
				false
			}
			Strategy::LiteralOnly => {
				self.tokens.clear();
				self.tokens.extend(effective_input.iter().map(|&b| verbatim::Token::Literal(b)));
				true
			}
			// Greedy feeds the stateful match finder so matches span chunks.
			Strategy::Greedy => {
				self.match_finder.process(effective_input, &mut self.tokens);
				true
			}
		};

		// A verbatim block requires a non-degenerate main tree: at least two
		// distinct symbols must appear. If the token stream has 0 or 1 unique
		// main-tree symbols, fall back to an uncompressed block for this chunk.
		// The check needs the current R-queue to simulate main-symbol lookup
		// accurately.
		let current_r = self.verbatim_state.r;
		let use_verbatim = has_tokens && verbatim::main_tree_is_nondegenerate(&self.tokens, current_r);

		if use_verbatim {
			verbatim::emit_verbatim_block(
				&mut w,
				&mut self.verbatim_state,
				&self.tokens,
				effective_input.len() as u32,
				self.window_size,
			);
		} else {
			self.emit_uncompressed_block(&mut w, effective_input);
		}

		self.input_offset += input.len() as u64;
		w.finish()
	}

	/// Block type 3: type+size, aligned, R0/R1/R2 (each initialized to 1),
	/// then raw bytes. Mirrors lzxd's `Block::read`.
	fn emit_uncompressed_block(&self, w: &mut BitWriter, data: &[u8]) {
		w.write_bits(BLOCK_TYPE_UNCOMPRESSED, 3);
		w.write_u24_be(data.len() as u32);
		w.align();
		w.write_u32_le(1);
		w.write_u32_le(1);
		w.write_u32_le(1);
		w.write_raw(data);
		// Odd-length blocks pad to even so the reader's re-align consumes the
		// stray byte when `size % 2 != 0`.
		if !data.len().is_multiple_of(2) {
			w.write_raw(&[0]);
		}
	}

	/// Window size this encoder was constructed with.
	pub fn window_size(&self) -> WindowSize {
		self.window_size
	}
}

/// E8 preprocessing: rewrite each 0xE8-prefixed 4-byte operand in the chunk
/// from the compiler-emitted *relative* call target into an *absolute*
/// offset in the decompressed stream. The decoder reverses this on the way
/// out, matching `lzxd`'s `postprocess` formula:
///
/// - `rel_val = if abs_val > 0 { abs_val - current_pointer } else { abs_val + translation_size }`,
///   gated on `-current_pointer <= abs_val < translation_size` and
///   `chunk.len() - pos > 10`.
///
/// The inverse applies here: given `rel_val` at `current_pointer = base +
/// pos`, pick either `abs = rel + current_pointer` (positive branch) or
/// `abs = rel - translation_size` (non-positive branch) -- whichever falls
/// into the decoder's valid range -- and write it back. If neither does,
/// leave the operand alone so the decoder's guard misses and the bytes
/// pass through untouched.
fn e8_preprocess_in_place(buf: &mut [u8], base_offset: u64, translation_size: i32) {
	// Only scan where the decoder's tail-guard (`len - pos > 10`) holds AND
	// `current_pointer < translation_size` (decoder stops early otherwise).
	if buf.len() <= 10 || translation_size <= 0 {
		return;
	}
	let scan_end = buf.len() - 10;
	let mut pos = 0;
	while pos < scan_end {
		let cp64 = base_offset.saturating_add(pos as u64);
		if cp64 >= translation_size as u64 {
			break;
		}
		if buf[pos] != 0xE8 {
			pos += 1;
			continue;
		}
		let cp = cp64 as i32;
		let rel = i32::from_le_bytes([buf[pos + 1], buf[pos + 2], buf[pos + 3], buf[pos + 4]]);
		let abs_pos = rel.wrapping_add(cp);
		let abs_neg = rel.wrapping_sub(translation_size);
		let abs = if abs_pos > 0 && abs_pos < translation_size {
			Some(abs_pos)
		} else if abs_neg >= -cp && abs_neg <= 0 {
			Some(abs_neg)
		} else {
			None
		};
		if let Some(a) = abs {
			buf[pos + 1..pos + 5].copy_from_slice(&a.to_le_bytes());
			pos += 5;
		} else {
			pos += 1;
		}
	}
}

/// `std::io::Write` sink that compresses its input with LZX and forwards
/// `u16 BE size | compressed chunk` frames to an inner writer.
///
/// Input is buffered into 32 KB slabs internally (the LZX chunk size) and
/// each slab is compressed through an [`Encoder`]. The frame format matches
/// what XEX "Normal"-compression data regions expect; for raw LZX streams
/// with custom framing, use [`Encoder`] directly.
///
/// # Always call [`finish`][Self::finish]
///
/// `finish` flushes the trailing partial chunk, surfaces any I/O error from
/// the final write, and returns the inner sink. [`Drop`] falls back to a
/// best-effort flush so a forgotten `finish` doesn't silently lose data,
/// but `Drop` can't return errors, so the write-failure case is invisible
/// if you rely on it.
///
/// # Example
///
/// ```no_run
/// use lzxc::{EncoderWriter, WindowSize};
/// use std::io::Write;
///
/// let mut out: Vec<u8> = Vec::new();
/// let mut enc = EncoderWriter::new(&mut out, WindowSize::KB64);
/// enc.write_all(b"hello, world").unwrap();
/// enc.finish().unwrap();
/// ```
#[must_use = "EncoderWriter is an unterminated compressed stream; call .finish() \
              to surface the final write's errors and recover the inner sink"]
pub struct EncoderWriter<W: std::io::Write> {
	/// `Option` so `finish` can move the inner sink out of `self`, while
	/// still allowing `Drop` to run its best-effort flush when the caller
	/// skipped `finish`.
	inner: Option<W>,
	encoder: Encoder,
	/// Accumulator for the current 32 KB slab. When full, we flush a chunk.
	buf: Vec<u8>,
}

impl<W: std::io::Write> EncoderWriter<W> {
	/// Wrap `inner`, compressing every byte written to this `EncoderWriter`
	/// before forwarding it.
	pub fn new(inner: W, window_size: WindowSize) -> Self {
		Self { inner: Some(inner), encoder: Encoder::new(window_size), buf: Vec::with_capacity(MAX_CHUNK_SIZE) }
	}

	/// Enable E8 preprocessing with the given translation bound. See
	/// [`Encoder::with_e8_translation`] for semantics.
	pub fn with_e8_translation(mut self, translation_size: u32) -> Self {
		self.encoder.set_e8_translation(translation_size);
		self
	}

	/// Select which [`Strategy`] to use for subsequent chunks.
	pub fn with_strategy(mut self, strategy: Strategy) -> Self {
		self.encoder.set_strategy(strategy);
		self
	}

	/// Flush the trailing partial chunk and return the inner sink. Call once
	/// after you've written all your input.
	///
	/// # Errors
	///
	/// Returns any I/O error from the final chunk's write. Errors earlier
	/// in the stream surface through [`Write::write`][std::io::Write::write]
	/// as they happen.
	pub fn finish(mut self) -> std::io::Result<W> {
		self.flush_trailing()?;
		// `inner` is still Some unless an earlier write error poisoned it;
		// Drop's best-effort flush only runs if we leave Some here, which we
		// don't want after a successful finish.
		Ok(self.inner.take().expect("EncoderWriter::finish called twice"))
	}

	fn flush_trailing(&mut self) -> std::io::Result<()> {
		if !self.buf.is_empty() && self.inner.is_some() {
			self.emit_chunk()?;
		}
		Ok(())
	}

	fn emit_chunk(&mut self) -> std::io::Result<()> {
		let chunk = self.encoder.encode_chunk(&self.buf);
		let size = u16::try_from(chunk.len()).map_err(|_| {
			std::io::Error::new(std::io::ErrorKind::InvalidData, "compressed chunk exceeds u16 size prefix")
		})?;
		// `unwrap` is safe: `flush_trailing` guards this path, and `write`
		// asserts on missing-inner before it ever reaches here.
		let inner = self.inner.as_mut().expect("inner sink taken; cannot emit");
		inner.write_all(&size.to_be_bytes())?;
		inner.write_all(&chunk)?;
		self.buf.clear();
		Ok(())
	}
}

impl<W: std::io::Write> std::io::Write for EncoderWriter<W> {
	fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
		assert!(self.inner.is_some(), "EncoderWriter: write after finish");
		let mut written = 0;
		while written < buf.len() {
			let want = MAX_CHUNK_SIZE - self.buf.len();
			let take = want.min(buf.len() - written);
			self.buf.extend_from_slice(&buf[written..written + take]);
			written += take;
			if self.buf.len() == MAX_CHUNK_SIZE {
				self.emit_chunk()?;
			}
		}
		Ok(written)
	}

	/// Flushes any chunks that are already full-sized (and forwards to the
	/// inner sink's flush). Does **not** emit the trailing partial chunk --
	/// a partial chunk can only be emitted once, at the end of the input.
	/// Use [`finish`][Self::finish] for that.
	fn flush(&mut self) -> std::io::Result<()> {
		self.inner.as_mut().expect("EncoderWriter: flush after finish").flush()
	}
}

impl<W: std::io::Write> Drop for EncoderWriter<W> {
	fn drop(&mut self) {
		// Best-effort: don't silently lose the trailing chunk when the caller
		// forgets `finish`. Any error is swallowed (we can't return it from
		// Drop); that's why `finish` is strongly preferred.
		let _ = self.flush_trailing();
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn to_lzxd(w: WindowSize) -> lzxd::WindowSize {
		match w {
			WindowSize::KB32 => lzxd::WindowSize::KB32,
			WindowSize::KB64 => lzxd::WindowSize::KB64,
			WindowSize::KB128 => lzxd::WindowSize::KB128,
			WindowSize::KB256 => lzxd::WindowSize::KB256,
			WindowSize::KB512 => lzxd::WindowSize::KB512,
			WindowSize::MB1 => lzxd::WindowSize::MB1,
			WindowSize::MB2 => lzxd::WindowSize::MB2,
			WindowSize::MB4 => lzxd::WindowSize::MB4,
			WindowSize::MB8 => lzxd::WindowSize::MB8,
			WindowSize::MB16 => lzxd::WindowSize::MB16,
			WindowSize::MB32 => lzxd::WindowSize::MB32,
		}
	}

	fn roundtrip(input: &[u8], window: WindowSize) {
		let mut enc = Encoder::new(window);
		let mut dec = lzxd::Lzxd::new(to_lzxd(window));
		let mut decoded: Vec<u8> = Vec::with_capacity(input.len());
		let mut offset = 0;
		let chunks: Vec<&[u8]> = if input.is_empty() { vec![&[][..]] } else { input.chunks(MAX_CHUNK_SIZE).collect() };
		for chunk in chunks {
			let chunk_bytes = enc.encode_chunk(chunk);
			if chunk.is_empty() {
				continue;
			}
			let out = dec.decompress_next(&chunk_bytes, chunk.len()).unwrap();
			assert_eq!(out, chunk, "chunk at offset {} mismatch", offset);
			decoded.extend_from_slice(out);
			offset += chunk.len();
		}
		assert_eq!(decoded, input);
	}

	#[test]
	fn empty_input() {
		roundtrip(&[], WindowSize::KB32);
	}

	#[test]
	fn short_input() {
		roundtrip(b"hello", WindowSize::KB32);
	}

	#[test]
	fn odd_length_input() {
		// Exercises the odd-length padding path in uncompressed blocks.
		roundtrip(b"hello world!", WindowSize::KB32);
	}

	#[test]
	fn max_chunk_size() {
		let input: Vec<u8> = (0..MAX_CHUNK_SIZE).map(|i| (i & 0xFF) as u8).collect();
		roundtrip(&input, WindowSize::KB32);
	}

	#[test]
	fn multiple_chunks() {
		let input: Vec<u8> = (0..MAX_CHUNK_SIZE * 3 + 100).map(|i| (i & 0xFF) as u8).collect();
		roundtrip(&input, WindowSize::KB64);
	}

	#[test]
	fn random_bytes() {
		let mut state = 0xDEAD_BEEFu32;
		let input: Vec<u8> = (0..5000)
			.map(|_| {
				state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
				(state >> 16) as u8
			})
			.collect();
		roundtrip(&input, WindowSize::KB128);
	}

	#[test]
	fn highly_repetitive_compresses() {
		// 20 KB of one pattern should compress dramatically via matches.
		let mut input = Vec::with_capacity(20_000);
		for _ in 0..(20_000 / 16) {
			input.extend_from_slice(b"abcdefghijklmnop");
		}
		roundtrip(&input, WindowSize::KB32);

		// Now check actual compression size.
		let mut enc = Encoder::new(WindowSize::KB32);
		let compressed: usize = input.chunks(MAX_CHUNK_SIZE).map(|c| enc.encode_chunk(c).len()).sum();
		assert!(
			compressed < input.len() / 4,
			"expected at least 4x compression, got {} -> {}",
			input.len(),
			compressed
		);
	}

	#[test]
	fn text_data_roundtrips() {
		let text = b"Lorem ipsum dolor sit amet, consectetur adipiscing elit. \
		             Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. \
		             Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris \
		             nisi ut aliquip ex ea commodo consequat. \
		             Lorem ipsum dolor sit amet, consectetur adipiscing elit.";
		roundtrip(text, WindowSize::KB32);
	}

	#[test]
	fn literal_only_strategy_roundtrips() {
		let mut enc = Encoder::new(WindowSize::KB32).with_strategy(Strategy::LiteralOnly);
		let mut dec = lzxd::Lzxd::new(lzxd::WindowSize::KB32);
		let input = b"hello literal-only path!";
		let bytes = enc.encode_chunk(input);
		let out = dec.decompress_next(&bytes, input.len()).unwrap();
		assert_eq!(out, input);
	}

	#[test]
	fn uncompressed_strategy_roundtrips() {
		let mut enc = Encoder::new(WindowSize::KB32).with_strategy(Strategy::Uncompressed);
		let mut dec = lzxd::Lzxd::new(lzxd::WindowSize::KB32);
		let input = b"hello uncompressed!";
		let bytes = enc.encode_chunk(input);
		let out = dec.decompress_next(&bytes, input.len()).unwrap();
		assert_eq!(out, input);
	}

	#[test]
	fn two_chunks_random() {
		let mut state = 0x1234_5678u32;
		let input: Vec<u8> = (0..40_000)
			.map(|_| {
				state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
				(state >> 16) as u8
			})
			.collect();
		roundtrip(&input, WindowSize::MB1);
	}

	#[test]
	fn four_chunks_random() {
		let mut state = 0x1234_5678u32;
		let input: Vec<u8> = (0..130_000)
			.map(|_| {
				state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
				(state >> 16) as u8
			})
			.collect();
		roundtrip(&input, WindowSize::MB1);
	}

	#[test]
	fn three_chunks_random() {
		let mut state = 0x1234_5678u32;
		let input: Vec<u8> = (0..70_000)
			.map(|_| {
				state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
				(state >> 16) as u8
			})
			.collect();
		roundtrip(&input, WindowSize::MB1);
	}

	#[test]
	fn large_random_multichunk() {
		let mut state = 0x1234_5678u32;
		let input: Vec<u8> = (0..200_000)
			.map(|_| {
				state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
				(state >> 16) as u8
			})
			.collect();
		roundtrip(&input, WindowSize::MB1);
	}

	#[test]
	fn large_random_literal_only() {
		let mut state = 0x1234_5678u32;
		let input: Vec<u8> = (0..200_000)
			.map(|_| {
				state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
				(state >> 16) as u8
			})
			.collect();
		let mut enc = Encoder::new(WindowSize::MB1).with_strategy(Strategy::LiteralOnly);
		let mut dec = lzxd::Lzxd::new(lzxd::WindowSize::MB1);
		for chunk in input.chunks(MAX_CHUNK_SIZE) {
			let bytes = enc.encode_chunk(chunk);
			let out = dec.decompress_next(&bytes, chunk.len()).unwrap();
			assert_eq!(out, chunk);
		}
	}

	#[test]
	fn real_pe_basefile_roundtrips() {
		// Use a known PE image (afplayer.xex's basefile). If the fixture
		// isn't available the test is skipped; in CI we always have it.
		let Ok(basefile) = std::fs::read("../xex_files/afplayer.xex") else {
			eprintln!("skipping: afplayer.xex not found");
			return;
		};
		// Just compress-round-trip the raw XEX bytes as a stress test.
		let mut enc = Encoder::new(WindowSize::KB64);
		let mut dec = lzxd::Lzxd::new(lzxd::WindowSize::KB64);
		let mut decoded = Vec::with_capacity(basefile.len());
		let mut compressed: usize = 0;
		for chunk in basefile.chunks(MAX_CHUNK_SIZE) {
			let bytes = enc.encode_chunk(chunk);
			compressed += bytes.len();
			let out = dec.decompress_next(&bytes, chunk.len()).unwrap();
			decoded.extend_from_slice(out);
		}
		assert_eq!(decoded, basefile, "round-trip failed on real XEX bytes");
		eprintln!(
			"afplayer.xex: {} -> {} ({:.1}%)",
			basefile.len(),
			compressed,
			100.0 * compressed as f64 / basefile.len() as f64
		);
	}

	#[test]
	fn single_byte_input_falls_back_to_uncompressed() {
		// Single byte -> single main-tree symbol -> must fall back to uncompressed.
		let mut enc = Encoder::new(WindowSize::KB32);
		let mut dec = lzxd::Lzxd::new(lzxd::WindowSize::KB32);
		let bytes = enc.encode_chunk(b"a");
		let out = dec.decompress_next(&bytes, 1).unwrap();
		assert_eq!(out, b"a");
	}

	#[test]
	fn identical_chunks_produce_identical_trees() {
		// Two identical chunks: block 2's Huffman tree == block 1's, so every
		// pretree delta is 0 (symbol 0). Exercises the ensure_multi_symbol
		// path and the case-B fallback (all-symbol-0 ops).
		let input: Vec<u8> = (0..40_000).map(|i| (i & 0xFF) as u8).collect();
		roundtrip(&input, WindowSize::KB64);
	}

	#[test]
	fn e8_preprocessing_round_trips_through_lzxd() {
		// Embed a few 0xE8 CALLs among plain bytes. The encoder rewrites the
		// relative operands to absolute; the decoder reverses it. Round-trip
		// must yield the original bytes.
		let mut input = vec![0u8; 8192];
		for (i, b) in input.iter_mut().enumerate() {
			*b = (i & 0x7F) as u8; // avoid accidental 0xE8 in filler
		}
		// 0xE8 at pos 100 with rel = 0x1000 (a normal-looking forward call).
		input[100] = 0xE8;
		input[101..105].copy_from_slice(&0x1000i32.to_le_bytes());
		// 0xE8 at pos 2000 with rel that would produce an in-range abs.
		input[2000] = 0xE8;
		input[2001..2005].copy_from_slice(&42i32.to_le_bytes());

		let mut enc = Encoder::new(WindowSize::KB32).with_e8_translation(input.len() as u32);
		let bytes = enc.encode_chunk(&input);

		let mut dec = lzxd::Lzxd::new(lzxd::WindowSize::KB32);
		let out = dec.decompress_next(&bytes, input.len()).unwrap();
		assert_eq!(out, input, "E8 preprocessing must round-trip through lzxd");
	}

	#[test]
	fn e8_preprocessing_leaves_non_e8_bytes_alone() {
		let input = vec![0x00u8; 4096];
		let mut buf = input.clone();
		e8_preprocess_in_place(&mut buf, 0, input.len() as i32);
		assert_eq!(buf, input);
	}

	#[test]
	fn encoder_writer_drop_flushes_trailing_chunk() {
		// Writing less than MAX_CHUNK_SIZE bytes and then *dropping* without
		// calling `finish` must still produce a decodable stream (Drop's
		// best-effort flush). Users should still prefer `finish` for error
		// reporting; this test only guards against silent data loss.
		let input = b"just a tiny bit of input, not even one chunk worth".to_vec();
		let mut out: Vec<u8> = Vec::new();
		{
			let mut w = EncoderWriter::new(&mut out, WindowSize::KB32);
			use std::io::Write as _;
			w.write_all(&input).unwrap();
			// No `finish` -- Drop must flush.
		}
		let mut dec = lzxd::Lzxd::new(lzxd::WindowSize::KB32);
		let size = u16::from_be_bytes([out[0], out[1]]) as usize;
		let chunk = &out[2..2 + size];
		let plain = dec.decompress_next(chunk, input.len()).unwrap();
		assert_eq!(plain, input);
	}

	#[test]
	fn encoder_writer_roundtrips_through_u16_prefixed_frames() {
		// Feed 80 KB via `Write`, read back frames, decompress each, verify.
		let input: Vec<u8> = (0..80_000).map(|i| ((i * 7) & 0xFF) as u8).collect();
		let mut out: Vec<u8> = Vec::new();
		{
			let mut w = EncoderWriter::new(&mut out, WindowSize::KB64);
			use std::io::Write as _;
			w.write_all(&input).unwrap();
			w.finish().unwrap();
		}

		let mut dec = lzxd::Lzxd::new(lzxd::WindowSize::KB64);
		let mut decoded = Vec::with_capacity(input.len());
		let mut p = 0;
		let mut remaining = input.len();
		while p < out.len() && remaining > 0 {
			let size = u16::from_be_bytes([out[p], out[p + 1]]) as usize;
			p += 2;
			let chunk = &out[p..p + size];
			p += size;
			let want = remaining.min(MAX_CHUNK_SIZE);
			let plain = dec.decompress_next(chunk, want).unwrap();
			decoded.extend_from_slice(plain);
			remaining -= plain.len();
		}
		assert_eq!(decoded, input);
	}

	#[test]
	fn mixed_pattern_compression() {
		// 128 KB: first half highly repetitive, second half random. Ensures
		// the encoder handles both regimes within one payload.
		let mut input = Vec::with_capacity(128 * 1024);
		for _ in 0..(64 * 1024 / 16) {
			input.extend_from_slice(b"The quick brown ");
		}
		let mut state = 0xFEED_FACEu32;
		for _ in 0..(64 * 1024) {
			state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
			input.push((state >> 16) as u8);
		}
		roundtrip(&input, WindowSize::KB128);
	}
}
