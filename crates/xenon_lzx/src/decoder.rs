use core::num::NonZeroU32;

use crate::bits::BitReader;
use crate::error::Error;
use crate::huffman::AlignedTable;
use crate::huffman::Build;
use crate::huffman::DecodeTable;

const MIN_WINDOW_SIZE: usize = 0x8000;
const MAX_WINDOW_SIZE: usize = 0x20_0000;
const MAX_POSITION_SLOTS: usize = 50;
const LITERAL_COUNT: usize = 256;
const MAX_MAIN_SYMBOLS: usize = LITERAL_COUNT + MAX_POSITION_SLOTS * 8;
const LENGTH_SYMBOLS: usize = 249;
const PRETREE_SYMBOLS: usize = 20;
const ALIGNED_SYMBOLS: usize = 8;
const MIN_MATCH: u32 = 2;
const MAIN_TABLE_BITS: u32 = 10;
const LENGTH_TABLE_BITS: u32 = 8;
const PRETREE_TABLE_BITS: u32 = 8;

/// E8 translation stops after this many frames.
const E8_FRAME_LIMIT: u32 = 0x8000;

/// Extra offset bits per position slot (kernel 21256 `0x801589f8`).
const EXTRA_BITS: [u8; 52] = [
	0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15,
	16, 16, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17,
];

/// Formatted offset base per position slot, as the kernel stores it: the
/// match distance is this base plus the slot's extra bits. Slots 0 to 2
/// select a repeated offset and do not use it.
const POSITION_BASE: [u32; 51] = [
	0xFFFF_FFFE,
	0xFFFF_FFFF,
	0,
	1,
	2,
	4,
	6,
	10,
	14,
	22,
	30,
	46,
	62,
	94,
	126,
	190,
	254,
	382,
	510,
	766,
	1022,
	1534,
	2046,
	3070,
	4094,
	6142,
	8190,
	12286,
	16382,
	24574,
	32766,
	49150,
	65534,
	98302,
	131070,
	196606,
	262142,
	393214,
	524286,
	655358,
	786430,
	917502,
	1048574,
	1179646,
	1310718,
	1441790,
	1572862,
	1703934,
	1835006,
	1966078,
	2097150,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
	Verbatim,
	AlignedOffset,
	Uncompressed,
}

/// The block being decoded. A block may span frames.
struct Block {
	/// `None` before the first block header and after the padding that
	/// follows an uncompressed block.
	kind: Option<BlockKind>,
	/// Uncompressed size from the block header.
	size: u32,
	/// Bytes of the block not yet decoded. A block header is read when zero.
	remaining: u32,
}

/// LZX decoder with the behavior of the Xbox 360 kernel's LDI decompressor
/// (`LDICreateDecompression`, `LDIDecompress`, `LDIResetDecompression` and
/// the window-data call at kernel 21256 `0x80158858`).
///
/// Each [`decompress`][Self::decompress] call decodes one frame: a separately
/// byte-aligned piece of compressed input that expands to a caller-chosen
/// size. Block, code and window state persist across frames until
/// [`reset`][Self::reset].
///
/// `W` is the window storage, exactly the window size in bytes. The decoder
/// does not allocate.
pub struct Decoder<W> {
	window: W,
	window_mask: u32,
	max_frame: usize,
	position_slots: usize,

	position: u32,
	repeated_offsets: [u32; 3],
	block: Block,

	main_lengths: [u8; MAX_MAIN_SYMBOLS],
	previous_main_lengths: [u8; MAX_MAIN_SYMBOLS],
	length_lengths: [u8; LENGTH_SYMBOLS],
	previous_length_lengths: [u8; LENGTH_SYMBOLS],
	aligned_lengths: [u8; ALIGNED_SYMBOLS],
	main_code: DecodeTable<{ 1 << MAIN_TABLE_BITS }, MAX_MAIN_SYMBOLS>,
	length_code: DecodeTable<{ 1 << LENGTH_TABLE_BITS }, LENGTH_SYMBOLS>,
	aligned_code: AlignedTable,

