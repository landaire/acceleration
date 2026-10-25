//! Basefile (loaded image) extraction.
//!
//! XEX files wrap a Windows PE image. The image data may be AES-128-CBC
//! encrypted with a per-file session key, and stored as Basic runs (data
//! followed by zero fill) or as a Normal LZX block chain. This module loads
//! the image the way the kernel loader does (`XexpLoadRawImage` at kernel
//! 21256 `0x800a5610` and `XexpLoadCompressedImage` at `0x800a5938`),
//! including the SHA-1 check of every Normal block.
//!
//! The primary entry point is [`Xex2::extract_basefile`][crate::Xex2::extract_basefile].

use std::ops::Range;

use rootcause::IntoReport;
use sha1::Digest;
use sha1::Sha1;

use crate::Xex2;
use crate::crypto;
use crate::crypto::CbcDecryptor;
use crate::error::Result;
use crate::error::Xex2Error;
use crate::header::AesKey;
use crate::header::CompressionType;
use crate::header::EncryptionType;
use crate::header::FileFormatInfo;
use crate::header::Sha1Hash;
use crate::header::Xex2Header;
use crate::writer::TargetMachine;

/// The loader decompresses Normal data in frames of at most this size.
const LZX_FRAME_SIZE: u32 = 0x8000;
/// Next block size and SHA-1 that begin every Normal block.
const BLOCK_HEADER_SIZE: usize = 24;
/// The DVD sector: the unit of every read the loader makes from a file.
const SECTOR_SIZE: u32 = 0x800;
/// Size of the loader's read buffers.
const MAX_BLOCK_SIZE: u32 = 0x1_0000;

/// Load the image with the checks the console makes: the header signature
/// and hash, which also identify the console type and so the master key;
/// the SHA-1 of every Normal block; and the page descriptors.
pub(crate) fn extract_basefile(data: &[u8], xex: &Xex2) -> Result<Vec<u8>> {
	let machine = crate::hashes::verify_headers(data, xex)?;
	let image = extract_basefile_unchecked(data, xex, machine)?;
	crate::page_descriptors::verify_chain(&image, &xex.header, &xex.security_info, data)?;

	Ok(image)
}

/// Load the image without checking the headers or page descriptors. The
/// caller names the console type whose master key wraps the file key. The
/// SHA-1 of every Normal block is still checked, because the block chain
/// depends on it.
pub(crate) fn extract_basefile_unchecked(data: &[u8], xex: &Xex2, machine: TargetMachine) -> Result<Vec<u8>> {
	let format = xex.header.file_format_info()?;
	let key = session_key(xex, &format, machine)?;
	let image_size = xex.security_info.image_size;
	let mut image = allocate(image_size as usize)?;
	load_image(image_data(data, &xex.header)?, &format, key, image_size, 0..image_size, &mut image)?;

	Ok(image)
}

/// The session key that decrypts the image data on `machine`, or `None` for
/// unencrypted data.
pub(crate) fn session_key(xex: &Xex2, format: &FileFormatInfo, machine: TargetMachine) -> Result<Option<AesKey>> {
	if format.encryption_type == EncryptionType::None {
		return Ok(None);
	}

	if format.compression_type == CompressionType::Delta {
		return Err(Xex2Error::DeltaPatchNeedsBase.into_report());
	}

	let image_flags = xex.security_info.image_info.image_flags;
	let master = crypto::master_key(machine, image_flags)
		.ok_or_else(|| Xex2Error::UnknownMasterKey { machine, image_flags: image_flags.bits() }.into_report())?;

	Ok(Some(crypto::unwrap_key(&xex.security_info.image_info.file_key, &master)))
}

/// A zeroed buffer, failing with an error instead of aborting when memory is
/// short.
pub(crate) fn allocate(size: usize) -> Result<Vec<u8>> {
	let mut buffer = Vec::new();
	buffer.try_reserve_exact(size).map_err(|_| Xex2Error::AllocationFailed { size }.into_report())?;
	buffer.resize(size, 0);

	Ok(buffer)
}

/// The image data region: everything after the headers.
pub(crate) fn image_data<'a>(data: &'a [u8], header: &Xex2Header) -> Result<&'a [u8]> {
	data.get(header.data_offset as usize..).ok_or_else(|| {
		Xex2Error::InvalidHeaderOffset { offset: header.data_offset, file_size: data.len() }.into_report()
	})
}

fn first_block(format: &FileFormatInfo) -> Result<(u32, Sha1Hash)> {
	match (format.first_block_size, format.first_block_hash) {
		(Some(size), Some(hash)) => Ok((size, hash)),
		_ => Err(Xex2Error::InvalidFileFormatInfo.into_report()),
	}
}

