//! Hash-chain match finder that collects every useful match candidate per
//! position for the optimal parser.
//!
//! [`MatchFinder`] is **stateful across calls**: it keeps a sliding-window
//! history of previously seen bytes plus hash chains rooted in that history,
//! so matches can reference bytes up to the LZX window back, spanning chunk
//! boundaries.
//!
//! For each position the finder reports a list of `(length, offset)` pairs
//! with strictly increasing length; each entry carries the smallest offset
//! found for lengths in `(previous.length, length]`. A direct-mapped table
//! keyed on two bytes supplies the nearest length-2 candidate, and a chain
//! keyed on a three-byte hash supplies longer ones.

const HASH_BITS: u32 = 16;
const HASH_SIZE: usize = 1 << HASH_BITS;

/// Candidates inspected per chain walk.
const MAX_CHAIN_DEPTH: usize = 1024;

/// A match at least this long ends the chain walk, and the parser does not
/// search the positions it covers.
pub const NICE_LEN: usize = 128;

pub const MAX_MATCH: usize = crate::block::MAX_MATCH_LEN;

const NONE: u32 = u32::MAX;

/// One candidate match: `length` bytes at distance `offset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
	pub length: u32,
	pub offset: u32,
}

/// Candidates for each position of one frame, stored flat.
#[derive(Default)]
pub struct CandidateTable {
	/// `items[starts[i]..starts[i + 1]]` belong to frame position `i`.
	starts: Vec<u32>,
	items: Vec<Candidate>,
}

impl CandidateTable {
	pub fn at(&self, i: usize) -> &[Candidate] {
		&self.items[self.starts[i] as usize..self.starts[i + 1] as usize]
	}
}

fn hash3(bytes: &[u8]) -> usize {
	let v = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0]);
	(v.wrapping_mul(2654435761) >> (32 - HASH_BITS)) as usize
}

fn hash2(bytes: &[u8]) -> usize {
	u16::from_le_bytes([bytes[0], bytes[1]]) as usize
}

/// Length of the longest common prefix of `a` and `b`, capped at `max`.
///
/// Compares 8 bytes at a time via native-endian word loads + XOR, finding
/// the first mismatching byte by bit-scanning the XOR.
pub fn common_prefix_len(a: &[u8], b: &[u8], max: usize) -> usize {
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
		// The first mismatching byte in memory order is the low byte on LE
		// and the high byte on BE.
		let bit = if cfg!(target_endian = "little") { diff.trailing_zeros() } else { diff.leading_zeros() };
		return len + (bit / 8) as usize;
	}
	while len < max && a[len] == b[len] {
		len += 1;
	}
	len
}

/// Stateful, cross-chunk match finder.
///
/// ## 4 GiB input limit
///
/// Absolute byte positions stored in the hash tables are truncated to
/// `u32`. Once more than `u32::MAX` bytes have been fed through a single
/// `MatchFinder`, stored positions no longer compare correctly against
/// `base`, every candidate is rejected as stale, and the encoder falls back
/// to all-literal output. The stream stays valid; only the ratio degrades.
pub struct MatchFinder {
	/// Largest offset the target window can encode.
	max_offset: usize,
	/// Window history followed by bytes not yet trimmed. Absolute position
	/// `base + i` lives at `history[i]`.
	history: Vec<u8>,
	base: u64,
	/// Most recent absolute position per three-byte hash, or `NONE`.
	head3: Vec<u32>,
	/// Most recent absolute position per two-byte value, or `NONE`.
	head2: Vec<u32>,
	/// `prev[i]` links `base + i` to the previous position with the same
	/// three-byte hash, or `NONE`.
	prev: Vec<u32>,
}

impl MatchFinder {
	pub fn new(max_offset: usize) -> Self {
		Self {
			max_offset,
			history: Vec::new(),
			base: 0,
			head3: vec![NONE; HASH_SIZE],
			head2: vec![NONE; 1 << 16],
			prev: Vec::new(),
		}
	}

	/// Append `data` to the history and return its starting index in
	/// [`history`][Self::history].
	pub fn append(&mut self, data: &[u8]) -> usize {
		let start = self.history.len();
		self.history.extend_from_slice(data);
		self.prev.resize(self.history.len(), NONE);
		debug_assert!(
			self.base + self.history.len() as u64 <= u32::MAX as u64,
			"lzxc MatchFinder: input crossed 4 GiB; construct a fresh Encoder per segment"
		);
		start
	}

	pub fn history(&self) -> &[u8] {
		&self.history
	}

