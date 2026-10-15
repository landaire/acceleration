//! Block encoding: token statistics, tree construction, and emission of
//! verbatim (type 1), aligned offset (type 2), and uncompressed (type 3)
//! blocks.
//!
//! A block may span several chunks (frames). The decoder starts a fresh
//! bitstream at each chunk, so [`FrameSink`] pads the bitstream to a 16-bit
//! boundary whenever the emitted output reaches a chunk end. Matches never
//! cross a chunk end; the parser caps them there.

use crate::bitstream::BitWriter;
use crate::huffman;
use crate::pretree;

/// Position-slot footer bits, from the LZX spec and mirrored from lzxd.
/// Indexed by position slot.
pub const FOOTER_BITS: [u8; 51] = [
	0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15,
	16, 16, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
];

/// Base formatted offset for each position slot.
const BASE_POSITION: [u32; 52] = [
	0, 1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096, 6144,
	8192, 12288, 16384, 24576, 32768, 49152, 65536, 98304, 131072, 196608, 262144, 393216, 524288, 655360, 786432,
	917504, 1048576, 1179648, 1310720, 1441792, 1572864, 1703936, 1835008, 1966080, 2097152, 2228224,
];

pub const MIN_MATCH_LEN: usize = 2;
pub const MAX_MATCH_LEN: usize = 257;
pub const LENGTH_SYM_COUNT: usize = 249;
pub const ALIGNED_SYM_COUNT: usize = 8;

/// Footer bits at or above this count carry their low three bits through
/// the aligned offset tree in an aligned offset block.
pub const ALIGNED_MIN_FOOTER_BITS: u8 = 3;

const BLOCK_TYPE_VERBATIM: u32 = 0b001;
const BLOCK_TYPE_ALIGNED: u32 = 0b010;
const BLOCK_TYPE_UNCOMPRESSED: u32 = 0b011;

/// An emit action: either a literal byte or an LZ77 back-reference.
///
/// `length` is `NonZeroU32` so the enum packs into 8 bytes; the zero niche
/// encodes the literal variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
	Literal(u8),
	Match { offset: u32, length: core::num::NonZeroU32 },
}

impl Token {
	pub fn len(self) -> usize {
		match self {
			Token::Literal(_) => 1,
			Token::Match { length, .. } => length.get() as usize,
		}
	}
}

/// Repeated-offset queue R0/R1/R2, updated exactly as the decoder does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepeatOffsets {
	pub r: [u32; 3],
}

impl RepeatOffsets {
	pub fn initial() -> Self {
		Self { r: [1, 1, 1] }
	}

	/// Index of the first queue entry equal to `offset`. The decoder cannot
	/// tell duplicate entries apart, so the encoder always codes the first.
	pub fn find(&self, offset: u32) -> Option<usize> {
		self.r.iter().position(|&r| r == offset)
	}

	/// Queue state after a match at repeated-offset index `index`.
	pub fn after_repeat(mut self, index: usize) -> Self {
		self.r.swap(0, index);
		self
	}

	/// Queue state after a match at a new offset.
	pub fn after_new(self, offset: u32) -> Self {
		Self { r: [offset, self.r[0], self.r[1]] }
	}
}

/// Position slot of a raw match offset: the slot `s` with
/// `BASE_POSITION[s] <= offset + 2 < BASE_POSITION[s + 1]`.
///
/// Slots 0..=3 equal the formatted offset. Up to `2^18`, slot sizes double
/// every two slots, so the slot is `2 * floor(log2(formatted))` plus the
/// bit below the most significant one. Above that every slot spans 2^17.
pub fn position_slot_for(offset: u32) -> u32 {
	let formatted = offset + 2;
	if formatted < 4 {
		return formatted;
	}
	let l = 31 - formatted.leading_zeros();
	if l < 18 {
		return 2 * l + ((formatted >> (l - 1)) & 1);
	}
	36 + ((formatted - 262_144) >> 17)
}

