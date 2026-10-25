/// Bit reader over the compressed input of one frame.
///
/// The console primes a 32-bit buffer with two little-endian 16-bit words and
/// refills one word at a time when no more than 16 valid bits remain. It may
/// read up to four bytes past the end of the frame input; those bytes read as
/// zero here. A valid stream never decodes from them.
pub(crate) struct BitReader<'a> {
	input: &'a [u8],
	/// Offset of the next word to load into `buffer`.
	pub(crate) position: usize,
	/// One past the last byte the console reads: the input length plus four.
	pub(crate) end: usize,
	buffer: u32,
	/// Valid bits in `buffer` beyond the first 16.
	extra: i32,
	/// A checked refill found no input left.
	exhausted: bool,
}

impl<'a> BitReader<'a> {
	pub(crate) fn new(input: &'a [u8]) -> Self {
		Self { input, position: 0, end: input.len() + 4, buffer: 0, extra: 0, exhausted: false }
	}

	pub(crate) fn is_exhausted(&self) -> bool {
		self.exhausted
	}

	/// Load two words into an empty buffer. Leaves the reader unchanged when
	/// fewer than four bytes remain before `end`.
	pub(crate) fn prime(&mut self) {
		if self.position + 4 > self.end {
			return;
		}

		self.buffer = (self.word(self.position) << 16) | self.word(self.position + 2);
		self.position += 4;
		self.extra = 16;
	}

	pub(crate) fn byte(&self, at: usize) -> u8 {
		self.input.get(at).copied().unwrap_or(0)
	}

	pub(crate) fn u32_le(&self, at: usize) -> u32 {
		u32::from_le_bytes([self.byte(at), self.byte(at + 1), self.byte(at + 2), self.byte(at + 3)])
	}

	fn word(&self, at: usize) -> u32 {
		u32::from(u16::from_le_bytes([self.byte(at), self.byte(at + 1)]))
	}

	/// The next 16 bits, most significant first.
	pub(crate) fn peek16(&self) -> u32 {
		self.buffer >> 16
	}

	/// Read `count` bits (1 to 16) and refill with end checks, as the header
	/// and code-length readers do.
	pub(crate) fn read(&mut self, count: u32) -> u32 {
		let value = self.buffer >> (32 - count);
		self.consume(count);

		value
	}

	/// Discard `count` bits and refill with end checks. A refill at the end of
	/// the input marks the reader exhausted and leaves the buffer short.
	pub(crate) fn consume(&mut self, count: u32) {
		self.discard(count);
		for _ in 0..2 {
			if self.extra > 0 {
				return;
			}

			if self.position >= self.end {
				self.exhausted = true;
				return;
			}

			self.refill();
		}
	}

	/// Read `count` bits (1 to 17) with the symbol decoders' refill, which
	/// does not check the end of the input.
	pub(crate) fn read_unchecked(&mut self, count: u32) -> u32 {
		let value = self.buffer >> (32 - count);
		self.consume_unchecked(count);

		value
	}

	/// Discard `count` bits with the symbol decoders' refill.
	pub(crate) fn consume_unchecked(&mut self, count: u32) {
		self.discard(count);
		while self.extra <= 0 {
			self.refill();
		}
	}

	fn discard(&mut self, count: u32) {
		self.buffer = self.buffer.checked_shl(count).unwrap_or(0);
		self.extra -= count as i32;
	}

	fn refill(&mut self) {
		self.buffer |= self.word(self.position) << -self.extra;
		self.position += 2;
		self.extra += 16;
	}
}
