use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use std::io::Cursor;

use crate::crypto;
use crate::error::IoResultExt;
use crate::error::Result;
use crate::error::Xex2Error;
use crate::header::CompressionType;
use crate::header::EncryptionType;
use crate::header::FileFormatInfo;
use crate::header::SecurityInfo;
use crate::header::Xex2Header;
use rootcause::IntoReport;

pub fn extract_basefile(data: &[u8], header: &Xex2Header, security_info: &SecurityInfo) -> Result<Vec<u8>> {
	let file_format = header.file_format_info(data)?;
	let encrypted_data = &data[header.data_offset as usize..];

	let (retail_key, devkit_key) = crypto::decrypt_file_key(&security_info.image_info.file_key);

	let decrypted = match file_format.encryption_type {
		EncryptionType::None => encrypted_data.to_vec(),
		EncryptionType::Normal => {
			let key =
				if try_decrypt_with_key(encrypted_data, &retail_key, &file_format) { retail_key } else { devkit_key };
			crypto::decrypt_data(encrypted_data, &key)
		}
	};

	match file_format.compression_type {
		CompressionType::None => Ok(decrypted),
		CompressionType::Basic => decompress_basic(&decrypted, &file_format),
		CompressionType::Normal => decompress_normal(&decrypted, security_info.image_size, &file_format),
		CompressionType::Delta => Err(Xex2Error::DecompressionFailed.into_report()),
	}
}

fn try_decrypt_with_key(data: &[u8], key: &[u8; 16], format: &FileFormatInfo) -> bool {
	if data.len() < 32 {
		return false;
	}

	match format.compression_type {
		CompressionType::Normal => {
			let first_block = crypto::decrypt_data(&data[..32], key);
			let mut c = Cursor::new(&first_block);
			if let Ok(block_size) = c.read_u32::<BigEndian>() {
				block_size > 0 && block_size < 0x100000
			} else {
				false
			}
		}
		CompressionType::Basic | CompressionType::None => {
			let first_block = crypto::decrypt_data(&data[..16], key);
			first_block[0] == b'M' && first_block[1] == b'Z'
		}
		CompressionType::Delta => false,
	}
}

fn decompress_basic(data: &[u8], format: &FileFormatInfo) -> Result<Vec<u8>> {
	let mut output = Vec::new();
	let mut offset = 0;

	for block in &format.blocks {
		let data_size = block.data_size as usize;
		let zero_size = block.zero_size as usize;

		if offset + data_size > data.len() {
			return Err(Xex2Error::DecompressionFailed.into_report());
		}

		output.extend_from_slice(&data[offset..offset + data_size]);
		output.resize(output.len() + zero_size, 0);
		offset += data_size;
	}

	Ok(output)
}

fn decompress_normal(data: &[u8], image_size: u32, format: &FileFormatInfo) -> Result<Vec<u8>> {
	let mut output = Vec::new();
	let window_size = window_size_from_format(format);
	let first_chunk_size =
		format.first_block_size.ok_or_else(|| Xex2Error::DecompressionFailed.into_report())? as usize;

	let mut block_offset = 0;
	let mut block_size = first_chunk_size;
	let mut lzx = lzxd::Lzxd::new(window_size);

	while block_size > 0 && output.len() < image_size as usize {
		if block_offset + block_size > data.len() {
			return Err(Xex2Error::DecompressionFailed.into_report());
		}

		let block_data = &data[block_offset..block_offset + block_size];

		let next_block_size = read_u32_be(block_data)? as usize;

		let compressed_payload = &block_data[24..];
		let mut payload_offset = 0;

		while payload_offset < compressed_payload.len() && output.len() < image_size as usize {
			if payload_offset + 2 > compressed_payload.len() {
				break;
			}

			let chunk_compressed_size =
				((compressed_payload[payload_offset] as usize) << 8) | compressed_payload[payload_offset + 1] as usize;
			payload_offset += 2;

			if chunk_compressed_size == 0 {
				break;
			}

			if payload_offset + chunk_compressed_size > compressed_payload.len() {
				return Err(Xex2Error::DecompressionFailed.into_report());
			}

			let chunk_data = &compressed_payload[payload_offset..payload_offset + chunk_compressed_size];
			let remaining = image_size as usize - output.len();
			let out_size = std::cmp::min(remaining, 0x8000);

			match lzx.decompress_next(chunk_data, out_size) {
				Ok(decompressed) => {
					output.extend_from_slice(decompressed);
				}
				Err(_) => {
					return Err(Xex2Error::DecompressionFailed.into_report());
				}
			}

			payload_offset += chunk_compressed_size;
		}

		block_offset += block_size;
		block_size = next_block_size;
	}

	output.resize(image_size as usize, 0);
	Ok(output)
}

fn window_size_from_format(format: &FileFormatInfo) -> lzxd::WindowSize {
	match format.window_size.unwrap_or(0x20000) {
		0x8000 => lzxd::WindowSize::KB32,
		0x10000 => lzxd::WindowSize::KB64,
		0x20000 => lzxd::WindowSize::KB128,
		0x40000 => lzxd::WindowSize::KB256,
		0x80000 => lzxd::WindowSize::KB512,
		0x100000 => lzxd::WindowSize::MB1,
		0x200000 => lzxd::WindowSize::MB2,
		_ => lzxd::WindowSize::KB64,
	}
}

fn read_u32_be(data: &[u8]) -> Result<u32> {
	let mut c = Cursor::new(data);
	c.read_u32::<BigEndian>().io()
}