/// Main-tree symbol for a match header in `slot` with match length `length`.
pub fn main_symbol(slot: u32, length: usize) -> usize {
	256 + slot as usize * 8 + (length - MIN_MATCH_LEN).min(7)
}

/// How one match is coded.
#[derive(Clone, Copy)]
struct MatchCode {
	main_symbol: u16,
	length_symbol: Option<u16>,
	footer_bits: u8,
	/// Formatted offset minus the slot base; meaningful when `footer_bits > 0`.
	footer: u32,
}

/// Code a match and advance `r` the way the decoder will.
fn code_match(offset: u32, length: u32, r: &mut RepeatOffsets) -> MatchCode {
	let (slot, footer_bits, footer) = match r.find(offset) {
		Some(index) => {
			*r = r.after_repeat(index);
			(index as u32, 0, 0)
		}
		None => {
			*r = r.after_new(offset);
			let slot = position_slot_for(offset);
			(slot, FOOTER_BITS[slot as usize], offset + 2 - BASE_POSITION[slot as usize])
		}
	};
	let length_symbol = (length as usize >= MIN_MATCH_LEN + 7).then(|| (length as usize - MIN_MATCH_LEN - 7) as u16);
	MatchCode { main_symbol: main_symbol(slot, length as usize) as u16, length_symbol, footer_bits, footer }
}

/// Symbol frequencies and footer bit totals for a run of tokens.
#[derive(Clone)]
pub struct Stats {
	pub main: Vec<u32>,
	pub length: [u32; LENGTH_SYM_COUNT],
	pub aligned: [u32; ALIGNED_SYM_COUNT],
	/// Footer bits written in a verbatim block.
	pub verbatim_footer_bits: u64,
	/// Footer bits written in an aligned offset block, excluding the
	/// aligned-tree codes counted in `aligned`.
	pub aligned_footer_bits: u64,
}

impl Stats {
	pub fn new(main_len: usize) -> Self {
		Self {
			main: vec![0; main_len],
			length: [0; LENGTH_SYM_COUNT],
			aligned: [0; ALIGNED_SYM_COUNT],
			verbatim_footer_bits: 0,
			aligned_footer_bits: 0,
		}
	}

	/// Count `tokens`, advancing `r` across them.
	pub fn add_tokens(&mut self, tokens: &[Token], r: &mut RepeatOffsets) {
		for &tok in tokens {
			match tok {
				Token::Literal(b) => self.main[b as usize] += 1,
				Token::Match { offset, length } => {
					let code = code_match(offset, length.get(), r);
					self.main[code.main_symbol as usize] += 1;
					if let Some(sym) = code.length_symbol {
						self.length[sym as usize] += 1;
					}
					self.verbatim_footer_bits += code.footer_bits as u64;
					if code.footer_bits >= ALIGNED_MIN_FOOTER_BITS {
						self.aligned_footer_bits += (code.footer_bits - ALIGNED_MIN_FOOTER_BITS) as u64;
						self.aligned[(code.footer & 7) as usize] += 1;
					} else {
						self.aligned_footer_bits += code.footer_bits as u64;
					}
				}
			}
		}
	}

	pub fn merge(&mut self, other: &Stats) {
		for (a, b) in self.main.iter_mut().zip(&other.main) {
			*a += b;
		}
		for (a, b) in self.length.iter_mut().zip(&other.length) {
			*a += b;
		}
		for (a, b) in self.aligned.iter_mut().zip(&other.aligned) {
			*a += b;
		}
		self.verbatim_footer_bits += other.verbatim_footer_bits;
		self.aligned_footer_bits += other.aligned_footer_bits;
	}
}

/// Path lengths of the previous block's trees. The decoder keeps them
/// across blocks and receives each new tree as a delta against them.
#[derive(Clone)]
pub struct Trees {
	main: Vec<u8>,
	length: Vec<u8>,
}