/// Load bytes `range` of an image of `image_size` bytes into `out`, which
/// holds `range.len()` zero bytes. Basic and raw data zero-fill the image
/// past their data; Normal data leaves bytes past the end of its stream
/// unwritten, as the loader does.
pub(crate) fn load_image(
	data: &[u8],
	format: &FileFormatInfo,
	key: Option<AesKey>,
	image_size: u32,
	range: Range<u32>,
	out: &mut [u8],
) -> Result<()> {
	if range.start > range.end || range.end > image_size || out.len() != (range.end - range.start) as usize {
		return Err(Xex2Error::InvalidImageRange { start: range.start, end: range.end, image_size }.into_report());
	}

	let decryptor = key.map(CbcDecryptor::new);
	match format.compression_type {
		CompressionType::None => load_raw(data, decryptor, range, out),
		CompressionType::Basic => load_basic(data, format, decryptor, range, out),
		CompressionType::Normal => load_normal(data, format, decryptor, image_size, range, out),
		CompressionType::Delta => Err(Xex2Error::DeltaPatchNeedsBase.into_report()),
	}
}

/// Image data stored as is. The kernel loader rejects this format; it is
/// read for XEX files that tools write without compression.
fn load_raw(data: &[u8], decryptor: Option<CbcDecryptor>, range: Range<u32>, out: &mut [u8]) -> Result<()> {
	let available = data.len().min(range.end as usize);
	let mut plain = data[..available].to_vec();
	if let Some(mut decryptor) = decryptor {
		decryptor.decrypt(&mut plain);
	}

	copy_overlap(&plain, 0, &range, out);

	Ok(())
}

/// Basic runs: each run's data then its zero fill, ending with the run whose
/// zero fill is zero. The image past the last run is zero.
fn load_basic(
	data: &[u8],
	format: &FileFormatInfo,
	mut decryptor: Option<CbcDecryptor>,
	range: Range<u32>,
	out: &mut [u8],
) -> Result<()> {
	if format.blocks.last().is_none_or(|run| run.zero_size != 0) {
		return Err(Xex2Error::InvalidFileFormatInfo.into_report());
	}

	let mut position = 0u32;
	let mut data_offset = 0usize;
	let mut run_buffer = Vec::new();
	for (run_index, run) in format.blocks.iter().enumerate() {
		if position >= range.end {
			break;
		}

		// Runs are read straight from the file in whole sectors (see
		// doc/re-notes/xex-loader.md, Basic image data).
		if !run.data_size.is_multiple_of(SECTOR_SIZE) {
			return Err(Xex2Error::BasicRunSize { run_index, size: run.data_size }.into_report());
		}

		let data_end = data_offset + run.data_size as usize;
		let Some(run_data) = data.get(data_offset..data_end) else {
			return Err(Xex2Error::DataBlockOutOfRange {
				block_index: run_index,
				offset: data_offset,
				size: run.data_size,
			}
			.into_report());
		};

		run_buffer.clear();
		run_buffer.extend_from_slice(run_data);
		if let Some(decryptor) = &mut decryptor {
			decryptor.decrypt(&mut run_buffer);
		}
		copy_overlap(&run_buffer, position, &range, out);
		position = position.saturating_add(run.data_size);
		data_offset = data_end;

		zero_overlap(position..position.saturating_add(run.zero_size), &range, out);
		position = position.saturating_add(run.zero_size);
		if run.zero_size == 0 {
			break;
		}
	}

	zero_overlap(position..range.end, &range, out);

	Ok(())
}

