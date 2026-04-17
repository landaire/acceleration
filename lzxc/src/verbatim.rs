//! Verbatim block (type 1) encoding.
//!
//! A verbatim block transmits three Huffman trees via the pretree mechanism:
//! main tree (literals + match headers), length tree (for match lengths
//! >= 9), and no aligned-offset tree. Then the compressed tokens follow.
//!
//! This module handles literal + match encoding but assumes the caller has
//! already performed parsing (i.e. produced a [`Token`] stream).

use crate::WindowSize;
use crate::bitstream::BitWriter;
use crate::huffman;
use crate::pretree;

/// Position-slot footer bits, from LZX spec and mirrored from lzxd. Indexed
/// by position_slot.
const FOOTER_BITS: [u8; 51] = [
	0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15,
	16, 16, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
];

/// Base position for each position_slot.
const BASE_POSITION: [u32; 52] = [
	0, 1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096, 6144,
	8192, 12288, 16384, 24576, 32768, 49152, 65536, 98304, 131072, 196608, 262144, 393216, 524288, 655360, 786432,
	917504, 1048576, 1179648, 1310720, 1441792, 1572864, 1703936, 1835008, 1966080, 2097152, 2228224,
];

pub const MIN_MATCH_LEN: usize = 2;
pub const MAX_MATCH_LEN: usize = 257;
pub const LENGTH_SYM_COUNT: usize = 249;

/// An emit action: either a literal byte or an LZ77 back-reference.
///
/// Match length is a `NonZeroU32` so the compiler niche-optimizes the
/// enum into 8 bytes (vs. the natural 12 if `length` were `u32`). The
/// niche -- the 0 value that `NonZeroU32` forbids -- encodes "this is a
/// Literal", which means no separate discriminant byte and no padding.
/// MIN_MATCH is 2 so no legitimate match has length 0; the restriction
/// is free.
#[derive(Debug, Clone, Copy)]
pub enum Token {
	Literal(u8),
	Match { offset: u32, length: core::num::NonZeroU32 },
}

/// Encoder-side repeated-offset queue. Mirror of the decoder's logic.
#[derive(Debug, Clone, Copy)]
pub struct RepeatOffsets {
	pub r: [u32; 3],
}

impl RepeatOffsets {
	pub fn initial() -> Self {
		Self { r: [1, 1, 1] }
	}
}

/// Compute which position_slot an offset falls into. Matches the decoder's
/// BASE_POSITION table -- the slot is the index `s` such that
/// `BASE_POSITION[s] <= formatted_offset < BASE_POSITION[s+1]`, where
/// `formatted_offset = offset + 2`.
fn position_slot_for(offset: u32) -> u32 {
	let formatted = offset + 2;
	for (s, &base) in BASE_POSITION.iter().enumerate().take(BASE_POSITION.len() - 1) {
		if formatted < BASE_POSITION[s + 1] {
			return s as u32;
		}
		let _ = base;
	}
	(BASE_POSITION.len() - 1) as u32
}

/// Derive (main_symbol, length_symbol, length_footer_bits, extra_length_bits,
/// verbatim_offset_bits) for a match. `length_symbol` is `None` if the match
/// length doesn't need a length-tree symbol (length 2..=8 uses the main tree
/// header bits directly). Returns `(main_symbol, length_tree_symbol,
/// position_slot, verbatim_offset_value, num_verbatim_bits)`.
fn encode_match(offset: u32, length: u32, r: &mut RepeatOffsets) -> MatchEncoding {
	// Repeated-offset check (match against r[0], r[1], r[2]).
	let mut position_slot = None;
	for (i, &rv) in r.r.iter().enumerate() {
		if rv == offset {
			position_slot = Some(i as u32);
			// Update the LRU queue exactly as the decoder does.
			match i {
				0 => {}
				1 => r.r.swap(0, 1),
				2 => r.r.swap(0, 2),
				_ => unreachable!(),
			}
			break;
		}
	}

	let (slot, verbatim_value, num_verbatim_bits) = if let Some(ps) = position_slot {
		(ps, 0u32, 0u8)
	} else {
		let s = position_slot_for(offset);
		let base = BASE_POSITION[s as usize];
		let fb = FOOTER_BITS[s as usize];
		let formatted = offset + 2;
		let verbatim = formatted - base;
		// Update LRU: push new offset to front.
		r.r[2] = r.r[1];
		r.r[1] = r.r[0];
		r.r[0] = offset;
		(s, verbatim, fb)
	};

	// Length encoding: header bits + optional length tree symbol.
	let (length_header, length_tree_symbol) = if length >= 9 {
		// length_tree_symbol = length - 9, emitted via length_tree.
		(7u32, Some((length - 9) as u16))
	} else {
		// length_header holds (length - 2), no length-tree symbol.
		(length - 2, None)
	};

	let main_symbol = 256 + slot * 8 + length_header;

	MatchEncoding { main_symbol: main_symbol as u16, length_tree_symbol, verbatim_value, num_verbatim_bits }
}

