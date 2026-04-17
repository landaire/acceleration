//! LZX compression of a PE image into an XEX "Normal" data region.
//!
//! Inverse of [`crate::basefile::decompress_normal`]. Each invocation produces
//! a single-block stream:
//!
//!   [u32 next_block_size = 0]
//!   [u8; 20 next_block_hash = 0]
//!   [u16 BE chunk_size][lzx_data] ...
//!   [u16 BE 0]   (terminator)
//!
//! The LZX encoder state persists across chunks within the stream, matching
//! what [`crate::basefile::decompress_normal`] expects on the decode side.
//!
//! Returned alongside the data region is the `first_block_hash` (SHA-1 of the
//! single block's bytes), which must be stitched into the FileFormatInfo blob
//! so the hash-chain invariant holds.

use crate::error::Result;
use crate::error::Xex2Error;
use crate::header::Sha1Hash;
use rootcause::IntoReport;
use sha1::Digest;
use std::io::Write;

/// A compressed data region and the integrity metadata the XEX header needs.
pub struct CompressedStream {
	/// The data region, ready to drop in at `Xex2Header::data_offset`.
	pub data: Vec<u8>,
	/// Size of the first (and only) block in bytes.
	pub first_block_size: u32,
	/// SHA-1 of the first block, to be stored in the Normal FileFormatInfo blob.
	pub first_block_hash: Sha1Hash,
	/// LZX window size in bytes (echoed back for convenience when synthesizing
	/// the FileFormatInfo blob).
	pub window_size: u32,
}

const BLOCK_HEADER_SIZE: usize = 24;

/// Compress `pe` using LZX with the given window size (in bytes). `window_size`
/// must be one of the LZX-legal values (0x8000..=0x200000 in powers of two).
pub fn compress_normal(pe: &[u8], window_size: u32) -> Result<CompressedStream> {
	let lzxc_window = map_window_size(window_size)?;

	// Block header placeholder: next_block_size = 0, next_block_hash = 0 (this
	// is the terminal block in a single-block stream).
	let mut block = Vec::with_capacity(BLOCK_HEADER_SIZE + pe.len() / 2);
	block.resize(BLOCK_HEADER_SIZE, 0);

	// `EncoderWriter` emits `u16 BE chunk_size | chunk` per 32 KB slab, which
	// is exactly the XEX per-chunk framing. We append a `u16 0` terminator
	// ourselves so `decompress_normal` exits the chunk loop.
	{
		let mut writer = lzxc::EncoderWriter::new(&mut block, lzxc_window);
		writer.write_all(pe).map_err(Xex2Error::Io).map_err(IntoReport::into_report)?;
		writer.finish().map_err(Xex2Error::Io).map_err(IntoReport::into_report)?;
	}
	block.extend_from_slice(&0u16.to_be_bytes());

	let first_block_size = u32::try_from(block.len()).map_err(|_| Xex2Error::DecompressionFailed.into_report())?;
	let first_block_hash = sha1_of(&block);

	Ok(CompressedStream { data: block, first_block_size, first_block_hash, window_size })
}

/// Synthesize the FileFormatInfo optional-header blob for a Normal-compressed
/// stream. Layout:
///
///   u32 info_size | u16 encryption_type | u16 compression_type=Normal
///   | u32 window_size | u32 first_block_size | [u8; 20] first_block_hash
pub fn file_format_info_blob_normal(
	encryption_type: crate::header::EncryptionType,
	stream: &CompressedStream,
) -> Vec<u8> {
	let mut blob = Vec::with_capacity(36);
	blob.extend_from_slice(&36u32.to_be_bytes());
	blob.extend_from_slice(&(encryption_type as u16).to_be_bytes());
	blob.extend_from_slice(&(crate::header::CompressionType::Normal as u16).to_be_bytes());
	blob.extend_from_slice(&stream.window_size.to_be_bytes());
	blob.extend_from_slice(&stream.first_block_size.to_be_bytes());
	blob.extend_from_slice(&*stream.first_block_hash);
	blob
}

/// Synthesize the FileFormatInfo optional-header blob for an uncompressed
/// stream. Layout: `u32 info_size | u16 encryption_type | u16 compression_type=None`.
pub fn file_format_info_blob_none(encryption_type: crate::header::EncryptionType) -> Vec<u8> {
	let mut blob = Vec::with_capacity(8);
	blob.extend_from_slice(&8u32.to_be_bytes());
	blob.extend_from_slice(&(encryption_type as u16).to_be_bytes());
	blob.extend_from_slice(&(crate::header::CompressionType::None as u16).to_be_bytes());
	blob
}

fn sha1_of(bytes: &[u8]) -> Sha1Hash {
	let mut hasher = sha1::Sha1::new();
	hasher.update(bytes);
	Sha1Hash(hasher.finalize().into())
}

fn map_window_size(bytes: u32) -> Result<lzxc::WindowSize> {
	match bytes {
		0x8000 => Ok(lzxc::WindowSize::KB32),
		0x10000 => Ok(lzxc::WindowSize::KB64),
		0x20000 => Ok(lzxc::WindowSize::KB128),
		0x40000 => Ok(lzxc::WindowSize::KB256),
		0x80000 => Ok(lzxc::WindowSize::KB512),
		0x100000 => Ok(lzxc::WindowSize::MB1),
		0x200000 => Ok(lzxc::WindowSize::MB2),
		_ => Err(Xex2Error::DecompressionFailed.into_report()),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn round_trip_through_lzxd_single_chunk() {
		let pe = b"MZ\x90\x00".repeat(1000);
		let stream = compress_normal(&pe, 0x10000).unwrap();
		let decoded = decompress_via_lzxd(&stream, pe.len(), 0x10000);
		assert_eq!(decoded, pe);
	}

	#[test]
	fn round_trip_through_lzxd_multi_chunk() {
		let pe: Vec<u8> = (0..200_000).map(|i| (i & 0xFF) as u8).collect();
		let stream = compress_normal(&pe, 0x10000).unwrap();
		let decoded = decompress_via_lzxd(&stream, pe.len(), 0x10000);
		assert_eq!(decoded, pe);
	}

	#[test]
	fn hash_matches_sha1_of_block() {
		let pe = vec![0u8; 4096];
		let stream = compress_normal(&pe, 0x10000).unwrap();
		let expected = sha1_of(&stream.data);
		assert_eq!(stream.first_block_hash, expected);
	}

	fn decompress_via_lzxd(stream: &CompressedStream, image_size: usize, window: u32) -> Vec<u8> {
		// Re-enter through the decode path to confirm the stream is a valid
		// XEX "Normal" block. This is a trimmed clone of the loop in
		// `basefile::decompress_normal` (single-block).
		let window_size = match window {
			0x8000 => lzxd::WindowSize::KB32,
			0x10000 => lzxd::WindowSize::KB64,
			_ => unreachable!(),
		};
		let mut lzx = lzxd::Lzxd::new(window_size);
		let mut output = Vec::with_capacity(image_size);
		let compressed_payload = &stream.data[BLOCK_HEADER_SIZE..];
		let mut p = 0;
		while p < compressed_payload.len() && output.len() < image_size {
			let size = u16::from_be_bytes([compressed_payload[p], compressed_payload[p + 1]]) as usize;
			p += 2;
			if size == 0 {
				break;
			}
			let chunk = &compressed_payload[p..p + size];
			let remaining = image_size - output.len();
			let out_size = remaining.min(lzxc::MAX_CHUNK_SIZE);
			let decoded = lzx.decompress_next(chunk, out_size).unwrap();
			output.extend_from_slice(decoded);
			p += size;
		}
		output
	}
}
