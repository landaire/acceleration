use lzxc::Encoder;
use lzxc::MAX_CHUNK_SIZE;
use lzxc::WindowSize as EncoderWindowSize;
use lzxd::Lzxd;
use lzxd::WindowSize as DecoderWindowSize;
use sha1::Digest;
use sha1::Sha1;
use thiserror::Error;

pub const HYPERVISOR_SIZE: usize = 0x40000;
pub const KERNEL_SIZE: usize = 0x240000;
pub const PAYLOAD_SIZE: usize = HYPERVISOR_SIZE + KERNEL_SIZE;

const STAGE_HEADER_SIZE: usize = 0x20;
const SE_STREAM_OFFSET: usize = 0x30;
const NONCE_OFFSET: usize = 0x10;
const NONCE_SIZE: usize = 0x10;
const SE_OUTPUT_SIZE_OFFSET: usize = 0x28;
const FRAME_HEADER_SIZE: usize = 4;

#[derive(Debug, Error)]
pub enum Error {
	#[error("boot stage {stage} is shorter than its 0x20-byte cryptographic header")]
	StageHeaderTruncated { stage: &'static str },
	#[error("SE is shorter than its 0x30-byte compression header")]
	SeHeaderTruncated,
	#[error("SE declares a {actual:#x}-byte output; expected {expected:#x}")]
	UnexpectedOutputSize { expected: usize, actual: usize },
	#[error("SE compression frame header at {offset:#x} is truncated")]
	FrameHeaderTruncated { offset: usize },
	#[error("SE compression frame at {offset:#x} has invalid sizes compressed={compressed:#x}, output={output:#x}")]
	InvalidFrameSize { offset: usize, compressed: usize, output: usize },
	#[error("SE compression frame at {offset:#x} extends past the stage")]
	FrameTruncated { offset: usize },
	#[error("SE LZX frame {index} failed to decompress")]
	DecompressionFailed { index: usize },
	#[error("SE decompression produced {actual:#x} bytes; expected {expected:#x}")]
	DecompressedSize { expected: usize, actual: usize },
	#[error("SE contains {trailing:#x} trailing bytes after the compressed payload")]
	TrailingData { trailing: usize },
	#[error("SE input payload has size {actual:#x}; expected {expected:#x}")]
	PayloadSize { expected: usize, actual: usize },
	#[error("LZX chunk {index} compressed to {size:#x} bytes, which exceeds the SE u16 frame limit")]
	CompressedChunkTooLarge { index: usize, size: usize },
	#[error("encoded SE size {size:#x} exceeds the u32 stage header limit")]
	StageTooLarge { size: usize },
}

pub fn decode(sc: &[u8], sd: &[u8], se: &[u8]) -> Result<Vec<u8>, Error> {
	validate_stage_header("SC", sc)?;
	validate_stage_header("SD", sd)?;
	validate_stage_header("SE", se)?;
	if se.len() < SE_STREAM_OFFSET {
		return Err(Error::SeHeaderTruncated);
	}

	let key = derive_key_chain([sc, sd, se])?;
	let mut plaintext = se.to_vec();
	rc4(&key, &mut plaintext[STAGE_HEADER_SIZE..]);

	let output_size = u32::from_be_bytes(
		plaintext[SE_OUTPUT_SIZE_OFFSET..SE_OUTPUT_SIZE_OFFSET + 4].try_into().expect("fixed SE output-size field"),
	) as usize;
	if output_size != PAYLOAD_SIZE {
		return Err(Error::UnexpectedOutputSize { expected: PAYLOAD_SIZE, actual: output_size });
	}

	let mut decoder = Lzxd::new(DecoderWindowSize::KB128);
	let mut output = Vec::with_capacity(output_size);
	let mut offset = SE_STREAM_OFFSET;
	let mut frame_index = 0usize;
	while output.len() < output_size {
		let header = plaintext.get(offset..offset + FRAME_HEADER_SIZE).ok_or(Error::FrameHeaderTruncated { offset })?;
		let compressed_size = u16::from_be_bytes([header[0], header[1]]) as usize;
		let frame_output_size = u16::from_be_bytes([header[2], header[3]]) as usize;
		if compressed_size == 0
			|| frame_output_size == 0
			|| frame_output_size > MAX_CHUNK_SIZE
			|| output.len() + frame_output_size > output_size
		{
			return Err(Error::InvalidFrameSize { offset, compressed: compressed_size, output: frame_output_size });
		}
		offset += FRAME_HEADER_SIZE;
		let frame = plaintext
			.get(offset..offset + compressed_size)
			.ok_or(Error::FrameTruncated { offset: offset - FRAME_HEADER_SIZE })?;
		let decompressed = decoder
			.decompress_next(frame, frame_output_size)
			.map_err(|_| Error::DecompressionFailed { index: frame_index })?;
		if decompressed.len() != frame_output_size {
			return Err(Error::DecompressedSize { expected: frame_output_size, actual: decompressed.len() });
		}
		output.extend_from_slice(decompressed);
		offset += compressed_size;
		frame_index += 1;
	}
	if output.len() != output_size {
		return Err(Error::DecompressedSize { expected: output_size, actual: output.len() });
	}
	if offset != plaintext.len() {
		return Err(Error::TrailingData { trailing: plaintext.len() - offset });
	}
	Ok(output)
}

pub fn encode(sc: &[u8], sd: &[u8], se_template: &[u8], payload: &[u8]) -> Result<Vec<u8>, Error> {
	validate_stage_header("SC", sc)?;
	validate_stage_header("SD", sd)?;
	validate_stage_header("SE", se_template)?;
	if se_template.len() < SE_STREAM_OFFSET {
		return Err(Error::SeHeaderTruncated);
	}
	if payload.len() != PAYLOAD_SIZE {
		return Err(Error::PayloadSize { expected: PAYLOAD_SIZE, actual: payload.len() });
	}

	let mut stage = Vec::with_capacity(se_template.len());
	stage.extend_from_slice(&se_template[..STAGE_HEADER_SIZE]);
	stage.resize(SE_STREAM_OFFSET, 0);
	stage[SE_OUTPUT_SIZE_OFFSET..SE_OUTPUT_SIZE_OFFSET + 4].copy_from_slice(&(PAYLOAD_SIZE as u32).to_be_bytes());

	let mut encoder = Encoder::new(EncoderWindowSize::KB128);
	for (index, chunk) in payload.chunks(MAX_CHUNK_SIZE).enumerate() {
		let compressed = encoder.encode_chunk(chunk);
		let compressed_size = u16::try_from(compressed.len())
			.map_err(|_| Error::CompressedChunkTooLarge { index, size: compressed.len() })?;
		let output_size =
			u16::try_from(chunk.len()).expect("LZX maximum input chunk fits the SE u16 output-size field");
		stage.extend_from_slice(&compressed_size.to_be_bytes());
		stage.extend_from_slice(&output_size.to_be_bytes());
		stage.extend_from_slice(&compressed);
	}

	let stage_size = u32::try_from(stage.len()).map_err(|_| Error::StageTooLarge { size: stage.len() })?;
	stage[0x0c..0x10].copy_from_slice(&stage_size.to_be_bytes());
	let key = derive_key_chain([sc, sd, stage.as_slice()])?;
	rc4(&key, &mut stage[STAGE_HEADER_SIZE..]);
	Ok(stage)
}

fn validate_stage_header(stage: &'static str, bytes: &[u8]) -> Result<(), Error> {
	if bytes.len() < STAGE_HEADER_SIZE {
		return Err(Error::StageHeaderTruncated { stage });
	}
	Ok(())
}

fn derive_key_chain<const N: usize>(stages: [&[u8]; N]) -> Result<[u8; 16], Error> {
	let mut state = [0u8; 16];
	for (index, stage) in stages.into_iter().enumerate() {
		let stage_name = match index {
			0 => "SC",
			1 => "SD",
			_ => "SE",
		};
		validate_stage_header(stage_name, stage)?;
		let digest = hmac_sha1(&state, &stage[NONCE_OFFSET..NONCE_OFFSET + NONCE_SIZE]);
		state.copy_from_slice(&digest[..16]);
	}
	Ok(state)
}

pub(crate) fn sc_key_state(sc: &[u8]) -> Result<[u8; 16], Error> {
	derive_key_chain([sc])
}

pub(crate) fn sd_key_state(sc: &[u8], sd: &[u8]) -> Result<[u8; 16], Error> {
	derive_key_chain([sc, sd])
}

fn hmac_sha1(key: &[u8], data: &[u8]) -> [u8; 20] {
	const BLOCK_SIZE: usize = 64;
	let mut key_block = [0u8; BLOCK_SIZE];
	if key.len() > BLOCK_SIZE {
		let digest = Sha1::digest(key);
		key_block[..digest.len()].copy_from_slice(&digest);
	} else {
		key_block[..key.len()].copy_from_slice(key);
	}

	let mut inner_pad = [0x36u8; BLOCK_SIZE];
	let mut outer_pad = [0x5cu8; BLOCK_SIZE];
	for index in 0..BLOCK_SIZE {
		inner_pad[index] ^= key_block[index];
		outer_pad[index] ^= key_block[index];
	}

	let mut inner = Sha1::new();
	inner.update(inner_pad);
	inner.update(data);
	let inner_digest = inner.finalize();

	let mut outer = Sha1::new();
	outer.update(outer_pad);
	outer.update(inner_digest);
	outer.finalize().into()
}

fn rc4(key: &[u8], bytes: &mut [u8]) {
	let mut state = core::array::from_fn::<u8, 256, _>(|index| index as u8);
	let mut j = 0u8;
	for i in 0..state.len() {
		j = j.wrapping_add(state[i]).wrapping_add(key[i % key.len()]);
		state.swap(i, j as usize);
	}

	let mut i = 0u8;
	j = 0;
	for byte in bytes {
		i = i.wrapping_add(1);
		j = j.wrapping_add(state[i as usize]);
		state.swap(i as usize, j as usize);
		let key_byte = state[state[i as usize].wrapping_add(state[j as usize]) as usize];
		*byte ^= key_byte;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn hmac_chain_matches_deanxbox_stage_keys() {
		let mut sc = [0u8; STAGE_HEADER_SIZE];
		sc[NONCE_OFFSET..].copy_from_slice(&hex::<NONCE_SIZE>("5145e4b206bf24db95c9986a700531fe"));
		let mut sd = [0u8; STAGE_HEADER_SIZE];
		sd[NONCE_OFFSET..].copy_from_slice(&hex::<NONCE_SIZE>("0d997d744eebd4e0bbee1b37cb0b2244"));
		let mut se = [0u8; STAGE_HEADER_SIZE];
		se[NONCE_OFFSET..].copy_from_slice(&hex::<NONCE_SIZE>("44678c874924cb86dae7abe047fdf74f"));

		assert_eq!(derive_key_chain([sc.as_slice()]).unwrap(), hex("e0fce3a79ae5465dc613750403a8321a"));
		assert_eq!(derive_key_chain([sc.as_slice(), sd.as_slice()]).unwrap(), hex("d1fd2c1f84d684cdd63426fabcdaef5d"));
		assert_eq!(
			derive_key_chain([sc.as_slice(), sd.as_slice(), se.as_slice()]).unwrap(),
			hex("a47ba40e6c538cc2ebf000cb2f76d6f5")
		);
	}

	#[test]
	fn generated_stage_decodes_to_exact_payload() {
		let sc = stage(*b"SC", "5145e4b206bf24db95c9986a700531fe");
		let sd = stage(*b"SD", "0d997d744eebd4e0bbee1b37cb0b2244");
		let se = stage(*b"SE", "44678c874924cb86dae7abe047fdf74f");
		let mut payload = vec![0u8; PAYLOAD_SIZE];
		for offset in (0..payload.len()).step_by(4093) {
			payload[offset] = (offset >> 8) as u8;
		}

		let encoded = encode(&sc, &sd, &se, &payload).unwrap();
		assert_eq!(decode(&sc, &sd, &encoded).unwrap(), payload);
	}

	fn stage(name: [u8; 2], nonce: &str) -> Vec<u8> {
		let mut stage = vec![0u8; SE_STREAM_OFFSET];
		let stage_size = stage.len() as u32;
		stage[..2].copy_from_slice(&name);
		stage[0x0c..0x10].copy_from_slice(&stage_size.to_be_bytes());
		stage[NONCE_OFFSET..NONCE_OFFSET + NONCE_SIZE].copy_from_slice(&hex::<NONCE_SIZE>(nonce));
		stage
	}

	fn hex<const N: usize>(value: &str) -> [u8; N] {
		assert_eq!(value.len(), N * 2);
		let mut bytes = [0u8; N];
		for index in 0..N {
			bytes[index] = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
		}
		bytes
	}
}