struct MatchEncoding {
	main_symbol: u16,
	length_tree_symbol: Option<u16>,
	verbatim_value: u32,
	num_verbatim_bits: u8,
}

/// Per-stream state carried across blocks: previous tree path lengths (for
/// delta encoding), the R0/R1/R2 repeated-offset queue (the decoder persists
/// this across blocks too), and reusable scratch buffers that would
/// otherwise churn through the allocator on every block.
pub struct VerbatimState {
	pub main_prev: Vec<u8>,
	pub length_prev: Vec<u8>,
	pub r: RepeatOffsets,
	/// Per-block scratch. Sized on first use; kept and cleared between
	/// blocks so we pay the allocation cost once.
	encoded: Vec<EncodedToken>,
	main_freq: Vec<u32>,
	length_freq: Vec<u32>,
}

impl VerbatimState {
	pub fn new(window: WindowSize) -> Self {
		let main_len = 256 + 8 * window.position_slots();
		Self {
			main_prev: vec![0u8; main_len],
			length_prev: vec![0u8; LENGTH_SYM_COUNT],
			r: RepeatOffsets::initial(),
			encoded: Vec::new(),
			main_freq: vec![0u32; main_len],
			length_freq: vec![0u32; LENGTH_SYM_COUNT],
		}
	}
}

/// Simulate the encoder's main-symbol computation for `tokens` starting
/// from R-queue state `initial_r`, and report whether at least two distinct
/// main-tree symbols would be emitted. Used to detect the degenerate case
/// where all matches collapse to the same main symbol (e.g. a chunk of
/// same-offset same-length-header matches when R[0] already equals the
/// offset).
pub fn main_tree_is_nondegenerate(tokens: &[Token], initial_r: RepeatOffsets) -> bool {
	let mut seen: Option<u16> = None;
	let mut r = initial_r;
	for &tok in tokens {
		let sym = match tok {
			Token::Literal(b) => b as u16,
			Token::Match { offset, length } => encode_match(offset, length.get(), &mut r).main_symbol,
		};
		match seen {
			None => seen = Some(sym),
			Some(s) if s != sym => return true,
			_ => {}
		}
	}
	false
}

/// Split one length `n >= 9` into a sequence of lengths all in 2..=8 whose
/// sum is `n`. All parts are representable without the length tree.
fn split_length(n: u32) -> Vec<u32> {
	assert!(n >= 9);
	// Greedy: emit as many 8s as fit. If the remainder is 1, borrow from the
	// previous 8 to form 7 + 2 instead.
	let mut out: Vec<u32> = Vec::new();
	let mut remaining = n;
	while remaining >= 8 {
		out.push(8);
		remaining -= 8;
	}
	if remaining == 1 {
		// Replace last 8 with 7 + 2.
		*out.last_mut().unwrap() = 7;
		out.push(2);
	} else if remaining >= 2 {
		out.push(remaining);
	}
	debug_assert_eq!(out.iter().sum::<u32>(), n);
	debug_assert!(out.iter().all(|&l| (2..=8).contains(&l)));
	out
}

/// If the token stream would produce a length tree with exactly one non-zero
/// symbol, recode affected matches into shorter pieces (each length in
/// 2..=8) so the length tree is empty. Multi-symbol length trees pass
/// through unchanged.
///
/// Returns `Cow::Borrowed` (zero-copy, the common case) unless recoding is
/// actually required; the decision early-exits the moment it sees two
/// distinct lengths, so we don't scan the full token stream when we don't
/// have to.
fn avoid_single_symbol_length_tree(tokens: &[Token]) -> std::borrow::Cow<'_, [Token]> {
	let mut first_len: Option<u32> = None;
	for t in tokens {
		if let Token::Match { length, .. } = *t
			&& length.get() >= 9
		{
			match first_len {
				None => first_len = Some(length.get()),
				Some(l) if l != length.get() => return std::borrow::Cow::Borrowed(tokens),
				_ => {}
			}
		}
	}
	// If no matches of length >= 9 exist, the length tree has zero symbols --
	// also fine, pass through unchanged. Only the single-symbol case needs
	// recoding.
	let Some(_only_len) = first_len else {
		return std::borrow::Cow::Borrowed(tokens);
	};

	let mut out = Vec::with_capacity(tokens.len());
	for &tok in tokens {
		match tok {
			Token::Match { offset, length } if length.get() >= 9 => {
				for part in split_length(length.get()) {
					let part_nz = core::num::NonZeroU32::new(part).expect("split_length parts are in 2..=8");
					out.push(Token::Match { offset, length: part_nz });
				}
			}
			t => out.push(t),
		}
	}
	std::borrow::Cow::Owned(out)
}

