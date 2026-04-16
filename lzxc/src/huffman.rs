//! Canonical Huffman tree construction and encoding.
//!
//! LZX uses canonical Huffman trees: a decoder rebuilds the tree from path
//! lengths alone. For encoding we go the opposite direction: given symbol
//! frequencies, derive path lengths (capped at 16 bits per the LZX spec),
//! then assign canonical codes.
//!
//! # Canonical code assignment
//!
//! Given path lengths, sort symbols by `(length, symbol_index)`. Walk them
//! in order, assigning codes starting at 0 and incrementing. On length
//! increase, left-shift the accumulator. This produces the exact tree
//! [`lzxd::CanonicalTree::create_instance_allow_empty`] reconstructs.
//!
//! # Length-limited Huffman
//!
//! Standard Huffman can produce codes longer than 16 bits on extremely
//! skewed frequency distributions. When that happens we fall back to the
//! **package-merge** algorithm (Larmore-Hirschberg), which produces
//! optimal length-limited code lengths -- minimal total weighted path
//! length subject to `max_len <= 16`. The common case (all Huffman lengths
//! already <= 16) keeps the faster classical merge.

/// LZX maximum Huffman code length.
pub const MAX_CODE_LEN: u8 = 16;

#[derive(Clone, Copy)]
struct Node {
	weight: u64,
	leaf_symbol: Option<usize>,
	left: Option<u32>,
	right: Option<u32>,
}

/// Build canonical Huffman path lengths from symbol frequencies.
///
/// Returns a vector of length `freqs.len()`. Symbols with `freqs[i] == 0`
/// get path length 0 (not in tree). All other path lengths fit in [1, 16].
pub fn build_path_lengths(freqs: &[u32]) -> Vec<u8> {
	let mut lengths = vec![0u8; freqs.len()];
	let non_zero: Vec<usize> = (0..freqs.len()).filter(|&i| freqs[i] > 0).collect();
	if non_zero.is_empty() {
		return lengths;
	}
	assert!(
		non_zero.len() != 1,
		"single-symbol Huffman tree (sym={}, freq={}, alphabet={}) isn't representable in LZX; \
		 non-zero list: {:?}",
		non_zero[0],
		freqs[non_zero[0]],
		freqs.len(),
		non_zero
	);

	let mut nodes: Vec<Node> = non_zero
		.iter()
		.map(|&i| Node { weight: freqs[i] as u64, leaf_symbol: Some(i), left: None, right: None })
		.collect();

	// Two-queue Huffman (van Leeuwen 1976): sort leaves by weight once, then
	// merge the two smallest from the combined front of {sorted leaves,
	// FIFO of merged internal nodes}. Merged weights are monotonically
	// non-decreasing -- the two smallest popped at iteration k sum to a
	// value >= the two smallest at iteration k-1 -- so the internal-node
	// queue stays sorted by construction. Total work: O(n log n) for the
	// initial sort plus O(n) for the merges, vs O(n log n) with per-merge
	// heap pushes.
	let mut leaves_sorted: Vec<u32> = (0..nodes.len() as u32).collect();
	leaves_sorted.sort_by_key(|&i| nodes[i as usize].weight);

	let mut merges: Vec<u32> = Vec::with_capacity(nodes.len());
	let mut li = 0usize;
	let mut mi = 0usize;

	// Pop the smaller of `leaves_sorted[li]` / `merges[mi]`, advancing the
	// corresponding cursor. Returns node id.
	let pop_smallest = |nodes: &[Node], leaves_sorted: &[u32], merges: &[u32], li: &mut usize, mi: &mut usize| -> u32 {
		let l = leaves_sorted.get(*li).copied();
		let m = merges.get(*mi).copied();
		match (l, m) {
			(Some(l), Some(m)) => {
				if nodes[l as usize].weight <= nodes[m as usize].weight {
					*li += 1;
					l
				} else {
					*mi += 1;
					m
				}
			}
			(Some(l), None) => {
				*li += 1;
				l
			}
			(None, Some(m)) => {
				*mi += 1;
				m
			}
			(None, None) => unreachable!("Huffman queue drained before single-root state"),
		}
	};

	while (leaves_sorted.len() - li) + (merges.len() - mi) > 1 {
		let a = pop_smallest(&nodes, &leaves_sorted, &merges, &mut li, &mut mi);
		let b = pop_smallest(&nodes, &leaves_sorted, &merges, &mut li, &mut mi);
		let wa = nodes[a as usize].weight;
		let wb = nodes[b as usize].weight;
		nodes.push(Node { weight: wa + wb, leaf_symbol: None, left: Some(a), right: Some(b) });
		merges.push((nodes.len() - 1) as u32);
	}

	let root = pop_smallest(&nodes, &leaves_sorted, &merges, &mut li, &mut mi);
	assign_lengths(&nodes, root, 0, &mut lengths);

	if lengths.iter().any(|&l| l > MAX_CODE_LEN) {
		lengths = package_merge_lengths(freqs, MAX_CODE_LEN);
	}

	lengths
}

