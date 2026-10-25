extern crate std;

use std::vec;
use std::vec::Vec;

use lzxc::Encoder;
use lzxc::Strategy;
use lzxc::WindowSize;

use crate::Decoder;
use crate::Error;

const FRAME: usize = 0x8000;

/// Text-like input with repeats at short and long distances.
fn sample_input(len: usize) -> Vec<u8> {
	let words: [&[u8]; 8] =
		[b"xenon ", b"kernel ", b"window ", b"frame ", b"\xe8\x10\x20\x00\x00", b"lzx ", b"delta ", b"\n"];
	let mut state = 0x1234_5678u32;
	let mut data = Vec::with_capacity(len);
	while data.len() < len {
		state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
		data.extend_from_slice(words[(state >> 16) as usize % words.len()]);
		if state & 0x1F == 0 {
			data.push((state >> 8) as u8);
		}
	}
	data.truncate(len);

	data
}

fn decode_stream(window_size: usize, frames: &[Vec<u8>], len: usize) -> Vec<u8> {
	let mut decoder = Decoder::new(vec![0u8; window_size], FRAME).unwrap();
	let mut output = vec![0u8; len];
	for (frame, chunk) in frames.iter().zip(output.chunks_mut(FRAME)) {
		decoder.decompress(frame, chunk).unwrap();
	}

	output
}

#[test]
fn decodes_blocks_that_span_frames() {
	let input = sample_input(0x2_3456);
	let frames = Encoder::new(WindowSize::KB64).encode_stream(&input);

	assert_eq!(decode_stream(0x1_0000, &frames, input.len()), input);
}

#[test]
fn decodes_one_block_per_frame() {
	let input = sample_input(0x1_8000);
	let mut encoder = Encoder::new(WindowSize::KB32);
	let frames: Vec<Vec<u8>> = input.chunks(FRAME).map(|chunk| encoder.encode_chunk(chunk)).collect();

	assert_eq!(decode_stream(0x8000, &frames, input.len()), input);
}

#[test]
fn decodes_uncompressed_blocks() {
	let input = sample_input(0x1_2345);
	let frames = Encoder::new(WindowSize::KB32).with_strategy(Strategy::Uncompressed).encode_stream(&input);

	assert_eq!(decode_stream(0x8000, &frames, input.len()), input);
}

#[test]
fn reverses_e8_translation() {
	let input = sample_input(0x1_0000);
	let frames = Encoder::new(WindowSize::KB32).with_e8_translation(0x0C00_0000).encode_stream(&input);

	assert_eq!(decode_stream(0x8000, &frames, input.len()), input);
}

#[test]
fn rejects_frames_larger_than_the_limit() {
	let mut decoder = Decoder::new(vec![0u8; 0x8000], 0x100).unwrap();
	let mut output = [0u8; 0x101];

	assert_eq!(decoder.decompress(&[], &mut output), Err(Error::FrameTooLarge { requested: 0x101, max: 0x100 }));
}

/// Bits most significant first, packed into little-endian 16-bit words.
struct BitWriter {
	bytes: Vec<u8>,
	word: u16,
	used: u32,
}

impl BitWriter {
	fn new() -> Self {
		Self { bytes: Vec::new(), word: 0, used: 0 }
	}

	fn write(&mut self, value: u32, count: u32) {
		for bit in (0..count).rev() {
			self.word = (self.word << 1) | ((value >> bit) & 1) as u16;
			self.used += 1;
			if self.used == 16 {
				self.bytes.extend_from_slice(&self.word.to_le_bytes());
				self.word = 0;
				self.used = 0;
			}
		}
	}

	fn finish(mut self) -> Vec<u8> {
		if self.used > 0 {
			self.write(0, 16 - self.used);
		}

		self.bytes
	}
}

/// Pretree in which symbols 0, 16, 17 and 18 have the 2-bit codes 00, 01, 10
/// and 11. From all-zero previous lengths, symbol 0 yields length 0 and 16
/// yields length 1.
fn write_pretree(bits: &mut BitWriter) {
	for symbol in 0..20 {
		let length = if matches!(symbol, 0 | 16 | 17 | 18) { 2 } else { 0 };
		bits.write(length, 4);
	}
}

/// Code `count` zero lengths with symbols 18, 17 and 0.
fn write_zero_run(bits: &mut BitWriter, mut count: u32) {
	while count >= 20 {
		let run = count.min(51);
		bits.write(0b11, 2);
		bits.write(run - 20, 5);
		count -= run;
	}

	if count >= 4 {
		bits.write(0b10, 2);
		bits.write(count - 4, 4);
		count = 0;
	}

	for _ in 0..count {
		bits.write(0b00, 2);
	}
}

