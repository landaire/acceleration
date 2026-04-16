//! Throughput and compression-ratio benchmarks for `lzxc`.
//!
//! Run with `cargo bench -p lzxc`. Each run also prints a one-line
//! compression-ratio summary for every corpus/strategy combination to
//! stderr so the README numbers can be regenerated.

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use lzxc::{Encoder, MAX_CHUNK_SIZE, Strategy, WindowSize};
use std::hint::black_box;
use std::io::Write as _;
use std::sync::OnceLock;

/// Inputs used for all benchmark groups. Each entry is `(label, bytes)`.
fn corpus() -> &'static [(&'static str, Vec<u8>)] {
	static CORPUS: OnceLock<Vec<(&'static str, Vec<u8>)>> = OnceLock::new();
	CORPUS.get_or_init(build_corpus)
}

fn build_corpus() -> Vec<(&'static str, Vec<u8>)> {
	// Corpora are generated deterministically from seeds so results are
	// reproducible on any host and the bench crate has no file fixtures.
	vec![
		("text-256k", generate_text(256 * 1024)),
		("structured-1m", generate_structured(1024 * 1024)),
		("random-256k", generate_random(256 * 1024, 0xCAFE_F00D)),
	]
}

/// Repetitive English-ish text. Small alphabet + phrase-level repetition
/// exercises both the Huffman side (literal bias) and the match finder.
fn generate_text(len: usize) -> Vec<u8> {
	let phrase = b"The quick brown fox jumps over the lazy dog. \
	               Lorem ipsum dolor sit amet, consectetur adipiscing elit. ";
	let mut out = Vec::with_capacity(len);
	while out.len() < len {
		let take = phrase.len().min(len - out.len());
		out.extend_from_slice(&phrase[..take]);
	}
	out
}

/// A synthetic binary-ish payload: a quarter zero-runs (BSS-like padding),
/// a quarter a repeating 64-byte "jump table" template, a quarter
/// low-entropy bytes biased toward common opcode-like values (0x00, 0x48,
/// 0xFF, 0x89, 0xE8), and a quarter LCG random. Roughly approximates the
/// compressibility mix of real executable files without impersonating any
/// specific one.
fn generate_structured(len: usize) -> Vec<u8> {
	let mut out = Vec::with_capacity(len);
	let quarter = len / 4;

	out.resize(quarter, 0);

	let template: [u8; 64] = [
		0x48, 0x89, 0xE5, 0x48, 0x83, 0xEC, 0x20, 0xC7, 0x45, 0xFC, 0x00, 0x00, 0x00, 0x00, 0xE8, 0x00,
		0x00, 0x00, 0x00, 0x48, 0x8B, 0x45, 0xF8, 0x48, 0x83, 0xC4, 0x20, 0x5D, 0xC3, 0x90, 0x90, 0x90,
		0x66, 0x90, 0x0F, 0x1F, 0x44, 0x00, 0x00, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC,
		0x55, 0x48, 0x89, 0xE5, 0x41, 0x57, 0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x48, 0x83, 0xEC,
	];
	while out.len() < 2 * quarter {
		let take = template.len().min(2 * quarter - out.len());
		out.extend_from_slice(&template[..take]);
	}

	let biased = [0x00u8, 0x48, 0xFF, 0x89, 0xE8, 0x90, 0xCC, 0x00, 0x00, 0x48];
	let mut state: u32 = 0xDEAD_BEEF;
	while out.len() < 3 * quarter {
		state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
		out.push(biased[(state as usize) % biased.len()]);
	}

	let mut state: u32 = 0xFACE_B00C;
	while out.len() < len {
		state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
		out.push((state >> 16) as u8);
	}

	out
}

/// Pseudo-random bytes from a tiny LCG. Worst case for LZ matching; the
/// encoder should fall through to uncompressed blocks.
fn generate_random(len: usize, seed: u32) -> Vec<u8> {
	let mut state = seed;
	let mut out = Vec::with_capacity(len);
	for _ in 0..len {
		state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
		out.push((state >> 16) as u8);
	}
	out
}

/// Compress `input` chunk-by-chunk and return the total compressed byte count.
fn compress_total(input: &[u8], strategy: Strategy, window: WindowSize) -> usize {
	let mut enc = Encoder::new(window).with_strategy(strategy);
	input.chunks(MAX_CHUNK_SIZE).map(|c| enc.encode_chunk(c).len()).sum()
}

fn bench_at(c: &mut Criterion, strategy: Strategy, window: WindowSize, group_name: &str, label_suffix: &str) {
	let mut group = c.benchmark_group(group_name);
	for (label, input) in corpus() {
		let param = format!("{}-{}", label, label_suffix);
		group.throughput(Throughput::Bytes(input.len() as u64));
		group.bench_with_input(BenchmarkId::from_parameter(&param), input, |b, input| {
			b.iter(|| {
				let mut enc = Encoder::new(window).with_strategy(strategy);
				for chunk in input.chunks(MAX_CHUNK_SIZE) {
					black_box(enc.encode_chunk(chunk));
				}
			});
		});
		let compressed = compress_total(input, strategy, window);
		let ratio = input.len() as f64 / compressed.max(1) as f64;
		let _ = writeln!(
			std::io::stderr(),
			"ratio: {:>20} / {:<20} {:>9} -> {:>9}  ({:.2}x)",
			group_name,
			param,
			input.len(),
			compressed,
			ratio
		);
	}
	group.finish();
}

/// Default strategy at the default window size: the common case.
fn greedy_kb64(c: &mut Criterion) {
	bench_at(c, Strategy::Greedy, WindowSize::KB64, "greedy", "kb64");
}

/// Sweep Greedy across window sizes so readers can see how window choice
/// trades off ratio vs throughput. Window affects the match-finder's
/// `max_offset`, so bigger windows let matches reach farther back.
fn greedy_window_sweep(c: &mut Criterion) {
	for (window, suffix) in [
		(WindowSize::KB32, "kb32"),
		(WindowSize::KB128, "kb128"),
		(WindowSize::KB512, "kb512"),
		(WindowSize::MB1, "mb1"),
		(WindowSize::MB2, "mb2"),
	] {
		bench_at(c, Strategy::Greedy, window, "greedy-window-sweep", suffix);
	}
}

fn literal_only(c: &mut Criterion) {
	bench_at(c, Strategy::LiteralOnly, WindowSize::KB64, "literal-only", "kb64");
}

fn uncompressed(c: &mut Criterion) {
	bench_at(c, Strategy::Uncompressed, WindowSize::KB64, "uncompressed", "kb64");
}

criterion_group!(benches, greedy_kb64, greedy_window_sweep, literal_only, uncompressed);
criterion_main!(benches);
