#![deny(missing_docs)]

//! LZX compression encoder.
//!
//! Produces byte streams that round-trip through the [lzxd] decoder. See the
//! [MS-PATCH LZX-DELTA format spec][spec] for the on-disk layout.
//!
//! [lzxd]: https://docs.rs/lzxd
//! [spec]: https://docs.microsoft.com/en-us/openspecs/exchange_server_protocols/ms-patch/cc78752a-b4af-4eee-88cb-01f4d8a4c2bf
//!
//! # Entry points
//!
//! - [`EncoderWriter`]: a `std::io::Write` sink. Buffers input into 32 KB
//!   slabs, compresses each slab, and forwards `u16 BE size | compressed`
//!   frames to an inner sink. Call [`EncoderWriter::finish`] at the end.
//! - [`Encoder::encode_chunk`]: one chunk at a time, one block per chunk.
//!   Use this when you want to control the framing yourself.
//! - [`Encoder::encode_stream`]: a whole input at once. Blocks may span
//!   chunks, which saves tree headers; an XEX "Normal" data region is
//!   built this way.
//!
//! # Strategy
//!
//! [`Strategy`] controls how a chunk is encoded:
//!
//! - [`Strategy::Optimal`] (default): hash-chain match finder and a
//!   cost-based optimal parser that prices literals, repeated-offset
//!   matches, and new matches with the block's own Huffman code lengths.
//!   Each block is emitted as verbatim or aligned offset, whichever is
//!   smaller. Match history persists across chunks.
//! - [`Strategy::LiteralOnly`]: Huffman-coded literals with no LZ77
//!   matching. Useful as a Huffman-only baseline.
//! - [`Strategy::Uncompressed`]: type-3 blocks only. No compression, fastest.
//!
//! Under `Optimal` and `LiteralOnly`, a block that would be larger than its
//! raw bytes is emitted as uncompressed blocks instead.
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
//!
//! # Example: whole stream
//!
//! ```no_run
//! use lzxc::{Encoder, WindowSize};
//!
//! let input: &[u8] = b"...some PE image...";
//! let mut enc = Encoder::new(WindowSize::KB32);
//! for compressed in enc.encode_stream(input) {
//!     // Each entry decodes with one `lzxd::Lzxd::decompress_next` call
//!     // for the matching 32 KB of input.
//!     # let _ = compressed;
//! }
//! ```

mod bitstream;
mod block;
mod huffman;
mod match_finder;
mod parse;
mod pretree;

use std::borrow::Cow;
use std::ops::Range;

use block::FrameSink;
use block::RepeatOffsets;
use block::Stats;
use block::Token;
use block::Trees;

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

	/// Main-tree alphabet size: 256 literals plus 8 match headers per slot.
	fn main_tree_len(self) -> usize {
		256 + 8 * self.position_slots()
	}
}

/// Uncompressed size of one chunk (32 KB): the largest input to
/// [`Encoder::encode_chunk`], and the unit [`Encoder::encode_stream`] splits
/// its input into.
pub const MAX_CHUNK_SIZE: usize = 32 * 1024;

/// Parses per chunk under chunk-local prices, each pass priced from the
/// previous pass's statistics.
const CHUNK_PASSES: usize = 2;

/// Parses per block under the block's merged prices.
const BLOCK_PASSES: usize = 2;

/// How a chunk gets encoded. Set via [`Encoder::with_strategy`] /
/// [`EncoderWriter::with_strategy`]; the default is [`Strategy::Optimal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
	/// Type-3 blocks only. Fastest, no compression.
	Uncompressed,
	/// Huffman-coded literals and no LZ77 matching. Compresses biased data
	/// (text).
	LiteralOnly,
	/// Hash-chain match finder with a cost-based optimal parser; verbatim
	/// or aligned offset blocks, whichever is smaller.
	Optimal,
}