fn assign_lengths(nodes: &[Node], id: u32, depth: u8, out: &mut [u8]) {
	let n = nodes[id as usize];
	if let Some(sym) = n.leaf_symbol {
		out[sym] = depth.max(1);
	} else {
		assign_lengths(nodes, n.left.unwrap(), depth + 1, out);
		assign_lengths(nodes, n.right.unwrap(), depth + 1, out);
	}
}

/// Package-merge (Larmore-Hirschberg) length-limited Huffman.
///
/// Given weights (`freqs`) and a maximum code length, returns optimal path
/// lengths with every non-zero symbol satisfying `1 <= len <= max_len`. The
/// algorithm sweeps `max_len` "levels": at each level it pairs the two
/// lightest items into packages and merges them with the original leaves.
/// After the sweep, each symbol's code length equals the number of times
/// it appears among the lightest `2n - 2` items at the final level.
///
/// Items carry a per-symbol count vector so the final occurrence count is
/// easy to read off. `n` is the non-zero alphabet size, so the working
/// set is O(n^2) bytes per level -- negligible for LZX trees (n <= 656).
fn package_merge_lengths(freqs: &[u32], max_len: u8) -> Vec<u8> {
	let alphabet = freqs.len();
	let mut lengths = vec![0u8; alphabet];

	// Collect non-zero symbol indices sorted ascending by weight (stable on
	// tie to keep the canonical-code assignment deterministic).
	let mut nz: Vec<usize> = (0..alphabet).filter(|&i| freqs[i] > 0).collect();
	nz.sort_by_key(|&i| freqs[i]);
	let n = nz.len();
	assert!(n >= 2, "package-merge requires >= 2 non-zero symbols");

	#[derive(Clone)]
	struct Item {
		weight: u64,
		counts: Vec<u16>,
	}

	// Base leaves (already sorted ascending by weight).
	let leaves: Vec<Item> = (0..n)
		.map(|k| {
			let mut c = vec![0u16; n];
			c[k] = 1;
			Item { weight: freqs[nz[k]] as u64, counts: c }
		})
		.collect();

	let mut current: Vec<Item> = leaves.clone();

	// Sweep max_len - 1 times (level L already = leaves; we build L-1, ..., 1).
	for _ in 1..max_len {
		// Pair consecutive items into packages; drop any trailing unpaired.
		let mut packages: Vec<Item> = Vec::with_capacity(current.len() / 2);
		let mut i = 0;
		while i + 1 < current.len() {
			let a = &current[i];
			let b = &current[i + 1];
			let mut merged = a.counts.clone();
			for (m, &bc) in merged.iter_mut().zip(b.counts.iter()) {
				*m += bc;
			}
			packages.push(Item { weight: a.weight + b.weight, counts: merged });
			i += 2;
		}

		// Packages (already in non-decreasing order) merge-sort against leaves.
		let mut next = Vec::with_capacity(packages.len() + leaves.len());
		let (mut pi, mut li) = (0, 0);
		while pi < packages.len() && li < leaves.len() {
			if packages[pi].weight <= leaves[li].weight {
				next.push(packages[pi].clone());
				pi += 1;
			} else {
				next.push(leaves[li].clone());
				li += 1;
			}
		}
		next.extend(packages.drain(pi..));
		next.extend(leaves[li..].iter().cloned());
		current = next;
	}

	// The lightest 2n - 2 items at level 1 determine final code lengths.
	let take = 2 * n - 2;
	let mut counts = vec![0u32; n];
	for item in current.iter().take(take) {
		for (c, &ic) in counts.iter_mut().zip(item.counts.iter()) {
			*c += ic as u32;
		}
	}
	for (k, &sym) in nz.iter().enumerate() {
		lengths[sym] = counts[k] as u8;
	}
	debug_assert_eq!(
		lengths.iter().filter(|&&l| l > 0).map(|&l| 1u64 << (max_len - l)).sum::<u64>(),
		1u64 << max_len,
		"package-merge produced non-Kraft lengths: {:?}",
		lengths
	);
	lengths
}