	e8_header_pending: bool,
	e8_translation_size: Option<NonZeroU32>,
	e8_position: u32,
	frame_count: u32,
}

impl<W> Decoder<W>
where
	W: AsRef<[u8]> + AsMut<[u8]>,
{
	/// Create a decoder over `window`, whose length is the LZX window size: a
	/// power of two from 32 KiB to 2 MiB. Frames may expand to at most
	/// `max_frame` bytes, the console's maximum block size argument.
	pub fn new(window: W, max_frame: usize) -> Result<Self, Error> {
		let size = window.as_ref().len();
		if !size.is_power_of_two() || !(MIN_WINDOW_SIZE..=MAX_WINDOW_SIZE).contains(&size) {
			return Err(Error::InvalidWindowSize { size });
		}

		let mut covered = 4;
		let mut position_slots = 4;
		while covered < size {
			covered += 1 << EXTRA_BITS[position_slots];
			position_slots += 1;
		}

		let mut decoder = Self {
			window,
			window_mask: (size - 1) as u32,
			max_frame,
			position_slots,
			position: 0,
			repeated_offsets: [1; 3],
			block: Block { kind: None, size: 0, remaining: 0 },
			main_lengths: [0; MAX_MAIN_SYMBOLS],
			previous_main_lengths: [0; MAX_MAIN_SYMBOLS],
			length_lengths: [0; LENGTH_SYMBOLS],
			previous_length_lengths: [0; LENGTH_SYMBOLS],
			aligned_lengths: [0; ALIGNED_SYMBOLS],
			main_code: DecodeTable::new(MAIN_TABLE_BITS),
			length_code: DecodeTable::new(LENGTH_TABLE_BITS),
			aligned_code: AlignedTable::new(),
			e8_header_pending: true,
			e8_translation_size: None,
			e8_position: 0,
			frame_count: 0,
		};
		decoder.window.as_mut().fill(0);
		decoder.reset();

		Ok(decoder)
	}

	/// Return to the state of a new stream. The window contents are kept.
	pub fn reset(&mut self) {
		self.main_lengths.fill(0);
		self.previous_main_lengths.fill(0);
		self.length_lengths.fill(0);
		self.previous_length_lengths.fill(0);
		self.position = 0;
		self.repeated_offsets = [1; 3];
		self.block = Block { kind: None, size: 0, remaining: 0 };
		self.e8_header_pending = true;
		self.e8_translation_size = None;
		self.e8_position = 0;
		self.frame_count = 0;
	}

	/// Place `data` at the end of the window and zero the rest, so that the
	/// next frame can copy from it as history.
	pub fn set_reference(&mut self, data: &[u8]) -> Result<(), Error> {
		let window = self.window.as_mut();
		let Some(split) = window.len().checked_sub(data.len()) else {
			return Err(Error::ReferenceTooLarge { len: data.len(), window: window.len() });
		};

		window[..split].fill(0);
		window[split..].copy_from_slice(data);

		Ok(())
	}

	/// Decode one frame of `input` into exactly `output.len()` bytes.
	pub fn decompress(&mut self, input: &[u8], output: &mut [u8]) -> Result<(), Error> {
		let len = output.len();
		if len > self.max_frame {
			return Err(Error::FrameTooLarge { requested: len, max: self.max_frame });
		}

		let start = self.position as usize;
		if start + len > self.window.as_ref().len() {
			return Err(Error::FrameCrossesWindowEnd { position: self.position, len });
		}

		let mut bits = BitReader::new(input);
		if self.block.kind != Some(BlockKind::Uncompressed) {
			bits.prime();
		}

		let result = self.decode_frame(&mut bits, len as u32);
		let frame_index = self.frame_count;
		self.frame_count = self.frame_count.wrapping_add(1);
		result?;

		output.copy_from_slice(&self.window.as_ref()[start..start + len]);
		if let Some(translation_size) = self.e8_translation_size
			&& frame_index < E8_FRAME_LIMIT
		{
			self.translate_e8(output, translation_size.get());
		}

		Ok(())
	}

	fn decode_frame(&mut self, bits: &mut BitReader<'_>, len: u32) -> Result<(), Error> {
		let mut frame_remaining = len;
		while frame_remaining > 0 {
			if self.block.remaining == 0 {
				self.read_block_header(bits)?;
			}

			while self.block.remaining > 0 && frame_remaining > 0 {
				let count = self.block.remaining.min(frame_remaining);
				match self.block.kind {
					Some(BlockKind::Verbatim) => self.decode_compressed(bits, count, false)?,
					Some(BlockKind::AlignedOffset) => self.decode_compressed(bits, count, true)?,
					Some(BlockKind::Uncompressed) => self.copy_uncompressed(bits, count)?,
					None => return Err(Error::InvalidBlockType { kind: 0 }),
				}

				self.block.remaining -= count;
				frame_remaining -= count;
			}
		}

		Ok(())
	}

	fn read_block_header(&mut self, bits: &mut BitReader<'_>) -> Result<(), Error> {
		if self.e8_header_pending {
			self.e8_header_pending = false;
			self.e8_translation_size = if bits.read(1) != 0 {
				let high = bits.read(16);
				let low = bits.read(16);
				NonZeroU32::new((high << 16) | low)
			} else {
				None
			};
		}

		// An uncompressed block is padded to an even length, and the bit
		// stream restarts after it.
		if self.block.kind == Some(BlockKind::Uncompressed) {
			if self.block.size & 1 != 0 && bits.position < bits.end {
				bits.position += 1;
			}

			self.block.kind = None;
			bits.prime();
		}

		let kind = bits.read(3);
		let size = (bits.read(8) << 16) | (bits.read(8) << 8) | bits.read(8);
		self.block.size = size;
		self.block.remaining = size;
		self.block.kind = match kind {
			1 => Some(BlockKind::Verbatim),
			2 => Some(BlockKind::AlignedOffset),
			3 => Some(BlockKind::Uncompressed),
			_ => return Err(Error::InvalidBlockType { kind }),
		};

		match self.block.kind {
			Some(BlockKind::Verbatim) => self.read_codes(bits),
			Some(BlockKind::AlignedOffset) => {
				self.read_aligned_code(bits)?;
				self.read_codes(bits)
			}
			Some(BlockKind::Uncompressed) => self.begin_uncompressed(bits),
			None => unreachable!("block kind was assigned above"),
		}
	}

	fn read_aligned_code(&mut self, bits: &mut BitReader<'_>) -> Result<(), Error> {
		for length in &mut self.aligned_lengths {
			*length = bits.read(3) as u8;
		}

		if bits.is_exhausted() {
			return Err(Error::InputExhausted);
		}

		// The kernel ignores the result (kernel 21256 `0x801590ec`).
		self.aligned_code.build(&self.aligned_lengths);

		Ok(())
	}

	/// Read the main and length code lengths, each coded as a difference from
	/// the lengths of the previous verbatim or aligned offset block.
	fn read_codes(&mut self, bits: &mut BitReader<'_>) -> Result<(), Error> {
		let main_count = LITERAL_COUNT + self.position_slots * 8;
		self.previous_main_lengths[..main_count].copy_from_slice(&self.main_lengths[..main_count]);
		self.previous_length_lengths = self.length_lengths;

		read_lengths(bits, &self.previous_main_lengths[..LITERAL_COUNT], &mut self.main_lengths[..LITERAL_COUNT])?;
		read_lengths(
			bits,
			&self.previous_main_lengths[LITERAL_COUNT..main_count],
			&mut self.main_lengths[LITERAL_COUNT..main_count],
		)?;
		// The kernel ignores the result of building the codes (`0x80159150`).
		// A rejected main code ends the code reading early, so the length
		// lengths and the length table stay as they were.
		if self.main_code.build(&self.main_lengths[..main_count])? == Build::Kept {
			return Ok(());
		}

		read_lengths(bits, &self.previous_length_lengths, &mut self.length_lengths)?;
		self.length_code.build(&self.length_lengths)?;

		Ok(())
	}

	/// The repeated offsets of an uncompressed block follow its header,
	/// aligned to the 16-bit word after the one the header ends in.
	fn begin_uncompressed(&mut self, bits: &mut BitReader<'_>) -> Result<(), Error> {
		let Some(position) = bits.position.checked_sub(2) else {
			return Err(Error::InputExhausted);
		};

		if position + 4 >= bits.end {
			return Err(Error::InputExhausted);
		}

		for (index, offset) in self.repeated_offsets.iter_mut().enumerate() {
			*offset = bits.u32_le(position + index * 4);
		}
		bits.position = position + 12;

		Ok(())
	}

	fn copy_uncompressed(&mut self, bits: &mut BitReader<'_>, count: u32) -> Result<(), Error> {
		let mask = self.window_mask;
		let window = self.window.as_mut();
		for index in 0..count {
			if bits.position >= bits.end {
				return Err(Error::InputExhausted);
			}

			window[(self.position.wrapping_add(index) & mask) as usize] = bits.byte(bits.position);
			bits.position += 1;
		}
		self.position = self.position.wrapping_add(count) & mask;

		Ok(())
	}

	/// Decode `count` bytes of a verbatim or aligned offset block. A match
	/// may not end past `count`.
	fn decode_compressed(&mut self, bits: &mut BitReader<'_>, count: u32, aligned: bool) -> Result<(), Error> {
		let mask = self.window_mask;
		let mut position = self.position;
		let end = position + count;
		while position < end {
			let main = self.main_code.decode(bits.peek16());
			let main_length = u32::from(self.main_lengths[usize::from(main)]);
			if bits.position >= bits.end {
				return Err(Error::InputExhausted);
			}
			bits.consume_unchecked(main_length);

			if usize::from(main) < LITERAL_COUNT {
				self.window.as_mut()[(position & mask) as usize] = main as u8;
				position += 1;
				continue;
			}

			let header = u32::from(main) - LITERAL_COUNT as u32;
			let slot = (header >> 3) as usize;
			let mut match_length = header & 7;
			if match_length == 7 {
				let length = self.length_code.decode(bits.peek16());
				let code_length = u32::from(self.length_lengths[usize::from(length)]);
				bits.consume_unchecked(code_length);
				match_length += u32::from(length);
			}
			match_length += MIN_MATCH;

			let distance = match slot {
				0 => self.repeated_offsets[0],
				1 | 2 => {
					self.repeated_offsets.swap(0, slot);
					self.repeated_offsets[0]
				}
				_ => {
					let distance = self.read_offset(bits, slot, aligned);
					self.repeated_offsets[2] = self.repeated_offsets[1];
					self.repeated_offsets[1] = self.repeated_offsets[0];
					self.repeated_offsets[0] = distance;
					distance
				}
			};

			let window = self.window.as_mut();
			for _ in 0..match_length {
				window[(position & mask) as usize] = window[(position.wrapping_sub(distance) & mask) as usize];
				position += 1;
			}
		}

		if position != end {
			return Err(Error::MatchOverrunsFrame { overrun: position - end });
		}
		self.position = end & mask;

		Ok(())
	}

	fn read_offset(&mut self, bits: &mut BitReader<'_>, slot: usize, aligned: bool) -> u32 {
		let extra = u32::from(EXTRA_BITS[slot]);
		let base = POSITION_BASE[slot];
		if slot == 3 {
			return 1;
		}

		if !aligned || extra < 3 {
			return base.wrapping_add(bits.read_unchecked(extra));
		}

		let verbatim = if extra > 3 { bits.read_unchecked(extra - 3) } else { 0 };
		let aligned_bits = self.aligned_code.decode(bits.peek16());
		let code_length = u32::from(self.aligned_lengths[usize::from(aligned_bits)]);
		bits.consume_unchecked(code_length);

		base.wrapping_add(verbatim << 3).wrapping_add(u32::from(aligned_bits))
	}

	/// Convert the absolute `call` targets the encoder wrote back to relative
	/// ones (kernel 21256 `0x80158ea0`).
	fn translate_e8(&mut self, output: &mut [u8], translation_size: u32) {
		const TAIL: usize = 6;

		let len = output.len();
		if len <= TAIL {
			self.e8_position = self.e8_position.wrapping_add(len as u32);
			return;
		}

		// Sentinel bytes stop the scan without bounds checks; translation
		// never starts in the last ten bytes.
		let mut saved = [0u8; TAIL];
		saved.copy_from_slice(&output[len - TAIL..]);
		output[len - TAIL..].fill(0xE8);
		let limit = self.e8_position.wrapping_add(len as u32).wrapping_sub(10);

		let mut index = 0;
		loop {
			while output[index] != 0xE8 {
				self.e8_position = self.e8_position.wrapping_add(1);
				index += 1;
			}

			let current = self.e8_position;
			if current >= limit {
				break;
			}

			let operand = &mut output[index + 1..index + 5];
			let absolute = u32::from_le_bytes([operand[0], operand[1], operand[2], operand[3]]);
			if absolute < translation_size {
				operand.copy_from_slice(&absolute.wrapping_sub(current).to_le_bytes());
			} else if absolute.wrapping_neg() <= current {
				operand.copy_from_slice(&absolute.wrapping_add(translation_size).to_le_bytes());
			}

			index += 5;
			self.e8_position = self.e8_position.wrapping_add(5);
		}

		self.e8_position = limit.wrapping_add(10);
		output[len - TAIL..].copy_from_slice(&saved);
	}
}