impl Trees {
	pub fn new(main_len: usize) -> Self {
		Self { main: vec![0; main_len], length: vec![0; LENGTH_SYM_COUNT] }
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockKind {
	Verbatim,
	Aligned,
}

/// Trees and block type chosen for a block, with its exact size.
pub struct BlockPlan {
	pub kind: BlockKind,
	main: Vec<u8>,
	length: Vec<u8>,
	aligned: Vec<u8>,
	/// Encoded size of the block in bits, header included.
	pub bits: u64,
}

impl BlockPlan {
	/// The trees this block leaves behind for the next block's deltas.
	pub fn trees(&self) -> Trees {
		Trees { main: self.main.clone(), length: self.length.clone() }
	}
}

fn weighted_bits(freqs: &[u32], lengths: &[u8]) -> u64 {
	freqs.iter().zip(lengths).map(|(&f, &l)| f as u64 * l as u64).sum()
}

fn write_trees(out: &mut BitWriter, prev: &Trees, main: &[u8], length: &[u8]) {
	pretree::encode(out, &prev.main[..256], &main[..256]);
	pretree::encode(out, &prev.main[256..], &main[256..]);
	pretree::encode(out, &prev.length, length);
}

/// Build trees for `stats` and pick the cheaper of a verbatim and an
/// aligned offset block.
pub fn plan_block(stats: &Stats, prev: &Trees) -> BlockPlan {
	let main = huffman::build_path_lengths(&stats.main, huffman::MAX_CODE_LEN);
	let length = huffman::build_path_lengths(&stats.length, huffman::MAX_CODE_LEN);
	// Aligned tree path lengths are 3-bit fields.
	let aligned = huffman::build_path_lengths(&stats.aligned, 7);

	let mut scratch = BitWriter::new();
	write_trees(&mut scratch, prev, &main, &length);
	let shared =
		3 + 24 + scratch.bit_len() as u64 + weighted_bits(&stats.main, &main) + weighted_bits(&stats.length, &length);
	let verbatim_bits = shared + stats.verbatim_footer_bits;
	let aligned_bits =
		shared + 3 * ALIGNED_SYM_COUNT as u64 + stats.aligned_footer_bits + weighted_bits(&stats.aligned, &aligned);

	let (kind, bits) = if stats.aligned.iter().any(|&f| f > 0) && aligned_bits < verbatim_bits {
		(BlockKind::Aligned, aligned_bits)
	} else {
		(BlockKind::Verbatim, verbatim_bits)
	};
	BlockPlan { kind, main, length, aligned, bits }
}

/// Collects the compressed output of consecutive chunks. Each chunk's
/// bitstream is padded to a 16-bit boundary when the uncompressed output
/// reaches the chunk's end.
pub struct FrameSink {
	writer: BitWriter,
	/// Cumulative uncompressed end offset of each chunk.
	frame_ends: Vec<usize>,
	pos: usize,
	frames: Vec<Vec<u8>>,
}

impl FrameSink {
	pub fn new(frame_lens: impl IntoIterator<Item = usize>) -> Self {
		let frame_ends: Vec<usize> = frame_lens
			.into_iter()
			.scan(0, |end, len| {
				*end += len;
				Some(*end)
			})
			.collect();
		Self { writer: BitWriter::new(), frames: Vec::with_capacity(frame_ends.len()), frame_ends, pos: 0 }
	}

	pub fn writer(&mut self) -> &mut BitWriter {
		&mut self.writer
	}

	/// Record `n` more bytes of uncompressed output, closing the current
	/// chunk if it is now complete.
	fn advance(&mut self, n: usize) {
		self.pos += n;
		let end = self.frame_ends[self.frames.len()];
		debug_assert!(self.pos <= end, "token crosses a chunk end");
		if self.pos == end {
			self.frames.push(self.writer.take());
		}
	}

	/// Compressed chunks, one per frame. A zero-length frame yields whatever
	/// preamble was written for it.
	pub fn into_frames(mut self) -> Vec<Vec<u8>> {
		if self.frames.len() < self.frame_ends.len() {
			self.frames.push(self.writer.take());
		}
		debug_assert_eq!(self.frames.len(), self.frame_ends.len());
		self.frames
	}
}

/// Emit one compressed block covering `tokens` (which expand to `size`
/// bytes) using the trees in `plan`, then make them the previous trees.
pub fn emit_block(
	sink: &mut FrameSink,
	trees: &mut Trees,
	r: &mut RepeatOffsets,
	tokens: &[Token],
	size: usize,
	plan: BlockPlan,
) {
	let out = &mut sink.writer;
	match plan.kind {
		BlockKind::Verbatim => {
			out.write_bits(BLOCK_TYPE_VERBATIM, 3);
			out.write_u24_be(size as u32);
		}
		BlockKind::Aligned => {
			out.write_bits(BLOCK_TYPE_ALIGNED, 3);
			out.write_u24_be(size as u32);
			for &l in &plan.aligned {
				out.write_bits(l as u32, 3);
			}
		}
	}
	write_trees(out, trees, &plan.main, &plan.length);
	trees.main = plan.main;
	trees.length = plan.length;

	let main_codes = huffman::build_codes(&trees.main);
	let length_codes = huffman::build_codes(&trees.length);
	let aligned_codes = huffman::build_codes(&plan.aligned);
	let aligned = plan.kind == BlockKind::Aligned;
	for &tok in tokens {
		let out = &mut sink.writer;
		match tok {
			Token::Literal(b) => {
				let c = main_codes[b as usize];
				out.write_bits(c.value, c.len);
			}
			Token::Match { offset, length } => {
				let code = code_match(offset, length.get(), r);
				let c = main_codes[code.main_symbol as usize];
				out.write_bits(c.value, c.len);
				if let Some(sym) = code.length_symbol {
					let c = length_codes[sym as usize];
					out.write_bits(c.value, c.len);
				}
				if aligned && code.footer_bits >= ALIGNED_MIN_FOOTER_BITS {
					out.write_bits(code.footer >> 3, code.footer_bits - ALIGNED_MIN_FOOTER_BITS);
					let c = aligned_codes[(code.footer & 7) as usize];
					out.write_bits(c.value, c.len);
				} else {
					out.write_bits(code.footer, code.footer_bits);
				}
			}
		}
		sink.advance(tok.len());
	}
}

/// Upper bound on the size in bits of `len` bytes as an uncompressed block:
/// header, up to one word of alignment, R0/R1/R2, data, and odd-size pad.
pub fn uncompressed_bits(len: usize) -> u64 {
	(3 + 24 + 16 + 3 * 32 + 8 * len.next_multiple_of(2)) as u64
}

/// Emit `data` as an uncompressed block. The header carries the current
/// R0/R1/R2 so the decoder's queue matches the encoder's afterwards.
pub fn emit_uncompressed(sink: &mut FrameSink, r: &RepeatOffsets, data: &[u8]) {
	let w = &mut sink.writer;
	w.write_bits(BLOCK_TYPE_UNCOMPRESSED, 3);
	w.write_u24_be(data.len() as u32);
	w.align();
	for &v in &r.r {
		w.write_u32_le(v);
	}
	w.write_raw(data);
	// The decoder skips one pad byte after an odd-sized block.
	if !data.len().is_multiple_of(2) {
		w.write_raw(&[0]);
	}
	sink.advance(data.len());
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn position_slots_at_boundaries() {
		// offset=0: formatted=2 -> slot 2 (BASE_POSITION[2]=2).
		assert_eq!(position_slot_for(0), 2);
		// offset=2: formatted=4 -> slot 4.
		assert_eq!(position_slot_for(2), 4);
		// offset=10: formatted=12 -> slot 7 (BASE=12).
		assert_eq!(position_slot_for(10), 7);
	}

	#[test]
	fn position_slot_matches_base_table() {
		for offset in 0..(1u32 << 20) {
			let slot = position_slot_for(offset) as usize;
			let formatted = offset + 2;
			assert!(BASE_POSITION[slot] <= formatted && formatted < BASE_POSITION[slot + 1], "offset {offset}");
		}
	}
}
