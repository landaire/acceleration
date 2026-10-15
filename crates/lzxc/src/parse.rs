//! Cost-based optimal parsing.
//!
//! [`Parser::parse`] finds the cheapest token sequence for one chunk by a
//! forward shortest-path search: every position is a node, and literals,
//! repeated-offset matches, and new-offset matches of every usable length
//! are edges priced in bits by a [`Prices`] model. The repeated-offset queue
//! is tracked along the cheapest path to each position, so repeated-offset
//! edges are priced against the queue the decoder will actually hold.

use crate::block::ALIGNED_MIN_FOOTER_BITS;
use crate::block::ALIGNED_SYM_COUNT;
use crate::block::FOOTER_BITS;
use crate::block::LENGTH_SYM_COUNT;
use crate::block::MAX_MATCH_LEN;
use crate::block::MIN_MATCH_LEN;
use crate::block::RepeatOffsets;
use crate::block::Stats;
use crate::block::Token;
use crate::block::main_symbol;
use crate::block::position_slot_for;
use crate::huffman;
use crate::match_finder::CandidateTable;
use crate::match_finder::NICE_LEN;
use crate::match_finder::common_prefix_len;

/// Bit cost of every symbol under a block's prospective trees.
#[derive(Clone)]
pub struct Prices {
	main: Vec<u32>,
	length: [u32; LENGTH_SYM_COUNT],
	aligned: [u32; ALIGNED_SYM_COUNT],
	aligned_block: bool,
}

impl Prices {
	/// Model for the first chunk of a stream, before any statistics exist.
	pub fn initial(main_len: usize) -> Self {
		let mut main = vec![9; main_len];
		main[..256].fill(8);
		Self { main, length: [6; LENGTH_SYM_COUNT], aligned: [3; ALIGNED_SYM_COUNT], aligned_block: true }
	}

	/// Model derived from the statistics of a previous parse. Every count is
	/// incremented by one so unused symbols get a finite, high price.
	pub fn from_stats(stats: &Stats) -> Self {
		fn lengths(freqs: &[u32]) -> Vec<u32> {
			let smoothed: Vec<u32> = freqs.iter().map(|&f| f + 1).collect();
			huffman::build_path_lengths(&smoothed, huffman::MAX_CODE_LEN).into_iter().map(u32::from).collect()
		}
		let aligned_lengths = huffman::build_path_lengths(&stats.aligned, 7);
		let aligned_tree_bits: u64 =
			stats.aligned.iter().zip(&aligned_lengths).map(|(&f, &l)| f as u64 * l as u64).sum();
		let aligned_header_bits = 3 * ALIGNED_SYM_COUNT as u64;
		let aligned_block =
			stats.aligned_footer_bits + aligned_tree_bits + aligned_header_bits < stats.verbatim_footer_bits;
		Self {
			main: lengths(&stats.main),
			length: lengths(&stats.length).try_into().expect("length alphabet size"),
			aligned: lengths(&stats.aligned).try_into().expect("aligned alphabet size"),
			aligned_block,
		}
	}

	fn literal(&self, b: u8) -> u32 {
		self.main[b as usize]
	}

	/// Main-tree symbol plus length-tree symbol for a match in `slot`.
	fn header(&self, slot: u32, length: usize) -> u32 {
		let main = self.main[main_symbol(slot, length)];
		if length >= MIN_MATCH_LEN + 7 { main + self.length[length - MIN_MATCH_LEN - 7] } else { main }
	}

	/// Offset footer cost of a new-offset match.
	fn footer(&self, offset: u32, slot: u32) -> u32 {
		let bits = FOOTER_BITS[slot as usize];
		if self.aligned_block && bits >= ALIGNED_MIN_FOOTER_BITS {
			// Slot bases with three or more footer bits are multiples of 8,
			// so the aligned symbol is the low three formatted-offset bits.
			(bits - ALIGNED_MIN_FOOTER_BITS) as u32 + self.aligned[((offset + 2) & 7) as usize]
		} else {
			bits as u32
		}
	}
}

/// Cheapest known way to reach a position.
#[derive(Clone, Copy)]
struct Arrival {
	cost: u32,
	/// Length of the final token: 1 for a literal, otherwise a match.
	length: u32,
	offset: u32,
	reps: RepeatOffsets,
}

#[derive(Default)]
pub struct Parser {
	arrivals: Vec<Arrival>,
}

impl Parser {
	/// Parse history positions `frame` (one chunk) into `tokens`, given the
	/// candidates `table` collected for that range and the queue `reps` in
	/// effect at its start. Returns the queue after the last token.
	pub fn parse(
		&mut self,
		history: &[u8],
		frame: std::ops::Range<usize>,
		table: &CandidateTable,
		reps: RepeatOffsets,
		prices: &Prices,
		tokens: &mut Vec<Token>,
	) -> RepeatOffsets {
		let start = frame.start;
		let n = frame.len();
		let arrivals = &mut self.arrivals;
		arrivals.clear();
		arrivals.resize(n + 1, Arrival { cost: u32::MAX, length: 0, offset: 0, reps });
		arrivals[0].cost = 0;

		fn relax(arrivals: &mut [Arrival], to: usize, cost: u32, length: usize, offset: u32, reps: RepeatOffsets) {
			if cost < arrivals[to].cost {
				arrivals[to] = Arrival { cost, length: length as u32, offset, reps };
			}
		}

		// Positions inside a match of at least NICE_LEN are only reached by
		// literals; searching them rarely pays off.
		let mut skip_until = 0;
		for i in 0..n {
			let Arrival { cost, reps, .. } = arrivals[i];
			let p = start + i;
			relax(arrivals, i + 1, cost + prices.literal(history[p]), 1, 0, reps);

			let max_len = MAX_MATCH_LEN.min(n - i);
			if i < skip_until || max_len < MIN_MATCH_LEN {
				continue;
			}
			let mut longest = 0;
			for (index, &offset) in reps.r.iter().enumerate() {
				if reps.r[..index].contains(&offset) || offset as usize > p {
					continue;
				}
				let len = common_prefix_len(&history[p - offset as usize..], &history[p..], max_len);
				let next = reps.after_repeat(index);
				for l in MIN_MATCH_LEN..=len {
					relax(arrivals, i + l, cost + prices.header(index as u32, l), l, offset, next);
				}
				longest = longest.max(len);
			}

			let mut covered = MIN_MATCH_LEN - 1;
			for cand in table.at(i) {
				let from = covered + 1;
				covered = cand.length as usize;
				// The repeated-offset edges above already cover this offset.
				if reps.find(cand.offset).is_some() {
					continue;
				}
				let slot = position_slot_for(cand.offset);
				let base = cost + prices.footer(cand.offset, slot);
				let next = reps.after_new(cand.offset);
				for l in from..=covered {
					relax(arrivals, i + l, base + prices.header(slot, l), l, cand.offset, next);
				}
				longest = longest.max(covered);
			}
			if longest >= NICE_LEN {
				skip_until = i + longest;
			}
		}

		let first = tokens.len();
		let mut j = n;
		while j > 0 {
			let a = arrivals[j];
			let length = a.length as usize;
			tokens.push(match core::num::NonZeroU32::new(a.length).filter(|_| length >= MIN_MATCH_LEN) {
				Some(length) => Token::Match { offset: a.offset, length },
				None => Token::Literal(history[start + j - 1]),
			});
			j -= length;
		}
		tokens[first..].reverse();
		arrivals[n].reps
	}
}
