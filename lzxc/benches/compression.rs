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
		("text-256k", generate_text(256 * 1024, 0xA17B_CD03)),
		("text-256k-pathological", generate_pathological_text(256 * 1024)),
		("structured-1m", generate_structured(1024 * 1024)),
		("random-256k", generate_random(256 * 1024, 0xCAFE_F00D)),
	]
}

/// Realistic-ish English text: emits words drawn by LCG from a vocabulary
/// of ~200 common English words, with occasional sentence boundaries.
/// Word-level (not phrase-level) selection keeps byte-level patterns
/// English-like (common bigrams, realistic letter frequencies) while
/// eliminating the degenerate long-periodicity of the pathological
/// variant, so the match finder lands on medium-distance matches
/// comparable to what it gets on real prose. Target ratio is the 2-3x
/// range typical of LZ77+Huffman on natural-language text.
fn generate_text(len: usize, seed: u32) -> Vec<u8> {
	// Frequency-weighted-ish common English words. Ordering matters less
	// than having enough variety that no single word dominates; ~200 words
	// is enough to approximate real token distribution at this scale.
	let words: &[&[u8]] = &[
		b"the", b"be", b"to", b"of", b"and", b"a", b"in", b"that", b"have", b"it",
		b"for", b"not", b"on", b"with", b"he", b"as", b"you", b"do", b"at", b"this",
		b"but", b"his", b"by", b"from", b"they", b"we", b"say", b"her", b"she", b"or",
		b"an", b"will", b"my", b"one", b"all", b"would", b"there", b"their", b"what", b"so",
		b"up", b"out", b"if", b"about", b"who", b"get", b"which", b"go", b"me", b"when",
		b"make", b"can", b"like", b"time", b"no", b"just", b"him", b"know", b"take", b"people",
		b"into", b"year", b"your", b"good", b"some", b"could", b"them", b"see", b"other", b"than",
		b"then", b"now", b"look", b"only", b"come", b"its", b"over", b"think", b"also", b"back",
		b"after", b"use", b"two", b"how", b"our", b"work", b"first", b"well", b"way", b"even",
		b"new", b"want", b"any", b"these", b"give", b"day", b"most", b"us", b"is", b"was",
		b"are", b"were", b"been", b"has", b"had", b"did", b"does", b"here", b"more", b"very",
		b"still", b"should", b"through", b"where", b"before", b"because", b"while", b"around", b"small", b"large",
		b"open", b"close", b"begin", b"end", b"stand", b"walk", b"run", b"write", b"read", b"speak",
		b"house", b"world", b"school", b"night", b"light", b"water", b"fire", b"earth", b"hand", b"eye",
		b"face", b"door", b"room", b"place", b"name", b"word", b"book", b"thing", b"part", b"life",
		b"story", b"idea", b"case", b"fact", b"point", b"group", b"state", b"family", b"number", b"side",
		b"child", b"mother", b"father", b"friend", b"woman", b"man", b"body", b"head", b"heart", b"mind",
		b"tree", b"river", b"road", b"field", b"city", b"street", b"country", b"hill", b"sea", b"sky",
		b"old", b"young", b"long", b"short", b"high", b"low", b"right", b"left", b"far", b"near",
		b"hot", b"cold", b"dark", b"bright", b"red", b"green", b"blue", b"white", b"black", b"gold",
	];
	let mut out = Vec::with_capacity(len);
	let mut state = seed;
	let mut words_this_sentence: u32 = 0;
	while out.len() < len {
		state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
		let word = words[(state as usize >> 16) % words.len()];
		out.extend_from_slice(word);
		if out.len() >= len {
			break;
		}
		words_this_sentence += 1;
		// Roughly every ~12 words, end a sentence with ". " and capitalize
		// the next word's first letter for variety. Uses another LCG draw.
		state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
		if words_this_sentence >= 8 && (state >> 16) as u32 % 5 == 0 {
			out.push(b'.');
			out.push(b' ');
			words_this_sentence = 0;
		} else {
			out.push(b' ');
		}
	}
	out.truncate(len);
	out
}

/// Degenerate best case: a single 107-byte phrase repeated to fill `len`.
/// After chunk 1 every position matches back at offset 107 with length
/// MAX_MATCH, and every match encodes to the same main-tree symbol, so
/// this compresses to the LZ77+Huffman best case (~270x). Kept as a
/// sanity-check ceiling; it is **not** representative of realistic text.
fn generate_pathological_text(len: usize) -> Vec<u8> {
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
