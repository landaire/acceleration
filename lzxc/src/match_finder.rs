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
		let chunk_start_abs = self.base + self.history.len() as u64;
		self.history.extend_from_slice(chunk);
		self.prev.resize(self.history.len(), u32::MAX);

		let end_abs = self.base + self.history.len() as u64;
		let mut p = chunk_start_abs;

		while p < end_abs {
			let p_rel = (p - self.base) as usize;

			// 4-byte hash lookahead requirement. The last few positions fall
			// through to literal emission; still a strict superset of the
			// MIN_MATCH=3 constraint on what we can actually emit.
			if end_abs - p < 4 {
				for i in p_rel..self.history.len() {
					tokens.push(Token::Literal(self.history[i]));
				}
				break;
			}

			let h = hash4(&self.history[p_rel..p_rel + 4]);
			let best = self.find_best_match(p, p_rel, h, end_abs);

			if best.length >= MIN_MATCH {
				tokens.push(Token::Match { offset: best.offset, length: best.length as u32 });
				// Insert every position covered by this match into the chain
				// so subsequent searches see all potential match starts. A
				// bounded-insert heuristic (zlib-fast style) was measured to
				// trade ~1.5% speed for ~0.3% compression; that's the wrong
				// side of the tradeoff for XEX output where size is king.
				for i in 0..best.length {
					let abs = p + i as u64;
					let rel = (abs - self.base) as usize;
					if rel + 4 <= self.history.len() {
						let hh = hash4(&self.history[rel..rel + 4]);
						self.prev[rel] = self.head[hh];
						self.head[hh] = abs as u32;
					}
				}
				p += best.length as u64;
			} else {
				tokens.push(Token::Literal(self.history[p_rel]));
				self.prev[p_rel] = self.head[h];
				self.head[h] = p as u32;
				p += 1;
			}
		}

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
	fn find_best_match(&self, p_abs: u64, p_rel: usize, hash: usize, end_abs: u64) -> BestMatch {
		let max_possible = MAX_MATCH.min((end_abs - p_abs) as usize);
		let mut best = BestMatch { length: 0, offset: 0 };
		let mut candidate_abs = self.head[hash] as u64;
		let mut depth = 0;
		while depth < MAX_CHAIN_DEPTH && candidate_abs != u32::MAX as u64 {
			// Reject stale (pre-base) or too-far (out-of-window) entries.
			if candidate_abs < self.base || candidate_abs >= p_abs {
				break;
			}
			let offset = p_abs - candidate_abs;
			if offset > self.max_offset as u64 || offset == 0 {
				break;
			}
			let c_rel = (candidate_abs - self.base) as usize;

			// Kick off the fetch for the next chain link up front. By the
			// time we're done with quick-reject + byte extension below, this
			// load is (ideally) already served from cache.
			let next_candidate_abs = self.prev[c_rel] as u64;

			// Quick-reject: for a candidate to beat the current best, the
			// byte at `best.length` must already match. Skip scanning entirely
			// when it doesn't -- this cuts out the ~90% of chain entries that
			// can't improve our answer, on typical inputs.
			if best.length > 0 && self.history[c_rel + best.length] != self.history[p_rel + best.length] {
				candidate_abs = next_candidate_abs;
				depth += 1;
				continue;
			}

			let len = common_prefix_len(&self.history[c_rel..], &self.history[p_rel..], max_possible);
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
				assert_eq!(*length, 4);
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
					if *length >= 4 && (*offset as usize) > pos {
						found_cross_chunk = true;
					}
					pos += *length as usize;
				}
			}
		}
		assert!(found_cross_chunk, "expected a match referencing chunk 1; tokens2 = {:?}", tokens2);
	}
}
