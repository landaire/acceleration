//! Delta + RLE encoding of Huffman tree path lengths using a "pretree".
//!
//! LZX transmits the path lengths of its main/length/aligned trees through
//! a 20-symbol Huffman tree (the **pretree**) whose own path lengths are
//! sent as 20 fixed 4-bit fields. The decoder reads the pretree, then reads
//! symbols from it to reconstruct the target tree's path lengths.
//!
//! The symbol alphabet is:
//! - `0..=16`: literal encoding of `(17 + prev - new) mod 17`, meaning "new
//!   path length differs from the previous one by this delta".
//! - `17`: run of `4 + read_bits(4)` zeros.
//! - `18`: run of `20 + read_bits(5)` zeros.
//! - `19`: run of `4 + read_bits(1)` of the same value, where the value's
//!   delta is encoded via a subsequent pretree symbol.
//!
//! This module encodes tree path lengths into that form.

use crate::bitstream::BitWriter;
use crate::huffman;

/// Encode `new_lengths` against `prev_lengths` and write to `out`.
///
/// `prev_lengths` represents the previous tree's path lengths (all zeros for
/// the first tree in a stream). After this call, the caller should update
/// its stored `prev_lengths` to `new_lengths`.
pub fn encode(out: &mut BitWriter, prev_lengths: &[u8], new_lengths: &[u8]) {
	assert_eq!(prev_lengths.len(), new_lengths.len(), "prev/new must be equal-length slices");

	let mut symbols = plan_symbols(prev_lengths, new_lengths);
	// The pretree is itself Huffman-coded; LZX forbids single-symbol trees,
	// so force a second symbol into the op stream if planning produced one.
	ensure_multi_symbol(&mut symbols, prev_lengths);

	// Sym-19's follow-up draws from the same pretree alphabet, so it counts
	// toward the histogram too; otherwise a delta that appears only as a
	// follow-up would get code length 0 and emit no bits.
	let mut freqs = [0u32; 20];
	for op in &symbols {
		freqs[op.symbol as usize] += 1;
		if op.symbol == 19 {
			freqs[op.extra2 as usize] += 1;
		}
	}
	let pretree_lengths = huffman::build_path_lengths(&freqs);
	let pretree_codes = huffman::build_codes(&pretree_lengths);

	for &l in pretree_lengths.iter().take(20) {
		out.write_bits(l as u32, 4);
	}
	for _ in pretree_lengths.len()..20 {
		out.write_bits(0, 4);
	}

	for op in &symbols {
		let c = pretree_codes[op.symbol as usize];
		out.write_bits(c.value, c.len);
		match op.symbol {
			17 => out.write_bits(op.extra, 4),
			18 => out.write_bits(op.extra, 5),
			19 => {
				out.write_bits(op.extra, 1);
				let follow = pretree_codes[op.extra2 as usize];
				out.write_bits(follow.value, follow.len);
			}
			_ => {}
		}
	}
}

#[derive(Debug, Clone, Copy)]
struct Op {
	symbol: u8,
	/// Extra bits for symbols 17/18 (zero-run) and 19 (same-run length).
	extra: u32,
	/// Follow-up pretree symbol for op 19.
	extra2: u8,
}

fn plan_symbols(prev: &[u8], new: &[u8]) -> Vec<Op> {
	// One op per position is the worst case; most blocks compress that to
	// 1 op per 3-5 positions via the RLE branches, but reserving full size
	// avoids any intermediate realloc (each pretree::encode call currently
	// churned through several growth steps when driving the main tree).
	let mut ops = Vec::with_capacity(new.len());
	let mut i = 0;
	while i < new.len() {
		if new[i] == 0 {
			// Count consecutive zeros ahead.
			let mut run = 1;
			while i + run < new.len() && new[i + run] == 0 {
				run += 1;
			}
			let mut remaining = run;
			while remaining >= 20 {
				let count = remaining.min(20 + 31);
				let extra = (count - 20) as u32;
				ops.push(Op { symbol: 18, extra, extra2: 0 });
				remaining -= count;
				i += count;
			}
			while remaining >= 4 {
				let count = remaining.min(4 + 15);
				let extra = (count - 4) as u32;
				ops.push(Op { symbol: 17, extra, extra2: 0 });
				remaining -= count;
				i += count;
			}
			// Any leftover single zeros: emit as delta symbols. With new=0
			// and prev=p, the decoder computes `new = (17 + p - code) % 17`,
			// so we need `code = p % 17`.
			for _ in 0..remaining {
				let delta = modsub(prev[i], 0);
				ops.push(Op { symbol: delta, extra: 0, extra2: 0 });
				i += 1;
			}
		} else {
			// Runs of identical non-zero new values compress via sym-19.
			// The decoder derives the broadcast value from prev[cover_start],
			// so the invariant preserved is "new[i+k] == new[i]" (not "delta
			// constant"), and each cover/literal re-derives its delta from
			// the current prev[i].
			let value = new[i];
			let mut run = 1;
			while i + run < new.len() && new[i + run] == value {
				run += 1;
			}
			let cover = cover_same_run(run);
			for _ in 0..cover.fives {
				ops.push(Op { symbol: 19, extra: 1, extra2: modsub(prev[i], value) });
				i += 5;
			}
			for _ in 0..cover.fours {
				ops.push(Op { symbol: 19, extra: 0, extra2: modsub(prev[i], value) });
				i += 4;
			}
			for _ in 0..cover.literals {
				ops.push(Op { symbol: modsub(prev[i], value), extra: 0, extra2: 0 });
				i += 1;
			}
		}
	}
	ops
}

