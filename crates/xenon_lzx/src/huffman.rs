use crate::error::Error;

const MAX_CODE_LENGTH: u32 = 16;

/// Whether [`DecodeTable::build`] replaced the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Build {
	Replaced,
	/// The code lengths neither form a complete code nor are all zero. The
	/// kernel leaves the previous table in place and keeps decoding with it.
	Kept,
}

/// Canonical prefix code decoding table in the layout of the kernel's table
/// builder (kernel 21256 `0x8015a6e0`): the first `table_bits` bits of a code
/// index `table`; a negative entry names a node whose two children continue
/// the code one bit at a time. Node numbers start at the symbol count.
///
/// The layout matters only when lengths are rejected, because the kernel
/// then decodes with the previous table while taking code lengths from the
/// new lengths.
pub(crate) struct DecodeTable<const ENTRIES: usize, const NODES: usize> {
	table_bits: u32,
	table: [i16; ENTRIES],
	nodes: [[i16; 2]; NODES],
	/// Number of the first node: the symbol count of the build that created
	/// the nodes.
	first_node: usize,
}

impl<const ENTRIES: usize, const NODES: usize> DecodeTable<ENTRIES, NODES> {
	/// An empty table, as in the zeroed memory the kernel allocates: every
	/// code decodes as symbol 0.
	pub(crate) const fn new(table_bits: u32) -> Self {
		Self { table_bits, table: [0; ENTRIES], nodes: [[0; 2]; NODES], first_node: 0 }
	}

	/// Build the table for `lengths`, at most `NODES` symbols.
	pub(crate) fn build(&mut self, lengths: &[u8]) -> Result<Build, Error> {
		let table_bits = self.table_bits;
		let mut counts = [0u32; MAX_CODE_LENGTH as usize + 1];
		for &length in lengths {
			if u32::from(length) > MAX_CODE_LENGTH {
				return Err(Error::CodeLengthTooLong { length });
			}

			counts[usize::from(length)] += 1;
		}

		let mut start = [0u32; MAX_CODE_LENGTH as usize + 2];
		for length in 1..=MAX_CODE_LENGTH as usize {
			start[length + 1] = start[length] + (counts[length] << (MAX_CODE_LENGTH as usize - length));
		}

		match start[MAX_CODE_LENGTH as usize + 1] {
			0x1_0000 => {}
			0 => {
				self.table[..1 << table_bits].fill(0);
				return Ok(Build::Replaced);
			}
			_ => return Ok(Build::Kept),
		}

		let shift = MAX_CODE_LENGTH - table_bits;
		let mut weight = [0u32; MAX_CODE_LENGTH as usize + 1];
		for length in 1..=MAX_CODE_LENGTH as usize {
			if length as u32 <= table_bits {
				start[length] >>= shift;
				weight[length] = 1 << (table_bits - length as u32);
			} else {
				weight[length] = 1 << (MAX_CODE_LENGTH - length as u32);
			}
		}

		let filled = (start[table_bits as usize + 1] >> shift) as usize;
		self.table[filled..1 << table_bits].fill(0);

		self.first_node = lengths.len();
		let mut next_node = lengths.len();
		for (symbol, &length) in lengths.iter().enumerate() {
			if length == 0 {
				continue;
			}

			let length = usize::from(length);
			let code = start[length];
			let next = code + weight[length];
			start[length] = next;
			if length as u32 <= table_bits {
				self.table[code as usize..next as usize].fill(symbol as i16);
				continue;
			}

			// Walk or extend the tree below the table entry, one node per
			// code bit past the table bits.
			let mut slot = Slot::Table((code >> shift) as usize);
			let mut code_bits = code << table_bits;
			for _ in 0..length as u32 - table_bits {
				if *self.slot(slot) == 0 {
					let Some(node) = self.nodes.get_mut(next_node - self.first_node) else {
						return Err(Error::DecodeTableFull);
					};

					*node = [0, 0];
					*self.slot_mut(slot) = -(next_node as i16);
					next_node += 1;
				}

				let node = (-*self.slot(slot)) as usize;
				let branch = usize::from(code_bits & 0x8000 != 0);
				slot = Slot::Node(node, branch);
				code_bits <<= 1;
			}

			*self.slot_mut(slot) = symbol as i16;
		}

		Ok(Build::Replaced)
	}

	fn slot(&self, slot: Slot) -> &i16 {
		match slot {
			Slot::Table(index) => &self.table[index],
			Slot::Node(node, branch) => &self.nodes[node - self.first_node][branch],
		}
	}

	fn slot_mut(&mut self, slot: Slot) -> &mut i16 {
		match slot {
			Slot::Table(index) => &mut self.table[index],
			Slot::Node(node, branch) => &mut self.nodes[node - self.first_node][branch],
		}
	}