/// Lengths for `count` symbols, all zero except a length of one at `one`.
fn write_lengths(bits: &mut BitWriter, count: u32, one: Option<u32>) {
	write_pretree(bits);
	match one {
		Some(index) => {
			write_zero_run(bits, index);
			bits.write(0b01, 2);
			write_zero_run(bits, count - index - 1);
		}
		None => write_zero_run(bits, count),
	}
}

#[test]
fn reference_data_is_history_before_position_zero() {
	const LITERAL: u32 = b'A' as u32;
	// Position slot 4: distance 2 plus one extra bit; match length 2.
	const MATCH_FOOTER: u32 = 4 << 3;

	let mut bits = BitWriter::new();
	bits.write(0, 1);
	bits.write(1, 3);
	bits.write(5, 24);
	write_lengths(&mut bits, 256, Some(LITERAL));
	write_lengths(&mut bits, 30 * 8, Some(MATCH_FOOTER));
	write_lengths(&mut bits, 249, None);

	// The literal sorts before the match symbol, so it has code 0.
	bits.write(1, 1);
	bits.write(1, 1);
	bits.write(0, 1);
	bits.write(1, 1);
	bits.write(0, 1);
	let input = bits.finish();

	let mut decoder = Decoder::new(vec![0u8; 0x8000], FRAME).unwrap();
	decoder.set_reference(b"0123456789").unwrap();
	let mut output = [0u8; 5];
	decoder.decompress(&input, &mut output).unwrap();

	assert_eq!(&output, b"78A8A");
}

/// Pretree lengths in which symbols 0 to 11 have 4-bit codes and 12 to 19
/// 5-bit codes, and the canonical code of each symbol.
fn full_pretree() -> ([u8; 20], [(u32, u32); 20]) {
	let lengths: [u8; 20] = core::array::from_fn(|symbol| if symbol < 12 { 4 } else { 5 });
	let mut codes = [(0, 0); 20];
	let mut code = 0u32;
	let mut previous_length = 0;
	for (symbol, &length) in lengths.iter().enumerate() {
		code <<= u32::from(length) - previous_length;
		previous_length = u32::from(length);
		codes[symbol] = (code, u32::from(length));
		code += 1;
	}

	(lengths, codes)
}

/// Code each length of `target` as a difference from `previous`.
fn write_lengths_from(bits: &mut BitWriter, previous: &[u8], target: &[u8]) {
	let (lengths, codes) = full_pretree();
	for length in lengths {
		bits.write(u32::from(length), 4);
	}

	for (&old, &new) in previous.iter().zip(target) {
		let difference = (17 + u32::from(old) - u32::from(new)) % 17;
		let (code, length) = codes[difference as usize];
		bits.write(code, length);
	}
}

#[test]
fn rejected_main_code_decodes_with_the_previous_table() {
	const MAIN_SYMBOLS: usize = 256 + 30 * 8;

	let mut first = vec![0u8; MAIN_SYMBOLS];
	first[usize::from(b'A')] = 1;
	first[usize::from(b'B')] = 1;
	let mut second = vec![0u8; MAIN_SYMBOLS];
	second[usize::from(b'C')] = 1;

	let mut bits = BitWriter::new();
	bits.write(0, 1);
	bits.write(1, 3);
	bits.write(2, 24);
	write_lengths_from(&mut bits, &[0; 256], &first[..256]);
	write_lengths_from(&mut bits, &[0; MAIN_SYMBOLS - 256], &first[256..]);
	write_lengths_from(&mut bits, &[0; 249], &[0; 249]);
	bits.write(0b01, 2);

	// The second block's single 1-bit code is incomplete: the kernel keeps
	// the first block's table, skips the length code, and takes each
	// symbol's length from the new lengths, where `A` has length 0.
	bits.write(1, 3);
	bits.write(2, 24);
	write_lengths_from(&mut bits, &first[..256], &second[..256]);
	write_lengths_from(&mut bits, &first[256..], &second[256..]);
	bits.write(0, 2);
	let input = bits.finish();

	let mut decoder = Decoder::new(vec![0u8; 0x8000], FRAME).unwrap();
	let mut output = [0u8; 4];
	decoder.decompress(&input, &mut output).unwrap();

	assert_eq!(&output, b"ABAA");
}
