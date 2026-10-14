//! LZX compression of a PE image into an XEX "Normal" data region.
//!
//! Inverse of [`crate::basefile::decompress_normal`]. The region is a chain of
//! blocks:
//!
//!   [u32 next_block_size][u8; 20 SHA-1 of next block]
//!   [u16 BE chunk_size][lzx_data] ...
//!   [u16 BE 0]   (terminator)
//!   zero padding to the block size
//!
//! The last block has a zero `next_block_size` and hash. The LZX encoder state
//! persists across chunks and blocks, matching the decode side.
//!
//! The console loader reads each block with unbuffered I/O into one of two
//! 64 KB buffers, so a block never exceeds 64 KB and its size is a multiple of
//! the 2 KB DVD sector.

use crate::error::Result;
use crate::error::Xex2Error;
use crate::header::Sha1Hash;
use rootcause::IntoReport;
use sha1::Digest;

/// A compressed data region and the integrity metadata the XEX header needs.
pub struct CompressedStream {
	/// The data region, ready to drop in at `Xex2Header::data_offset`.
	pub data: Vec<u8>,
	/// Size of the first block in bytes.
	pub first_block_size: u32,
	/// SHA-1 of the first block, to be stored in the Normal FileFormatInfo blob.
	pub first_block_hash: Sha1Hash,
	/// LZX window size in bytes (echoed back for convenience when synthesizing
	/// the FileFormatInfo blob).
	pub window_size: u32,
}

const BLOCK_HEADER_SIZE: usize = 24;
const CHUNK_SIZE_PREFIX_LEN: usize = 2;
const MAX_BLOCK_SIZE: usize = 0x10000;
const BLOCK_ALIGNMENT: usize = 0x800;

/// Compress `pe` using LZX with the given window size (in bytes). `window_size`
/// must be one of the LZX-legal values (0x8000..=0x200000 in powers of two).
pub fn compress_normal(pe: &[u8], window_size: u32) -> Result<CompressedStream> {
	let mut encoder = lzxc::Encoder::new(map_window_size(window_size)?);

	let mut blocks = Vec::new();
	let mut block = vec![0u8; BLOCK_HEADER_SIZE];
	for slab in pe.chunks(lzxc::MAX_CHUNK_SIZE) {
		let chunk = encoder.encode_chunk(slab);
		let framed = CHUNK_SIZE_PREFIX_LEN + chunk.len();
		if BLOCK_HEADER_SIZE + framed + CHUNK_SIZE_PREFIX_LEN > MAX_BLOCK_SIZE {
			return Err(Xex2Error::CompressedChunkTooLarge { size: chunk.len() }.into_report());
		}
		if block.len() + framed + CHUNK_SIZE_PREFIX_LEN > MAX_BLOCK_SIZE {
			blocks.push(finish_block(block));
			block = vec![0u8; BLOCK_HEADER_SIZE];
		}
		// The size check above bounds `chunk.len()` below 64 KB.
		block.extend_from_slice(&(chunk.len() as u16).to_be_bytes());
		block.extend_from_slice(&chunk);
	}
	blocks.push(finish_block(block));

	// Each header names the following block, so the chain is built from the
	// end. The last block keeps a zero size and hash.
	let mut next_size = 0u32;
	let mut next_hash = Sha1Hash::ZERO;
	for block in blocks.iter_mut().rev() {
		block[..4].copy_from_slice(&next_size.to_be_bytes());
		block[4..BLOCK_HEADER_SIZE].copy_from_slice(&*next_hash);
		// Blocks are at most `MAX_BLOCK_SIZE` bytes.
		next_size = block.len() as u32;
		next_hash = sha1_of(block);
	}

	Ok(CompressedStream {
		data: blocks.concat(),
		first_block_size: next_size,
		first_block_hash: next_hash,
		window_size,
	})
}

fn finish_block(mut block: Vec<u8>) -> Vec<u8> {
	block.extend_from_slice(&0u16.to_be_bytes());
	block.resize(block.len().next_multiple_of(BLOCK_ALIGNMENT), 0);
	block
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
	use crate::header::CompressionType;
	use crate::header::EncryptionType;
	use crate::header::FileFormatInfo;

	const WINDOW: u32 = 0x10000;

	/// Bytes that LZX cannot shrink, so the stream spans several blocks.
	fn incompressible(len: usize) -> Vec<u8> {
		let mut state = 0x1234_5678u32;
		(0..len)
			.map(|_| {
				state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
				(state >> 24) as u8
			})
			.collect()
	}

	fn decode(stream: &CompressedStream, image_size: usize) -> Vec<u8> {
		let format = FileFormatInfo {
			encryption_type: EncryptionType::None,
			compression_type: CompressionType::Normal,
			blocks: Vec::new(),
			window_size: Some(stream.window_size),
			first_block_size: Some(stream.first_block_size),
		};
		crate::basefile::decompress_normal(&stream.data, image_size as u32, &format).unwrap()
	}

	/// Walk the chain from the FileFormatInfo values and return each block size.
	fn chain_block_sizes(stream: &CompressedStream) -> Vec<usize> {
		let mut sizes = Vec::new();
		let mut offset = 0;
		let mut size = stream.first_block_size as usize;
		let mut hash = stream.first_block_hash;
		while size != 0 {
			let block = &stream.data[offset..offset + size];
			assert_eq!(sha1_of(block), hash, "block at {offset:#x} does not match the hash naming it");
			sizes.push(size);
			size = u32::from_be_bytes(block[..4].try_into().unwrap()) as usize;
			hash = Sha1Hash(block[4..BLOCK_HEADER_SIZE].try_into().unwrap());
			offset += block.len();
		}
		assert_eq!(offset, stream.data.len(), "chain does not cover the data region");
		sizes
	}

	#[test]
	fn small_image_is_one_sector_aligned_block() {
		let pe = b"MZ\x90\x00".repeat(1000);
		let stream = compress_normal(&pe, WINDOW).unwrap();
		assert_eq!(chain_block_sizes(&stream), [BLOCK_ALIGNMENT]);
		assert_eq!(decode(&stream, pe.len()), pe);
	}

	#[test]
	fn large_image_splits_into_bounded_sector_aligned_blocks() {
		let pe = incompressible(0x50000);
		let stream = compress_normal(&pe, WINDOW).unwrap();
		let sizes = chain_block_sizes(&stream);
		assert!(sizes.len() > 1);
		for size in sizes {
			assert!(size <= MAX_BLOCK_SIZE);
			assert_eq!(size % BLOCK_ALIGNMENT, 0);
		}
		assert_eq!(decode(&stream, pe.len()), pe);
	}
}