/// LZX encoder.
///
/// Use this directly when you want to control chunk framing yourself. For a
/// `std::io::Write` sink with built-in `u16 BE size | chunk` framing, use
/// [`EncoderWriter`].
///
/// The encoder is **stateful**: match-finder history, the R0/R1/R2
/// repeat-offset queue, previous-tree path lengths, and the one-shot
/// per-stream preamble all persist across calls so the concatenated output
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
	/// Absolute byte offset of the next chunk in the decompressed stream;
	/// the base for E8 `current_pointer` when preprocessing is enabled.
	input_offset: u64,
	strategy: Strategy,
	/// Previous block's trees, which the next block's trees are coded against.
	trees: Trees,
	/// R0/R1/R2 as the decoder holds them after the last emitted block.
	r: RepeatOffsets,
	match_finder: match_finder::MatchFinder,
	parser: parse::Parser,
	/// Price model of the last emitted block; seeds the next parse.
	prices: Option<parse::Prices>,
}

impl Encoder {
	/// Construct an encoder for the given window size. Default strategy is
	/// [`Strategy::Optimal`]; E8 preprocessing is off.
	pub fn new(window_size: WindowSize) -> Self {
		Self {
			window_size,
			e8_translation_size: None,
			first_chunk: true,
			input_offset: 0,
			strategy: Strategy::Optimal,
			trees: Trees::new(window_size.main_tree_len()),
			r: RepeatOffsets::initial(),
			match_finder: match_finder::MatchFinder::new(window_size.max_match_offset()),
			parser: parse::Parser::default(),
			prices: None,
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

	/// Encode one chunk (at most [`MAX_CHUNK_SIZE`] bytes) as a complete
	/// block. The returned bytes correspond to exactly one
	/// `lzxd::Lzxd::decompress_next` call on the decode side with
	/// `output_len == input.len()`.
	///
	/// # Panics
	///
	/// Panics if `input.len() > MAX_CHUNK_SIZE`.
	pub fn encode_chunk(&mut self, input: &[u8]) -> Vec<u8> {
		assert!(input.len() <= MAX_CHUNK_SIZE);
		let mut frames = self.encode_frames(input, vec![input.len()], false);
		frames.pop().expect("one frame per chunk")
	}

	/// Encode `input` as consecutive [`MAX_CHUNK_SIZE`] chunks, returning
	/// one compressed chunk per input chunk. Each corresponds to one
	/// `lzxd::Lzxd::decompress_next` call with that chunk's length.
	///
	/// Blocks may span chunks, so the chunks must be decoded in order by
	/// one decoder. The last block ends with `input`, so further calls can
	/// continue the stream.
	pub fn encode_stream(&mut self, input: &[u8]) -> Vec<Vec<u8>> {
		let frame_lens = input.chunks(MAX_CHUNK_SIZE).map(<[u8]>::len).collect();
		self.encode_frames(input, frame_lens, true)
	}

	fn encode_frames(&mut self, input: &[u8], frame_lens: Vec<usize>, span_blocks: bool) -> Vec<Vec<u8>> {
		// With no chunk to carry it, the preamble waits for the next call.
		if frame_lens.is_empty() {
			return Vec::new();
		}
		let mut sink = FrameSink::new(frame_lens.iter().copied());
		if self.first_chunk {
			self.first_chunk = false;
			// Per-stream preamble: one flag bit (E8 translation) optionally
			// followed by 32 bits of translation size. The decoder reads it
			// once, at the start of the first chunk.
			let w = sink.writer();
			match self.e8_translation_size {
				Some(size) => {
					w.write_bit(true);
					w.write_bits(size, 32);
				}
				None => w.write_bit(false),
			}
		}

		// E8 preprocessing works on a copy so the caller's buffer is not
		// mutated. The decoder reverses it per chunk.
		let data: Cow<'_, [u8]> = match self.e8_translation_size {
			Some(size) => {
				let mut buf = input.to_vec();
				let mut offset = self.input_offset;
				let mut start = 0;
				for &len in &frame_lens {
					e8_preprocess_in_place(&mut buf[start..start + len], offset, size as i32);
					offset += len as u64;
					start += len;
				}
				Cow::Owned(buf)
			}
			None => Cow::Borrowed(input),
		};

		let start = self.match_finder.append(&data);
		let mut frames: Vec<Range<usize>> = Vec::with_capacity(frame_lens.len());
		let mut end = start;
		for &len in &frame_lens {
			if len > 0 {
				frames.push(end..end + len);
			}
			end += len;
		}

		match self.strategy {
			Strategy::Uncompressed => {
				for frame in frames {
					block::emit_uncompressed(&mut sink, &self.r, &self.match_finder.history()[frame]);
				}
			}
			Strategy::LiteralOnly => {
				for frame in frames {
					let data = self.match_finder.history()[frame].to_vec();
					let tokens: Vec<Token> = data.iter().map(|&b| Token::Literal(b)).collect();
					self.emit(&mut sink, &tokens, &data);
				}
			}
			Strategy::Optimal => self.encode_optimal(&mut sink, &frames, span_blocks),
		}

		self.match_finder.trim();
		self.input_offset += input.len() as u64;
		sink.into_frames()
	}

	/// Parse `frames` (history ranges, one per chunk), group them into
	/// blocks, and emit the blocks.
	fn encode_optimal(&mut self, sink: &mut FrameSink, frames: &[Range<usize>], span_blocks: bool) {
		let main_len = self.window_size.main_tree_len();
		let tables: Vec<match_finder::CandidateTable> = frames
			.iter()
			.map(|frame| {
				let mut table = match_finder::CandidateTable::default();
				self.match_finder.collect(frame.start, frame.end, &mut table);
				table
			})
			.collect();
		let history = self.match_finder.history();

		// First pass: parse each chunk under prices refined from its own
		// statistics, to learn what each chunk looks like.
		let mut prices = self.prices.take().unwrap_or_else(|| parse::Prices::initial(main_len));
		let mut r = self.r;
		let mut tokens = Vec::new();
		let mut frame_stats = Vec::with_capacity(frames.len());
		for (frame, table) in frames.iter().zip(&tables) {
			let mut stats = Stats::new(main_len);
			let mut r_end = r;
			for _ in 0..CHUNK_PASSES {
				tokens.clear();
				r_end = self.parser.parse(history, frame.clone(), table, r, &prices, &mut tokens);
				stats = Stats::new(main_len);
				stats.add_tokens(&tokens, &mut r.clone());
				prices = parse::Prices::from_stats(&stats);
			}
			r = r_end;
			frame_stats.push(stats);
		}

		let groups = if span_blocks {
			group_frames(&frame_stats, &self.trees)
		} else {
			(0..frames.len()).map(|f| f..f + 1).collect()
		};

		// Final pass: reparse each block under its merged prices so the
		// parse and the emitted trees agree.
		for group in groups {
			let mut stats = frame_stats[group.start].clone();
			for s in &frame_stats[group.start + 1..group.end] {
				stats.merge(s);
			}
			for _ in 0..BLOCK_PASSES {
				let prices = parse::Prices::from_stats(&stats);
				tokens.clear();
				let mut r = self.r;
				for f in group.clone() {
					r = self.parser.parse(history, frames[f].clone(), &tables[f], r, &prices, &mut tokens);
				}
				stats = Stats::new(main_len);
				stats.add_tokens(&tokens, &mut self.r.clone());
			}
			let plan = block::plan_block(&stats, &self.trees);
			let group_frames = &frames[group];
			if plan.bits > group_frames.iter().map(|f| block::uncompressed_bits(f.len())).sum() {
				for frame in group_frames {
					block::emit_uncompressed(sink, &self.r, &history[frame.clone()]);
				}
				continue;
			}
			let size = group_frames.iter().map(Range::len).sum();
			block::emit_block(sink, &mut self.trees, &mut self.r, &tokens, size, plan);
			self.prices = Some(parse::Prices::from_stats(&stats));
		}
	}

	/// Emit `tokens` (expanding to the bytes `data`) as one block, or as an
	/// uncompressed block if that is smaller.
	fn emit(&mut self, sink: &mut FrameSink, tokens: &[Token], data: &[u8]) {
		let mut stats = Stats::new(self.window_size.main_tree_len());
		stats.add_tokens(tokens, &mut self.r.clone());
		let plan = block::plan_block(&stats, &self.trees);
		if plan.bits > block::uncompressed_bits(data.len()) {
			block::emit_uncompressed(sink, &self.r, data);
		} else {
			block::emit_block(sink, &mut self.trees, &mut self.r, tokens, data.len(), plan);
		}
	}

	/// Window size this encoder was constructed with.
	pub fn window_size(&self) -> WindowSize {
		self.window_size
	}
}

/// Most chunks one block may span: the block size field is 24 bits.
const MAX_BLOCK_FRAMES: usize = ((1 << 24) - 1) / MAX_CHUNK_SIZE;

/// Group consecutive chunks into blocks. A chunk joins the current block
/// when one block with merged trees is smaller than ending the block and
/// starting a new one.
fn group_frames(frame_stats: &[Stats], trees: &Trees) -> Vec<Range<usize>> {
	let mut groups = Vec::new();
	let Some(first) = frame_stats.first() else { return groups };
	let mut prev = trees.clone();
	let mut start = 0;
	let mut acc = first.clone();
	let mut acc_plan = block::plan_block(&acc, &prev);
	for (f, stats) in frame_stats.iter().enumerate().skip(1) {
		let mut merged = acc.clone();
		merged.merge(stats);
		let merged_plan = block::plan_block(&merged, &prev);
		let split_plan = block::plan_block(stats, &acc_plan.trees());
		if f - start < MAX_BLOCK_FRAMES && merged_plan.bits <= acc_plan.bits + split_plan.bits {
			acc = merged;
			acc_plan = merged_plan;
		} else {
			groups.push(start..f);
			prev = acc_plan.trees();
			start = f;
			acc = stats.clone();
			acc_plan = split_plan;
		}
	}
	groups.push(start..frame_stats.len());
	groups
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
		let Ok(basefile) = std::fs::read("../../xex_files/afplayer.xex") else {
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
	fn single_byte_input_roundtrips() {
		// One literal is a single-symbol main tree, which gets a partner
		// symbol so the tree stays decodable.
		let mut enc = Encoder::new(WindowSize::KB32);
		let mut dec = lzxd::Lzxd::new(lzxd::WindowSize::KB32);
		let bytes = enc.encode_chunk(b"a");
		let out = dec.decompress_next(&bytes, 1).unwrap();
		assert_eq!(out, b"a");
	}

	#[test]
	fn identical_chunks_produce_identical_trees() {
		// Two identical chunks: block 2's trees equal block 1's, so every
		// pretree delta is 0 and the pretree may have a single used symbol.
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
		roundtrip_stream(&input, WindowSize::KB128);
	}

	/// Decode `chunks` (one per `MAX_CHUNK_SIZE` of `input`) with one
	/// decoder and compare against `input`.
	fn decode_chunks(chunks: &[Vec<u8>], input: &[u8], window: WindowSize) {
		assert_eq!(chunks.len(), input.chunks(MAX_CHUNK_SIZE).len());
		let mut dec = lzxd::Lzxd::new(to_lzxd(window));
		for (i, (chunk, expected)) in chunks.iter().zip(input.chunks(MAX_CHUNK_SIZE)).enumerate() {
			let out = dec.decompress_next(chunk, expected.len()).unwrap();
			assert!(out == expected, "chunk {i} mismatch");
		}
	}

	fn roundtrip_stream(input: &[u8], window: WindowSize) {
		let chunks = Encoder::new(window).encode_stream(input);
		decode_chunks(&chunks, input, window);
	}

	/// PowerPC-shaped image: big-endian instruction words drawn from a small
	/// set of opcodes with varying fields, branch offsets that are multiples
	/// of 4, pointer tables, strings, and zero fill.
	fn ppc_like(len: usize, seed: u32) -> Vec<u8> {
		let mut state = seed;
		let mut next = move || {
			state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
			state
		};
		let opcodes = [0x3860_0000u32, 0x7C08_02A6, 0x4E80_0020, 0x9421_FF00, 0x8001_0000, 0x4800_0001, 0x3D60_8200];
		let mut out = Vec::with_capacity(len);
		while out.len() < len {
			match next() % 8 {
				0..=4 => {
					for _ in 0..(next() % 48) {
						let op = opcodes[(next() >> 8) as usize % opcodes.len()];
						let word = if op == 0x4800_0001 { op | ((next() % 0x4000) << 2) } else { op | (next() >> 24) };
						out.extend_from_slice(&word.to_be_bytes());
					}
				}
				5 => {
					for _ in 0..(next() % 16) {
						out.extend_from_slice(&(0x8200_0000u32 + (next() % 0x10000) * 4).to_be_bytes());
					}
				}
				6 => out.extend_from_slice(b"DmGetThreadInfo\0DmSetBreakpoint\0"),
				_ => out.resize(out.len() + (next() % 512) as usize, 0),
			}
		}
		out.truncate(len);
		out
	}

	#[test]
	fn stream_roundtrips_ppc_like_image() {
		let input = ppc_like(300_000, 1);
		roundtrip(&input, WindowSize::KB32);
		roundtrip_stream(&input, WindowSize::KB32);
		roundtrip_stream(&input, WindowSize::KB64);
	}

	#[test]
	fn stream_roundtrips_random_and_repetitive() {
		let mut state = 0x0BAD_F00Du32;
		let random: Vec<u8> = (0..100_000)
			.map(|_| {
				state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
				(state >> 16) as u8
			})
			.collect();
		roundtrip_stream(&random, WindowSize::KB32);
		roundtrip_stream(&vec![0u8; 200_000], WindowSize::KB64);
		roundtrip_stream(b"x", WindowSize::KB32);
		roundtrip_stream(&[], WindowSize::KB32);
		let mut mixed = ppc_like(70_000, 2);
		mixed.extend_from_slice(&random[..50_000]);
		mixed.extend(ppc_like(70_000, 2));
		roundtrip_stream(&mixed, WindowSize::KB64);
	}

	#[test]
	fn stream_continues_after_chunks_and_strategy_changes() {
		// Uncompressed blocks carry R0/R1/R2, and later matches may use the
		// repeated offsets that were current before them.
		let input = ppc_like(6 * MAX_CHUNK_SIZE, 3);
		let window = WindowSize::KB64;
		let mut enc = Encoder::new(window);
		let mut chunks = vec![enc.encode_chunk(&input[..MAX_CHUNK_SIZE])];
		enc.set_strategy(Strategy::Uncompressed);
		chunks.push(enc.encode_chunk(&input[MAX_CHUNK_SIZE..2 * MAX_CHUNK_SIZE]));
		enc.set_strategy(Strategy::LiteralOnly);
		chunks.push(enc.encode_chunk(&input[2 * MAX_CHUNK_SIZE..3 * MAX_CHUNK_SIZE]));
		enc.set_strategy(Strategy::Optimal);
		chunks.extend(enc.encode_stream(&input[3 * MAX_CHUNK_SIZE..]));
		decode_chunks(&chunks, &input, window);
	}

	#[test]
	fn empty_stream_keeps_preamble_for_next_call() {
		let input = ppc_like(50_000, 5);
		let mut enc = Encoder::new(WindowSize::KB64);
		assert!(enc.encode_stream(&[]).is_empty());
		decode_chunks(&enc.encode_stream(&input), &input, WindowSize::KB64);
	}

	#[test]
	fn stream_beats_chunked_output_on_structured_input() {
		let input = ppc_like(400_000, 4);
		let mut enc = Encoder::new(WindowSize::KB64);
		let chunked: usize = input.chunks(MAX_CHUNK_SIZE).map(|c| enc.encode_chunk(c).len()).sum();
		let stream: usize = Encoder::new(WindowSize::KB64).encode_stream(&input).iter().map(Vec::len).sum();
		assert!(stream < chunked, "stream {stream} >= chunked {chunked}");
	}
}
