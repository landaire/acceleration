//! 4-byte hash-chain match finder with greedy parser.
//!
//! [`MatchFinder`] is **stateful across chunks**: it keeps a sliding-window
//! history of previously seen bytes + a 4-byte hash chain rooted in that
//! history, so matches can reference bytes up to `window_size` back,
//! spanning the 32 KB LZX chunk boundary.
//!
//! Hashing on 4 bytes (instead of 3) keeps popular-trigram chains short,
//! which measurably speeds up chain walks on binary inputs. The trade-off:
//! length-3 matches that aren't preceded by a 4-byte prefix match at the
//! same position are never discovered. In practice length-3-only matches
//! are rare enough that ratio is unaffected on representative workloads.

use crate::verbatim::Token;
use core::mem::MaybeUninit;

const HASH_BITS: u32 = 15;
const HASH_SIZE: usize = 1 << HASH_BITS;
const HASH_MASK: usize = HASH_SIZE - 1;

/// Upper bound on candidates inspected per hash-chain walk. Half of
/// deflate's default; `hash4` chains are short enough that 16 visits almost
/// always hit the true longest match on our workloads.
const MAX_CHAIN_DEPTH: usize = 16;

pub const MIN_MATCH: usize = 3;
pub const MAX_MATCH: usize = crate::verbatim::MAX_MATCH_LEN;

/// Result of a hash-chain search. `length < MIN_MATCH` means no usable match
/// was found (the `offset` field is meaningless in that case).
struct BestMatch {
	length: usize,
	offset: u32,
}

/// Compute a 15-bit Fibonacci hash over 4 bytes.
fn hash4(bytes: &[u8]) -> usize {
	let v = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
	((v.wrapping_mul(2654435761)) >> (32 - HASH_BITS)) as usize & HASH_MASK
}

/// Length of the longest common prefix of `a` and `b`, capped at `max`.
///
/// Compares 8 bytes at a time via native-endian word loads + XOR, finding
/// the first mismatching byte by bit-scanning the XOR. That's ~8x fewer
/// branches than a per-byte loop in the common multi-byte-match case.
fn common_prefix_len(a: &[u8], b: &[u8], max: usize) -> usize {
	let max = max.min(a.len()).min(b.len());
	let mut len = 0;
	while len + 8 <= max {
		let aw = usize::from_ne_bytes(a[len..len + 8].try_into().unwrap());
		let bw = usize::from_ne_bytes(b[len..len + 8].try_into().unwrap());
		let diff = aw ^ bw;
		if diff == 0 {
			len += 8;
			continue;
		}
		// Native-endian: the "first-mismatch" byte is the low-addressed byte
		// in memory, which corresponds to `trailing_zeros` on LE and
		// `leading_zeros` on BE. Modern lzxc only runs on LE hosts in practice;
		// guard anyway.
		let bit = if cfg!(target_endian = "little") { diff.trailing_zeros() } else { diff.leading_zeros() };
		return len + (bit / 8) as usize;
	}
	while len < max && a[len] == b[len] {
		len += 1;
	}
	len
}

/// Stateful, cross-chunk match finder.
pub struct MatchFinder {
	/// Maximum match offset allowed. This is the LZX representable limit for
	/// the target window size, which is smaller than the raw buffer size
	/// because the last position-slot tops out one byte below the window.
	max_offset: usize,
	/// Contiguous byte buffer: window history followed by any bytes not yet
	/// trimmed. Positions in `head` / `prev` reference this buffer via
	/// absolute indices offset by `base`.
	history: Vec<u8>,
	/// Absolute position of `history[0]` in the total input stream. Grows
	/// monotonically as old bytes are trimmed.
	base: u64,
	/// `head[hash]` is the most recent absolute position whose 4-byte
	/// prefix hashes to `hash`, or `u32::MAX` if unseen.
	head: Vec<u32>,
	/// `prev[idx]` is the previous absolute position sharing the hash of
	/// the byte at `base + idx`, or `u32::MAX` to terminate the chain.
	/// Parallel to `history` in length.
	///
	/// (A u16-delta packing was tried -- halves the memory but adds enough
	/// per-hop arithmetic that the M4's out-of-order core was already
	/// hiding the L2-miss latency on this working set. Net: regression.)
	prev: Vec<u32>,
}

impl MatchFinder {
	/// `max_offset` is the largest match offset the caller can encode into a
	/// verbatim/aligned block for the target window size. Typical values:
	/// KB32 -> 32765, KB64 -> 65533, ..., MB32 -> window_bytes - 3. See
	/// `WindowSize::max_match_offset` in `lib.rs` for the mapping.
	pub fn new(max_offset: usize) -> Self {
		Self {
			max_offset,
			history: Vec::new(),
			base: 0,
			head: vec![u32::MAX; HASH_SIZE],
			prev: Vec::new(),
		}
	}