/// Emit a verbatim block for `tokens`. `block_uncompressed_size` is the total
/// uncompressed byte count this block represents (literals + match lengths).
pub fn emit_verbatim_block(
	out: &mut BitWriter,
	state: &mut VerbatimState,
	tokens: &[Token],
	block_uncompressed_size: u32,
	window: WindowSize,
) {
	// Ensure we never produce a single-symbol length tree. Zero-copy when
	// no recoding is needed (the overwhelmingly common case).
	let tokens = avoid_single_symbol_length_tree(tokens);

	// Reuse the state's scratch buffers instead of allocating fresh Vecs
	// every block. The sizes are fixed by the window, so after the first
	// call these are just zero-fills + push-backed writes.
	let main_len = 256 + 8 * window.position_slots();
	state.main_freq.clear();
	state.main_freq.resize(main_len, 0);
	state.length_freq.clear();
	state.length_freq.resize(LENGTH_SYM_COUNT, 0);
	state.encoded.clear();
	state.encoded.reserve(tokens.len());

	for &tok in tokens.as_ref() {
		match tok {
			Token::Literal(b) => {
				state.main_freq[b as usize] += 1;
				state.encoded.push(EncodedToken::Literal(b));
			}
			Token::Match { offset, length } => {
				let length = length.get();
				debug_assert!((MIN_MATCH_LEN as u32..=MAX_MATCH_LEN as u32).contains(&length));
				let enc = encode_match(offset, length, &mut state.r);
				state.main_freq[enc.main_symbol as usize] += 1;
				if let Some(sym) = enc.length_tree_symbol {
					state.length_freq[sym as usize] += 1;
				}
				state.encoded.push(EncodedToken::Match(enc));
			}
		}
	}

	// Guards: main tree and length tree must have 0 or 2+ non-zero symbols.
	// If these fire, the upstream recoding passes have a bug. Count without
	// allocating -- we just need "is there exactly one non-zero symbol?".
	debug_assert!(
		state.main_freq.iter().filter(|&&f| f > 0).take(2).count() != 1,
		"main tree degenerate after recoding: single non-zero symbol"
	);
	debug_assert!(
		state.length_freq.iter().filter(|&&f| f > 0).take(2).count() != 1,
		"length tree degenerate: single non-zero symbol"
	);

	let main_lengths = huffman::build_path_lengths(&state.main_freq);
	let length_lengths = huffman::build_path_lengths(&state.length_freq);
	let main_codes = huffman::build_codes(&main_lengths);
	let length_codes = huffman::build_codes(&length_lengths);

	// Block header: 3-bit type = 001, 24-bit size.
	out.write_bits(0b001, 3);
	out.write_u24_be(block_uncompressed_size);

	// Transmit trees via pretree: main[0..256], main[256..end], length[0..249].
	pretree::encode(out, &state.main_prev[0..256], &main_lengths[0..256]);
	pretree::encode(out, &state.main_prev[256..], &main_lengths[256..]);
	pretree::encode(out, &state.length_prev[..], &length_lengths[..]);
	state.main_prev.copy_from_slice(&main_lengths);
	state.length_prev.copy_from_slice(&length_lengths);

	// Emit tokens.
	for enc in &state.encoded {
		match enc {
			EncodedToken::Literal(b) => {
				let c = main_codes[*b as usize];
				out.write_bits(c.value, c.len);
			}
			EncodedToken::Match(m) => {
				let c = main_codes[m.main_symbol as usize];
				out.write_bits(c.value, c.len);
				if let Some(sym) = m.length_tree_symbol {
					let lc = length_codes[sym as usize];
					out.write_bits(lc.value, lc.len);
				}
				if m.num_verbatim_bits > 0 {
					out.write_bits(m.verbatim_value, m.num_verbatim_bits);
				}
			}
		}
	}
}

enum EncodedToken {
	Literal(u8),
	Match(MatchEncoding),
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
}