	/// Decode the symbol whose code begins the 16 bits `bits`. The caller
	/// takes the code length from its current lengths, as the kernel does.
	pub(crate) fn decode(&self, bits: u32) -> u16 {
		let mut value = self.table[(bits >> (MAX_CODE_LENGTH - self.table_bits)) as usize];
		let mut mask = 1u32 << (MAX_CODE_LENGTH - 1 - self.table_bits);
		while value < 0 {
			let branch = usize::from(bits & mask != 0);
			value = self.nodes[(-value) as usize - self.first_node][branch];
			mask >>= 1;
		}

		value as u16
	}
}

#[derive(Debug, Clone, Copy)]
enum Slot {
	Table(usize),
	Node(usize, usize),
}

/// The aligned offset table: 7 table bits, no tree (kernel 21256
/// `0x8015a9a0`). The kernel ignores its result, so a rejected code still
/// changes the table: totals are taken modulo 0x10000, and the table is
/// cleared and filled until a code no longer fits.
pub(crate) struct AlignedTable {
	table: [u8; 1 << Self::TABLE_BITS],
}

impl AlignedTable {
	const TABLE_BITS: u32 = 7;

	pub(crate) const fn new() -> Self {
		Self { table: [0; 1 << Self::TABLE_BITS] }
	}

	pub(crate) fn build(&mut self, lengths: &[u8; 8]) {
		let mut counts = [0u16; MAX_CODE_LENGTH as usize + 1];
		for &length in lengths {
			counts[usize::from(length)] = counts[usize::from(length)].wrapping_add(1);
		}

		let mut start = [0u16; MAX_CODE_LENGTH as usize + 2];
		for length in 1..=MAX_CODE_LENGTH as usize {
			start[length + 1] =
				start[length].wrapping_add(counts[length].wrapping_shl(MAX_CODE_LENGTH - length as u32));
		}

		if start[MAX_CODE_LENGTH as usize + 1] != 0 {
			return;
		}

		let mut weight = [0u16; Self::TABLE_BITS as usize + 1];
		for length in 1..=Self::TABLE_BITS as usize {
			start[length] >>= MAX_CODE_LENGTH - Self::TABLE_BITS;
			weight[length] = 1 << (Self::TABLE_BITS - length as u32);
		}

		self.table.fill(0);
		for (symbol, &length) in lengths.iter().enumerate() {
			if length == 0 {
				continue;
			}

			let length = usize::from(length);
			let next = start[length] + weight[length];
			if usize::from(next) > self.table.len() {
				return;
			}

			self.table[usize::from(start[length])..usize::from(next)].fill(symbol as u8);
			start[length] = next;
		}
	}

	pub(crate) fn decode(&self, bits: u32) -> u8 {
		self.table[(bits >> (MAX_CODE_LENGTH - Self::TABLE_BITS)) as usize]
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	type Small = DecodeTable<4, 8>;

	#[test]
	fn decodes_canonical_codes_in_length_then_symbol_order() {
		// Symbol 2 has the only 1-bit code (0); symbols 0 and 1 share the
		// 2-bit codes 10 and 11.
		let mut code = Small::new(2);
		assert_eq!(code.build(&[2, 2, 1]).unwrap(), Build::Replaced);

		assert_eq!(code.decode(0b0 << 15), 2);
		assert_eq!(code.decode(0b10 << 14), 0);
		assert_eq!(code.decode(0b11 << 14), 1);
	}

	#[test]
	fn codes_longer_than_the_table_use_nodes() {
		// Lengths 1, 2, 3, 3 with 1 table bit: codes 0, 10, 110, 111.
		let mut code = Small::new(1);
		code.build(&[1, 2, 3, 3]).unwrap();

		assert_eq!(code.decode(0b0 << 15), 0);
		assert_eq!(code.decode(0b10 << 14), 1);
		assert_eq!(code.decode(0b110 << 13), 2);
		assert_eq!(code.decode(0b111 << 13), 3);
	}

	#[test]
	fn all_zero_lengths_clear_the_table() {
		let mut code = Small::new(2);
		code.build(&[1, 1, 0]).unwrap();
		code.build(&[0, 0, 0]).unwrap();

		assert_eq!(code.decode(0xFFFF), 0);
	}

	#[test]
	fn incomplete_and_oversubscribed_lengths_keep_the_previous_table() {
		let mut code = Small::new(2);
		code.build(&[2, 2, 1]).unwrap();

		assert_eq!(code.build(&[1, 0, 0]).unwrap(), Build::Kept);
		assert_eq!(code.build(&[1, 1, 1]).unwrap(), Build::Kept);
		assert_eq!(code.decode(0b11 << 14), 1);
	}

	#[test]
	fn oversubscribed_aligned_lengths_fill_until_a_code_does_not_fit() {
		// Eight 1-bit codes total 0x40000, which is 0 modulo 0x10000.
		let mut table = AlignedTable::new();
		table.build(&[1; 8]);

		assert_eq!(table.decode(0), 0);
		assert_eq!(table.decode(0x8000), 1);
	}
}