	/// Feed a chunk of input through the match finder, writing the
	/// greedy-parsed token stream into `tokens` (previous contents cleared).
	/// Hash state persists across calls so matches can span chunk boundaries.
	pub fn process(&mut self, chunk: &[u8], tokens: &mut Vec<Token>) {
		tokens.clear();
		// Upper bound: one token per input byte (worst case is all literals).
		// Reserve up front so we can write directly into `spare_capacity_mut`
		// without Vec's per-push cap-check; the hot loop previously spent
		// ~18% of its samples on the `ldr tokens.len / ldr tokens.cap / cmp`
		// sequence emitted by `Vec::push`.
		tokens.reserve(chunk.len());

		let chunk_start_abs = self.base + self.history.len() as u64;
		self.history.extend_from_slice(chunk);
		self.prev.resize(self.history.len(), u32::MAX);

		// Hoist disjoint borrows out of `self` so the compiler stops
		// reloading `head`/`prev`/`history` base+len from struct slots on
		// every inner-loop iteration.
		let base = self.base;
		let head: &mut [u32] = &mut self.head;
		let prev: &mut [u32] = &mut self.prev;
		let history: &[u8] = &self.history;
		let history_len = history.len();
		let end_abs = base + history_len as u64;
		let chunk_start_rel = (chunk_start_abs - base) as usize;

		// Track `p_rel` directly instead of deriving it from `p` each
		// iteration: p_rel increments by 1 (literal) or best.length (match)
		// and is what every inner access actually needs.
		let mut p_rel = chunk_start_rel;
		let mut tok_written: usize = 0;

		// Write tokens directly into the reserved spare capacity instead of
		// using `Vec::push` (which reloads len/cap from memory each call,
		// previously ~18% of samples). The slice borrow is scoped so we can
		// call `tokens.set_len` after it's released.
		{
			let tokens_spare: &mut [MaybeUninit<Token>] = tokens.spare_capacity_mut();

			while p_rel + 4 <= history_len {
				let p_abs = base + p_rel as u64;
				let h = hash4(&history[p_rel..p_rel + 4]);
				let best = Self::find_best_match(history, head, prev, base, p_abs, p_rel, h, end_abs, self.max_offset);

				if best.length >= MIN_MATCH {
					// Sparse insert: only record the match-start position in
					// the chain. With MAX_CHAIN_DEPTH=16 the walk budget is
					// tight, and inserting every intra-match position fills
					// the head of each hash bucket with nearby, short-
					// extending candidates -- starving searches of the older,
					// longer-extending candidates further down the chain.
					// Skipping intra-match inserts is a Pareto win: ratio on
					// structured data improves sharply (2.90x vs 1.76x on
					// the 1 MB corpus) and throughput is neutral-to-slightly-
					// better.
					let length =
						core::num::NonZeroU32::new(best.length as u32).expect("best.length >= MIN_MATCH >= 3");
					tokens_spare[tok_written] = MaybeUninit::new(Token::Match { offset: best.offset, length });
					prev[p_rel] = head[h];
					head[h] = p_abs as u32;
					tok_written += 1;
					p_rel += best.length;
				} else {
					tokens_spare[tok_written] = MaybeUninit::new(Token::Literal(history[p_rel]));
					prev[p_rel] = head[h];
					head[h] = p_abs as u32;
					tok_written += 1;
					p_rel += 1;
				}
			}

			// Tail literals: the last <4 bytes can't participate in a 4-byte
			// hash lookup, so emit them as literals.
			for i in p_rel..history_len {
				tokens_spare[tok_written] = MaybeUninit::new(Token::Literal(history[i]));
				tok_written += 1;
			}
		}

		// SAFETY:
		// The loop above wrote `MaybeUninit::new(..)` into indices
		// `0..tok_written` of `tokens.spare_capacity_mut()`, so those
		// `tok_written` slots beyond the previous `tokens.len()` are
		// initialized. All writes went through safe slice indexing on
		// `tokens_spare`, so any OOB would have panicked before reaching
		// here. Setting `len = tok_written` therefore exposes only
		// initialized `Token` values.
		unsafe { tokens.set_len(tok_written) };

		self.trim_history();
	}