	/// Collect candidates for history positions `start..end` into `out`,
	/// inserting each position into the hash tables. Match lengths are
	/// capped at `end`, so no match crosses it.
	pub fn collect(&mut self, start: usize, end: usize, out: &mut CandidateTable) {
		out.starts.clear();
		out.items.clear();
		out.starts.reserve(end - start + 1);
		let history: &[u8] = &self.history;
		let base = self.base;
		let max_offset = self.max_offset as u64;
		let in_window = |candidate: u32, p_abs: u64| -> Option<usize> {
			if candidate == NONE {
				return None;
			}
			let c = candidate as u64;
			if c < base || c >= p_abs || p_abs - c > max_offset {
				return None;
			}
			Some((c - base) as usize)
		};

		for p in start..end {
			out.starts.push(out.items.len() as u32);
			let p_abs = base + p as u64;
			let max_len = MAX_MATCH.min(end - p);
			let mut best = 1usize;

			if p + 2 <= history.len() {
				let h = hash2(&history[p..]);
				if max_len >= 2
					&& let Some(c) = in_window(self.head2[h], p_abs)
				{
					let len = common_prefix_len(&history[c..], &history[p..], max_len);
					if len >= 2 {
						out.items.push(Candidate { length: len as u32, offset: (p - c) as u32 });
						best = len;
					}
				}
				self.head2[h] = p_abs as u32;
			}

			if p + 3 > history.len() {
				continue;
			}
			let h = hash3(&history[p..]);
			let mut candidate = self.head3[h];
			self.prev[p] = candidate;
			self.head3[h] = p_abs as u32;
			let mut depth = 0;
			while best < max_len.min(NICE_LEN) && depth < MAX_CHAIN_DEPTH {
				let Some(c) = in_window(candidate, p_abs) else { break };
				candidate = self.prev[c];
				depth += 1;
				// A candidate can only beat `best` if it matches at index `best`.
				if history[c + best] != history[p + best] {
					continue;
				}
				let len = common_prefix_len(&history[c..], &history[p..], max_len);
				if len > best {
					out.items.push(Candidate { length: len as u32, offset: (p - c) as u32 });
					best = len;
				}
			}
		}
		out.starts.push(out.items.len() as u32);
	}

	/// Drop history older than the window. The drain shifts every retained
	/// byte, so it only runs once the history has grown to twice the
	/// retained size; stale table entries are rejected lazily by `collect`.
	pub fn trim(&mut self) {
		let keep = self.max_offset + crate::MAX_CHUNK_SIZE;
		if self.history.len() >= keep.saturating_mul(2) {
			let drop = self.history.len() - keep;
			self.history.drain(0..drop);
			self.prev.drain(0..drop);
			self.base += drop as u64;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn candidates(input: &[u8]) -> CandidateTable {
		let mut mf = MatchFinder::new(1 << 16);
		let start = mf.append(input);
		let mut table = CandidateTable::default();
		mf.collect(start, start + input.len(), &mut table);
		table
	}

	#[test]
	fn no_matches_short_input() {
		let table = candidates(b"ab");
		assert!(table.at(0).is_empty());
		assert!(table.at(1).is_empty());
	}

	#[test]
	fn finds_simple_repeat() {
		let table = candidates(b"abcdabcd");
		assert_eq!(table.at(4), &[Candidate { length: 4, offset: 4 }]);
	}

	#[test]
	fn lengths_increase_and_offsets_are_nearest() {
		// At the final "abcX", "ab" occurs 4 back and "abcX" 12 back.
		let table = candidates(b"abcXabYYabZZabcX");
		assert_eq!(table.at(12), &[Candidate { length: 2, offset: 4 }, Candidate { length: 4, offset: 12 }]);
	}

	#[test]
	fn cross_call_match_detected() {
		let mut mf = MatchFinder::new(1 << 16);
		let mut table = CandidateTable::default();
		let first = b"filler-bytes-here..WXYZ";
		let start = mf.append(first);
		mf.collect(start, start + first.len(), &mut table);
		let second = b"more-WXYZ";
		let start = mf.append(second);
		mf.collect(start, start + second.len(), &mut table);
		assert!(table.at(5).iter().any(|c| c.length == 4 && c.offset == 9));
	}

	#[test]
	fn lengths_stop_at_end() {
		let mut mf = MatchFinder::new(1 << 16);
		let input = [7u8; 64];
		let start = mf.append(&input);
		let mut table = CandidateTable::default();
		mf.collect(start, start + 32, &mut table);
		assert_eq!(table.at(10), &[Candidate { length: 22, offset: 1 }]);
	}
}