/// The Normal block chain: each block starts with the size and SHA-1 of the
/// next block, then LZX frames prefixed by big-endian 16-bit sizes and ended
/// by a zero size. Every frame but the last expands to 32 KiB.
fn load_normal(
	data: &[u8],
	format: &FileFormatInfo,
	decryptor: Option<CbcDecryptor>,
	image_size: u32,
	range: Range<u32>,
	out: &mut [u8],
) -> Result<()> {
	let window_size = format.window_size.ok_or_else(|| Xex2Error::InvalidFileFormatInfo.into_report())?;
	let mut decoder = xenon_lzx::Decoder::new(vec![0u8; window_size as usize], LZX_FRAME_SIZE as usize)
		.map_err(|e| Xex2Error::Lzx(e).into_report())?;
	let mut frame_buffer = vec![0u8; LZX_FRAME_SIZE as usize];

	let mut chain = BlockChain::new(data, format, decryptor)?;
	let mut position = 0u32;
	let mut image_remaining = image_size;
	while let Some((block_index, block)) = chain.next_block()? {
		let mut cursor = BLOCK_HEADER_SIZE;
		while let Some(size_bytes) = block.get(cursor..cursor + 2) {
			let frame_size = usize::from(u16::from_be_bytes([size_bytes[0], size_bytes[1]]));
			if frame_size == 0 {
				break;
			}

			if position >= range.end {
				return Ok(());
			}

			let Some(frame) = block.get(cursor + 2..cursor + 2 + frame_size) else {
				return Err(Xex2Error::LzxFrameOutOfRange { block_index, offset: cursor }.into_report());
			};

			let len = LZX_FRAME_SIZE.min(image_remaining);
			let frame_end = position + len;
			let destination = if position >= range.start && frame_end <= range.end {
				&mut out[(position - range.start) as usize..(frame_end - range.start) as usize]
			} else {
				&mut frame_buffer[..len as usize]
			};
			decoder.decompress(frame, destination).map_err(|e| Xex2Error::Lzx(e).into_report())?;
			if position < range.start || frame_end > range.end {
				copy_overlap(&frame_buffer[..len as usize], position, &range, out);
			}

			position = frame_end;
			image_remaining -= len;
			cursor += 2 + frame_size;
		}
	}

	Ok(())
}

/// Copy the part of `bytes`, which belong at image offset `position`, that
/// lies in `range`.
fn copy_overlap(bytes: &[u8], position: u32, range: &Range<u32>, out: &mut [u8]) {
	let start = position.max(range.start);
	let end = position.saturating_add(bytes.len() as u32).min(range.end);
	if start >= end {
		return;
	}

	out[(start - range.start) as usize..(end - range.start) as usize]
		.copy_from_slice(&bytes[(start - position) as usize..(end - position) as usize]);
}

fn zero_overlap(span: Range<u32>, range: &Range<u32>, out: &mut [u8]) {
	let start = span.start.max(range.start);
	let end = span.end.min(range.end);
	if start >= end {
		return;
	}

	out[(start - range.start) as usize..(end - range.start) as usize].fill(0);
}

/// Blocks of a Normal or Delta data region, decrypted and checked against
/// the SHA-1 that the previous block (or FileFormatInfo) records.
pub(crate) struct BlockChain<'a> {
	data: &'a [u8],
	decryptor: Option<CbcDecryptor>,
	offset: usize,
	next_size: u32,
	next_hash: Sha1Hash,
	index: usize,
	buffer: Vec<u8>,
}

impl<'a> BlockChain<'a> {
	pub(crate) fn new(data: &'a [u8], format: &FileFormatInfo, decryptor: Option<CbcDecryptor>) -> Result<Self> {
		let (next_size, next_hash) = first_block(format)?;

		Ok(Self { data, decryptor, offset: 0, next_size, next_hash, index: 0, buffer: Vec::new() })
	}

	/// The index and contents of the next decrypted block, or `None` after
	/// the block whose next size is zero.
	pub(crate) fn next_block(&mut self) -> Result<Option<(usize, &[u8])>> {
		if self.next_size == 0 {
			return Ok(None);
		}

		// Blocks are read into a 64 KiB buffer in whole sectors (see
		// doc/re-notes/xex-loader.md, Normal image data).
		if self.next_size > MAX_BLOCK_SIZE || !self.next_size.is_multiple_of(SECTOR_SIZE) {
			return Err(Xex2Error::DataBlockSize { block_index: self.index, size: self.next_size }.into_report());
		}

		let size = self.next_size as usize;
		let Some(encrypted) = self.data.get(self.offset..self.offset + size) else {
			return Err(Xex2Error::DataBlockOutOfRange {
				block_index: self.index,
				offset: self.offset,
				size: self.next_size,
			}
			.into_report());
		};

		self.buffer.clear();
		self.buffer.extend_from_slice(encrypted);
		if let Some(decryptor) = &mut self.decryptor {
			decryptor.decrypt(&mut self.buffer);
		}

		if Sha1Hash(Sha1::digest(&self.buffer).into()) != self.next_hash {
			return Err(Xex2Error::DataBlockHashMismatch { block_index: self.index }.into_report());
		}

		if self.buffer.len() < BLOCK_HEADER_SIZE {
			return Err(Xex2Error::DataBlockOutOfRange {
				block_index: self.index,
				offset: self.offset,
				size: self.next_size,
			}
			.into_report());
		}

		self.next_size = u32::from_be_bytes(self.buffer[0..4].try_into().unwrap());
		self.next_hash = Sha1Hash(self.buffer[4..BLOCK_HEADER_SIZE].try_into().unwrap());
		let index = self.index;
		self.offset += size;
		self.index += 1;

		Ok(Some((index, &self.buffer)))
	}
}