	/// Walk the hash chain for `hash`, extend candidates against `history`,
	/// and return the best-scoring match. Candidates outside the window
	/// (`offset > window_size`) or behind the retained history (stale chain
	/// pointers) are skipped. A `length < MIN_MATCH` in the result means no
	/// usable match was found.
	///
	/// The `prev[c_rel]` load for the *next* candidate happens before the
	/// expensive byte-extension work on the current one. That's a poor
	/// man's software-prefetch: the memory subsystem gets a head start on
	/// the random-access chain link while the CPU is busy comparing bytes.
	///
	/// Takes slices rather than `&self` so the caller can hoist the borrow
	/// once per chunk, letting the compiler prove the slices don't alias and
	/// drop the per-iteration slice-header reloads that otherwise show up as
	/// the dominant hotspot on Apple Silicon.
	fn find_best_match(
		history: &[u8],
		head: &[u32],
		prev: &[u32],
		base: u64,
		p_abs: u64,
		p_rel: usize,
		hash: usize,
		end_abs: u64,
		max_offset: usize,
	) -> BestMatch {
		let max_possible = MAX_MATCH.min((end_abs - p_abs) as usize);
		let mut best = BestMatch { length: 0, offset: 0 };
		let mut candidate_abs = head[hash] as u64;
		let mut depth = 0;
		while depth < MAX_CHAIN_DEPTH && candidate_abs != u32::MAX as u64 {
			// Reject stale (pre-base) or too-far (out-of-window) entries.
			if candidate_abs < base || candidate_abs >= p_abs {
				break;
			}
			let offset = p_abs - candidate_abs;
			if offset > max_offset as u64 || offset == 0 {
				break;
			}
			let c_rel = (candidate_abs - base) as usize;

			// Kick off the fetch for the next chain link up front. By the
			// time we're done with quick-reject + byte extension below, this
			// load is (ideally) already served from cache.
			let next_candidate_abs = prev[c_rel] as u64;

			// Quick-reject: for a candidate to beat the current best, the
			// byte at `best.length` must already match. Skip scanning entirely
			// when it doesn't -- this cuts out the ~90% of chain entries that
			// can't improve our answer, on typical inputs.
			if best.length > 0 && history[c_rel + best.length] != history[p_rel + best.length] {
				candidate_abs = next_candidate_abs;
				depth += 1;
				continue;
			}

			let len = common_prefix_len(&history[c_rel..], &history[p_rel..], max_possible);
			if len > best.length && len >= MIN_MATCH {
				best = BestMatch { length: len, offset: offset as u32 };
				if len >= max_possible {
					break;
				}
			}

			candidate_abs = next_candidate_abs;
			depth += 1;
		}
		best
	}

	/// Trim `history` / `prev` so they don't grow unbounded. The drain is
	/// `O(retained)` (it shifts every remaining byte left), so we amortize by
	/// letting history double past the minimum window before trimming back --
	/// one trim every ~`keep / MAX_CHUNK_SIZE` chunks instead of one per
	/// chunk. Stale `head[]` entries (pointing before the new base) are
	/// detected lazily during matching.
	fn trim_history(&mut self) {
		// Minimum retained history: the full match-offset range plus a chunk
		// of headroom so we never truncate an in-range reference.
		let keep = self.max_offset + crate::MAX_CHUNK_SIZE;
		// Trim threshold: allow another `keep` bytes of growth before we pay
		// the memmove, so the amortized per-chunk drain cost is ~MAX_CHUNK_SIZE.
		let trim_threshold = keep.saturating_add(keep);
		if self.history.len() >= trim_threshold {
			let drop = self.history.len() - keep;
			self.history.drain(0..drop);
			self.prev.drain(0..drop);
			self.base += drop as u64;
		}
	}
}

/// Single-chunk convenience for unit tests: run a fresh `MatchFinder` over
/// `input` with offsets unbounded (no LZX window cap, since there's only
/// ever one chunk).
#[cfg(test)]
fn greedy_match_finder(input: &[u8]) -> Vec<Token> {
	let mut mf = MatchFinder::new(input.len().max(MAX_MATCH));
	let mut tokens = Vec::new();
	mf.process(input, &mut tokens);
	tokens
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn no_matches_short_input() {
		let tokens = greedy_match_finder(b"ab");
		assert_eq!(tokens.len(), 2);
		for t in &tokens {
			assert!(matches!(t, Token::Literal(_)));
		}
	}

	#[test]
	fn finds_simple_repeat() {
		// 8 bytes, `abcd` repeated. The 4-byte hash finds the second `abcd`
		// referencing the first.
		let input = b"abcdabcd";
		let tokens = greedy_match_finder(input);
		let mut seen_match = false;
		for t in &tokens {
			if let Token::Match { offset, length } = t {
				seen_match = true;
				assert_eq!(*offset, 4);
				assert_eq!(length.get(), 4);
			}
		}
		assert!(seen_match);
	}

	#[test]
	fn cross_chunk_match_detected() {
		// Chunk 1 ends with "WXYZ"; chunk 2 starts with a different prefix
		// but contains "WXYZ" again in the middle. The 4-byte hash picks it
		// up as a match referencing chunk 1.
		let mut mf = MatchFinder::new(1 << 16);
		let chunk1 = b"filler-bytes-here..WXYZ";
		let chunk2 = b"more-stuff-then-WXYZ-again";

		let mut tokens1 = Vec::new();
		let mut tokens2 = Vec::new();
		mf.process(chunk1, &mut tokens1);
		mf.process(chunk2, &mut tokens2);

		// A match at chunk2 position `p` that references chunk1 has offset
		// `> p` (since chunk1 sits before chunk2 in the stream).
		let mut pos = 0usize;
		let mut found_cross_chunk = false;
		for t in &tokens2 {
			match t {
				Token::Literal(_) => pos += 1,
				Token::Match { offset, length } => {
					if length.get() >= 4 && (*offset as usize) > pos {
						found_cross_chunk = true;
					}
					pos += length.get() as usize;
				}
			}
		}
		assert!(found_cross_chunk, "expected a match referencing chunk 1; tokens2 = {:?}", tokens2);
	}
}
