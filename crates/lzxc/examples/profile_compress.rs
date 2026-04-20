//! Hot loop for profiling. Generates synthetic binary-shaped data on the
//! fly, so no external file is required.
//!
//! Run with: `cargo instruments --release --example profile_compress -t "Time Profiler"`

use lzxc::Encoder;
use lzxc::MAX_CHUNK_SIZE;
use lzxc::WindowSize;

fn main() {
	let iters: usize = std::env::var("ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
	let bytes: usize = std::env::var("BYTES").ok().and_then(|s| s.parse().ok()).unwrap_or(3 * 1024 * 1024);
	let data = structured(bytes);

	eprintln!("input: {} bytes, iters: {}", data.len(), iters);
	let t0 = std::time::Instant::now();
	let mut total_out = 0usize;
	for _ in 0..iters {
		let mut enc = Encoder::new(WindowSize::KB64);
		for chunk in data.chunks(MAX_CHUNK_SIZE) {
			total_out += enc.encode_chunk(chunk).len();
		}
	}
	let dt = t0.elapsed();
	eprintln!(
		"total {:?}, per-iter {:?}, throughput {:.1} MB/s, avg compressed {} bytes",
		dt,
		dt / iters as u32,
		(data.len() * iters) as f64 / dt.as_secs_f64() / 1_000_000.0,
		total_out / iters
	);
}

fn structured(len: usize) -> Vec<u8> {
	let mut out = Vec::with_capacity(len);
	let quarter = len / 4;

	out.resize(quarter, 0);

	let template: [u8; 64] = [
		0x48, 0x89, 0xE5, 0x48, 0x83, 0xEC, 0x20, 0xC7, 0x45, 0xFC, 0x00, 0x00, 0x00, 0x00, 0xE8, 0x00, 0x00, 0x00,
		0x00, 0x48, 0x8B, 0x45, 0xF8, 0x48, 0x83, 0xC4, 0x20, 0x5D, 0xC3, 0x90, 0x90, 0x90, 0x66, 0x90, 0x0F, 0x1F,
		0x44, 0x00, 0x00, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0x55, 0x48, 0x89, 0xE5, 0x41, 0x57,
		0x41, 0x56, 0x41, 0x55, 0x41, 0x54, 0x53, 0x48, 0x83, 0xEC,
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