/// How to cover a run of identical path lengths with 4-covers, 5-covers, and
/// literal-delta leftover positions. Fields are counts, not totals (i.e.
/// `fours = 2` means two symbol-19 ops covering 4 positions each).
struct CoverPlan {
	fours: usize,
	fives: usize,
	literals: usize,
}

/// Decompose a run length into `CoverPlan { fours, fives, literals }` that
/// maximizes total cover count and, within that, minimizes literals.
///
/// Symbol 19 covers 4 or 5 positions. For runs >= 12 every length has an exact
/// 4a + 5b decomposition (4 and 5 are coprime, Frobenius number = 11); shorter
/// runs use the hardcoded optimum.
fn cover_same_run(run: usize) -> CoverPlan {
	let (fours, fives, literals) = match run {
		0 => (0, 0, 0),
		1..=3 => (0, 0, run),
		4 => (1, 0, 0),
		5 => (0, 1, 0),
		6 => (0, 1, 1),
		7 => (0, 1, 2),
		8 => (2, 0, 0),
		9 => (1, 1, 0),
		10 => (0, 2, 0),
		11 => (0, 2, 1),
		_ => match run % 4 {
			0 => (run / 4, 0, 0),
			1 => ((run - 5) / 4, 1, 0),
			2 => ((run - 10) / 4, 2, 0),
			3 => ((run - 15) / 4, 3, 0),
			_ => unreachable!(),
		},
	};
	CoverPlan { fours, fives, literals }
}

/// `(17 + prev - new) mod 17` as a u8.
fn modsub(prev: u8, new: u8) -> u8 {
	((17 + prev as i16 - new as i16).rem_euclid(17)) as u8
}

/// Post-process the op stream to guarantee at least two distinct pretree
/// symbols appear. Called only when the naive planner would produce a
/// mono-symbol sequence (e.g. all symbol-18 for a long all-zero range).
///
/// Strategy: find the last RLE op (symbol 17 or 18) covering at least one
/// more position than its minimum, shrink its cover by 1, and append a
/// literal-delta op covering the last position. Since the last position
/// is a zero and prev[last]'s value determines the literal symbol, the
/// inserted op uses `prev[last] % 17` as its symbol -- which differs from
/// 17 and 18 by construction.
fn ensure_multi_symbol(ops: &mut Vec<Op>, prev_lengths: &[u8]) {
	let first = match ops.first() {
		Some(op) => op.symbol,
		None => return,
	};
	if ops.iter().any(|o| o.symbol != first) {
		return;
	}

	// A: RLE op with headroom. Shrink it by one position and append a
	// literal-delta op for the stolen byte. The stolen byte's literal symbol
	// is `prev[pos] % 17`, always in 0..=16 and so distinct from 17/18.
	for idx in (0..ops.len()).rev() {
		let op = ops[idx];
		if (op.symbol == 17 || op.symbol == 18) && op.extra > 0 {
			ops[idx].extra -= 1;
			let stolen = end_position_after_ops(ops, idx);
			let literal_sym = prev_lengths.get(stolen).map_or(0, |&p| p % 17);
			ops.insert(idx + 1, Op { symbol: literal_sym, extra: 0, extra2: 0 });
			return;
		}
	}

	// B: every RLE op is at minimum cover. Unpack the last one into literal
	// deltas. In the (statistically very rare) case those literals all have
	// the same delta too, fall through to C.
	if first == 17 || first == 18 {
		let last_op = *ops.last().unwrap();
		let cover = op_cover(&last_op);
		let end_pos = end_position_after_ops(ops, ops.len() - 1);
		let start_pos = end_pos.saturating_sub(cover);
		ops.pop();
		for p in start_pos..end_pos {
			let literal_sym = prev_lengths.get(p).map_or(0, |&v| v % 17);
			ops.push(Op { symbol: literal_sym, extra: 0, extra2: 0 });
		}
		if ops.iter().any(|o| o.symbol != first) {
			return;
		}
	}

	// C: all ops are the same literal-delta symbol X. Replace the first 4 of
	// them with one sym-19 (same-run) whose follow-up is X. This introduces
	// sym-19 into the histogram while preserving the encoded path lengths.
	if first <= 16 && ops.len() >= 4 {
		let new_op = Op { symbol: 19, extra: 0, extra2: first };
		ops.drain(0..4);
		ops.insert(0, new_op);
	}
}

/// Absolute path-length-array position just past what `ops[0..=idx]` covers.
fn end_position_after_ops(ops: &[Op], idx: usize) -> usize {
	let mut pos = 0usize;
	for (i, op) in ops.iter().enumerate() {
		pos += op_cover(op);
		if i == idx {
			break;
		}
	}
	pos
}

fn op_cover(op: &Op) -> usize {
	match op.symbol {
		17 => 4 + op.extra as usize,
		18 => 20 + op.extra as usize,
		19 => 4 + op.extra as usize,
		_ => 1,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn modsub_basic() {
		assert_eq!(modsub(0, 0), 0);
		assert_eq!(modsub(5, 3), 2);
		assert_eq!(modsub(3, 5), (17 + 3 - 5) as u8 % 17); // 15
		assert_eq!(modsub(0, 8), 9);
	}

	#[test]
	fn encodes_all_zeros_without_panic() {
		let prev = vec![0u8; 256];
		let new = vec![0u8; 256];
		let mut w = BitWriter::new();
		encode(&mut w, &prev, &new);
		// Result should be compact (mostly symbol 18 runs of 20 each, so
		// 256 = 12*20 + 16 -> ~12 eighteen-symbols plus 3 seventeen-symbols
		// plus 4 direct zeros -- all 0 symbols effectively).
		let _ = w.finish();
	}
}