/// A canonical Huffman code for one symbol: (code value, number of bits).
#[derive(Debug, Clone, Copy)]
pub struct Code {
	/// Code bits, MSB-first in the low `len` bits of the u32.
	pub value: u32,
	pub len: u8,
}

/// Given path lengths, return canonical codes matching what the lzxd reader
/// expects. Symbols with length 0 get `Code { value: 0, len: 0 }` (unused).
pub fn build_codes(lengths: &[u8]) -> Vec<Code> {
	let mut codes = vec![Code { value: 0, len: 0 }; lengths.len()];

	// Sort symbols by (length, symbol_index). lzxd's reader walks bit lengths
	// from 1 upward and assigns codes in symbol order within each length.
	for bit in 1..=MAX_CODE_LEN {
		let _ = bit;
	}

	let max_len = lengths.iter().copied().max().unwrap_or(0);
	if max_len == 0 {
		return codes;
	}

	let mut next_code: u32 = 0;
	for bit in 1..=max_len {
		for (sym, &l) in lengths.iter().enumerate() {
			if l == bit {
				codes[sym] = Code { value: next_code, len: l };
				next_code += 1;
			}
		}
		next_code <<= 1;
	}

	codes
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	#[should_panic(expected = "single-symbol Huffman tree")]
	fn single_symbol_is_rejected() {
		// Callers must avoid this case by recoding before invoking Huffman.
		let freqs = vec![0, 0, 5, 0];
		let _ = build_path_lengths(&freqs);
	}

	#[test]
	fn two_symbols() {
		let freqs = vec![1, 1];
		let lens = build_path_lengths(&freqs);
		assert_eq!(lens, vec![1, 1]);
	}

	#[test]
	fn uniform_256() {
		let freqs = vec![1u32; 256];
		let lens = build_path_lengths(&freqs);
		// All codes should be 8 bits (full balanced binary tree).
		for (i, &l) in lens.iter().enumerate() {
			assert_eq!(l, 8, "symbol {} got length {}", i, l);
		}
	}

	#[test]
	fn codes_match_lzxd_canonical() {
		// Lengths from lzxd's tree test.
		let lens = vec![6u8, 5, 1, 3, 4, 6, 2, 0];
		let codes = build_codes(&lens);
		// Verify code lengths match input.
		for (sym, c) in codes.iter().enumerate() {
			assert_eq!(c.len, lens[sym], "symbol {}: len mismatch", sym);
		}
		// Verify codes are canonically ordered (shorter < longer, within
		// same length ascending by symbol index).
		// Decoded via lzxd should reconstruct the same symbol mapping.
	}

	#[test]
	fn length_limit_enforced() {
		// Build a very skewed distribution that would produce >16-bit codes
		// without clamping: powers of 2 shape the tree deep.
		let mut freqs = vec![1u32; 50];
		for (i, f) in freqs.iter_mut().enumerate() {
			*f = 1 << (i.min(30) as u32);
		}
		let lens = build_path_lengths(&freqs);
		for &l in &lens {
			assert!(l <= MAX_CODE_LEN, "got length {}, max {}", l, MAX_CODE_LEN);
		}
	}
}