/// Read code lengths through a pretree. `previous` holds the lengths the
/// differences apply to; `lengths` receives the result.
fn read_lengths(bits: &mut BitReader<'_>, previous: &[u8], lengths: &mut [u8]) -> Result<(), Error> {
	let mut pretree_lengths = [0u8; PRETREE_SYMBOLS];
	for length in &mut pretree_lengths {
		*length = bits.read(4) as u8;
	}

	if bits.is_exhausted() {
		return Err(Error::InputExhausted);
	}

	// The pretree tables are on the kernel's stack (`0x8015a2d8`): a rejected
	// pretree decodes with whatever the stack held, so no defined result exists.
	let mut pretree = DecodeTable::<{ 1 << PRETREE_TABLE_BITS }, PRETREE_SYMBOLS>::new(PRETREE_TABLE_BITS);
	if pretree.build(&pretree_lengths)? == Build::Kept {
		return Err(Error::IncompletePretree);
	}

	let count = lengths.len();
	let mut index = 0;
	while index < count {
		let symbol = pretree.decode(bits.peek16());
		let code_length = u32::from(pretree_lengths[usize::from(symbol)]);
		bits.consume(code_length);
		if bits.is_exhausted() {
			return Err(Error::InputExhausted);
		}

		match symbol {
			17 | 18 => {
				let run = if symbol == 17 { bits.read(4) + 4 } else { bits.read(5) + 20 };
				let run = (run as usize).min(count - index);
				lengths[index..index + run].fill(0);
				index += run;
			}
			19 => {
				let run = ((bits.read(1) + 4) as usize).min(count - index);
				let difference = pretree.decode(bits.peek16());
				let code_length = u32::from(pretree_lengths[usize::from(difference)]);
				bits.consume(code_length);
				let length = apply_difference(previous[index], difference);
				lengths[index..index + run].fill(length);
				index += run;
			}
			_ => {
				lengths[index] = apply_difference(previous[index], symbol);
				index += 1;
			}
		}
	}

	if bits.is_exhausted() {
		return Err(Error::InputExhausted);
	}

	Ok(())
}

/// The console computes `(previous - difference + 17) mod 17` with one
/// conditional subtraction; a difference above 16 yields a length that the
/// code builder rejects.
fn apply_difference(previous: u8, difference: u16) -> u8 {
	let mut length = i32::from(previous) - i32::from(difference) + 17;
	if length >= 17 {
		length -= 17;
	}

	length as u8
}
