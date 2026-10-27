use lzxc::Encoder;
use lzxc::WindowSize;
use rootcause::IntoReport;
use sha1::Digest;
use sha1::Sha1;
use xenon_types::AesKey;

use crate::Xex2;
use crate::crypto;
use crate::error::Result;
use crate::error::Xex2Error;
use crate::header::CompressionType;
use crate::header::EncryptionType;
use crate::header::OptionalHeaderKey;
use crate::header::Sha1Hash;
use crate::opt::ImageFlags;
use crate::opt::ModuleFlags;

const DELTA_WINDOW_SIZE: usize = 0x8000;
const DATA_BLOCK_LIMIT: usize = 0x1_0000;
const DATA_BLOCK_ALIGNMENT: usize = 0x800;
const DATA_BLOCK_HEADER_SIZE: usize = 24;
const SECURITY_INFO_SIZE: usize = 0x184;
const IMAGE_INFO_OFFSET: usize = 0x108;
const IMAGE_INFO_SIGNED_SIZE: usize = 0x74;
const DELTA_DESCRIPTOR_FIXED_SIZE: usize = 0x4C;
const RECORD_HEADER_SIZE: usize = 12;
const OPTIONAL_HEADER_COUNT: usize = 2;
const OPTIONAL_HEADER_INDEX_OFFSET: usize = 0x18;

/// Build a development-signed XEXP that reconstructs `target` from `base`.
///
/// The target headers are reproduced byte for byte. The target image starts
/// with the common prefix of the base image, then one delta record replaces
/// each changed 32 KiB region. Both inputs are verified before use.
pub fn create_delta_patch(base: &[u8], target: &[u8]) -> Result<Vec<u8>> {
	let base_xex = Xex2::parse(base)?;
	let target_xex = Xex2::parse(target)?;
	crate::hashes::check_file_layout(base, &base_xex)?;
	crate::hashes::check_file_layout(target, &target_xex)?;
	let base_machine = crate::hashes::verify_headers(base, &base_xex)?;
	let target_machine = crate::hashes::verify_headers(target, &target_xex)?;

	let rejected = ModuleFlags::PATCH | ModuleFlags::PATCH_FULL;
	if base_xex.header.module_flags.intersects(rejected) {
		return Err(Xex2Error::BaseHasPatchFlags { module_flags: base_xex.header.module_flags.bits() }.into_report());
	}

	let base_format = base_xex.header.file_format_info()?;
	if !matches!(base_format.compression_type, CompressionType::Basic | CompressionType::Normal) {
		return Err(Xex2Error::InvalidCompressionFormat(base_format.compression_type as u16).into_report());
	}

	let base_image = base_xex.extract_basefile(base)?;
	let target_image = target_xex.extract_basefile(target)?;
	let target_image_size = u32::try_from(target_image.len())
		.map_err(|_| Xex2Error::ImageSizeTooLarge { image_size: target_image.len() }.into_report())?;

	let base_headers = &base[..base_xex.header.data_offset as usize];
	let target_headers = &target[..target_xex.header.data_offset as usize];
	let header_copy_size = base_headers.len().min(target_headers.len());
	let image_copy_size = base_image.len().min(target_image.len());

	let header_records = replacement_records(target_headers, |offset| base_headers.get(offset).copied().unwrap_or(0))?;
	let image_records = replacement_records(&target_image, |offset| base_image.get(offset).copied().unwrap_or(0))?;
	let blocks = link_blocks(data_blocks(image_records)?);

	let target_session_key = session_key(&target_xex, target_machine)?;
	let base_image_key = match base_format.encryption_type {
		EncryptionType::None => AesKey([0u8; 16]),
		EncryptionType::Normal => {
			let base_session_key = session_key(&base_xex, base_machine)?;
			crypto::wrap_file_key(&base_session_key, &target_session_key)
		}
	};

	let descriptor = descriptor(
		&base_xex,
		&target_xex,
		base_image_key,
		target_headers.len() as u32,
		header_copy_size as u32,
		image_copy_size as u32,
		header_records,
	);
	let file_format = file_format(blocks.first_size, blocks.first_hash);
	build_patch_file(&target_xex, target_image_size, &file_format, &descriptor, &blocks.data)
}

fn session_key(xex: &Xex2, machine: crate::writer::TargetMachine) -> Result<AesKey> {
	let image_flags = xex.security_info.image_info.image_flags;
	let master = crypto::master_key(machine, image_flags)
		.ok_or_else(|| Xex2Error::UnknownMasterKey { machine, image_flags: image_flags.bits() }.into_report())?;
	Ok(crypto::unwrap_key(&xex.security_info.image_info.file_key, &master))
}

fn replacement_records<F>(target: &[u8], source_byte: F) -> Result<Vec<Vec<u8>>>
where
	F: Fn(usize) -> u8,
{
	let mut records = Vec::new();
	for (chunk_index, chunk) in target.chunks(DELTA_WINDOW_SIZE).enumerate() {
		let chunk_offset = chunk_index * DELTA_WINDOW_SIZE;
		let first = chunk.iter().enumerate().find(|(index, byte)| **byte != source_byte(chunk_offset + index));
		let Some((first, _)) = first else {
			continue;
		};
		let last = chunk
			.iter()
			.enumerate()
			.rfind(|(index, byte)| **byte != source_byte(chunk_offset + index))
			.map(|(index, _)| index + 1)
			.expect("a first difference has a last difference");
		let offset = chunk_offset + first;
		let bytes = &chunk[first..last];
		records.push(replacement_record(offset, bytes)?);
	}
	Ok(records)
}

fn replacement_record(offset: usize, bytes: &[u8]) -> Result<Vec<u8>> {
	let size = u16::try_from(bytes.len()).map_err(|_| Xex2Error::InvalidDeltaRecord { offset }.into_report())?;
	let mut record = Vec::with_capacity(RECORD_HEADER_SIZE + bytes.len());
	record.extend_from_slice(&(offset as u32).to_be_bytes());
	record.extend_from_slice(&(offset as u32).to_be_bytes());
	record.extend_from_slice(&size.to_be_bytes());

	if bytes.iter().all(|byte| *byte == 0) {
		record.extend_from_slice(&0u16.to_be_bytes());
		return Ok(record);
	}

	let compressed = Encoder::new(WindowSize::KB32).encode_chunk(bytes);
	let compressed_size = u16::try_from(compressed.len())
		.map_err(|_| Xex2Error::CompressedChunkTooLarge { size: compressed.len() }.into_report())?;
	if compressed_size <= 1 {
		return Err(Xex2Error::InvalidDeltaRecord { offset }.into_report());
	}
	record.extend_from_slice(&compressed_size.to_be_bytes());
	record.extend_from_slice(&compressed);
	Ok(record)
}

fn data_blocks(records: Vec<Vec<u8>>) -> Result<Vec<Vec<u8>>> {
	let mut blocks = Vec::new();
	let mut records = records.into_iter().peekable();
	while records.peek().is_some() {
		let mut block = vec![0u8; DATA_BLOCK_HEADER_SIZE];
		while let Some(record) = records.peek() {
			let finished_size =
				(block.len() + record.len() + RECORD_HEADER_SIZE).next_multiple_of(DATA_BLOCK_ALIGNMENT);
			if finished_size > DATA_BLOCK_LIMIT {
				break;
			}
			block.extend(records.next().expect("peeked record"));
		}
		if block.len() == DATA_BLOCK_HEADER_SIZE {
			let size = records.peek().map_or(0, Vec::len);
			return Err(Xex2Error::CompressedChunkTooLarge { size }.into_report());
		}
		block.extend_from_slice(&[0u8; RECORD_HEADER_SIZE]);
		block.resize(block.len().next_multiple_of(DATA_BLOCK_ALIGNMENT), 0);
		blocks.push(block);
	}
	if blocks.is_empty() {
		let mut block = vec![0u8; DATA_BLOCK_HEADER_SIZE + RECORD_HEADER_SIZE];
		block.resize(DATA_BLOCK_ALIGNMENT, 0);
		blocks.push(block);
	}
	Ok(blocks)
}

struct LinkedBlocks {
	data: Vec<u8>,
	first_size: u32,
	first_hash: Sha1Hash,
}

fn link_blocks(mut blocks: Vec<Vec<u8>>) -> LinkedBlocks {
	let mut next_size = 0u32;
	let mut next_hash = Sha1Hash::ZERO;
	for block in blocks.iter_mut().rev() {
		block[..4].copy_from_slice(&next_size.to_be_bytes());
		block[4..DATA_BLOCK_HEADER_SIZE].copy_from_slice(&*next_hash);
		next_size = block.len() as u32;
		next_hash = sha1(block);
	}
	LinkedBlocks {
		first_size: blocks.first().map_or(0, |block| block.len() as u32),
		first_hash: blocks.first().map_or(Sha1Hash::ZERO, |block| sha1(block)),
		data: blocks.concat(),
	}
}

fn descriptor(
	base: &Xex2,
	target: &Xex2,
	base_image_key: AesKey,
	target_header_size: u32,
	header_copy_size: u32,
	image_copy_size: u32,
	header_records: Vec<Vec<u8>>,
) -> Vec<u8> {
	let target_version = target.header.execution_info().map_or(0, |info| u32::from(info.version));
	let base_version = base.header.execution_info().map_or(0, |info| u32::from(info.version));
	let mut out = Vec::with_capacity(DELTA_DESCRIPTOR_FIXED_SIZE + header_records.iter().map(Vec::len).sum::<usize>());
	out.extend_from_slice(&0u32.to_be_bytes());
	out.extend_from_slice(&target_version.to_be_bytes());
	out.extend_from_slice(&base_version.to_be_bytes());
	out.extend_from_slice(&Sha1::digest(base.security_info.rsa_signature));
	out.extend_from_slice(&*base_image_key);
	out.extend_from_slice(&target_header_size.to_be_bytes());
	out.extend_from_slice(&0u32.to_be_bytes());
	out.extend_from_slice(&header_copy_size.to_be_bytes());
	out.extend_from_slice(&0u32.to_be_bytes());
	out.extend_from_slice(&0u32.to_be_bytes());
	out.extend_from_slice(&image_copy_size.to_be_bytes());
	out.extend_from_slice(&0u32.to_be_bytes());
	for record in header_records {
		out.extend(record);
	}
	out.extend_from_slice(&[0u8; RECORD_HEADER_SIZE]);
	let size = out.len() as u32;
	out[..4].copy_from_slice(&size.to_be_bytes());
	out
}

fn file_format(first_block_size: u32, first_block_hash: Sha1Hash) -> Vec<u8> {
	let mut out = Vec::with_capacity(36);
	out.extend_from_slice(&36u32.to_be_bytes());
	out.extend_from_slice(&(EncryptionType::None as u16).to_be_bytes());
	out.extend_from_slice(&(CompressionType::Delta as u16).to_be_bytes());
	out.extend_from_slice(&(DELTA_WINDOW_SIZE as u32).to_be_bytes());
	out.extend_from_slice(&first_block_size.to_be_bytes());
	out.extend_from_slice(&*first_block_hash);
	out
}

fn build_patch_file(
	target: &Xex2,
	image_size: u32,
	file_format: &[u8],
	descriptor: &[u8],
	data: &[u8],
) -> Result<Vec<u8>> {
	let security_offset = OPTIONAL_HEADER_INDEX_OFFSET + OPTIONAL_HEADER_COUNT * 8;
	let file_format_offset = security_offset + SECURITY_INFO_SIZE;
	let descriptor_offset = file_format_offset + file_format.len();
	let data_offset = (descriptor_offset + descriptor.len()).next_multiple_of(DATA_BLOCK_ALIGNMENT);
	if data_offset > super::FILE_HEADER_LIMIT as usize {
		return Err(Xex2Error::HeaderSize { size: data_offset as u32 }.into_report());
	}

	let mut out = vec![0u8; data_offset + data.len()];
	out[..4].copy_from_slice(b"XEX2");
	let target_kind = target.header.module_flags
		& (ModuleFlags::TITLE | ModuleFlags::EXPORTS_TO_TITLE | ModuleFlags::SYSTEM_DEBUGGER | ModuleFlags::DLL);
	let patch_flags = target_kind | ModuleFlags::PATCH | ModuleFlags::PATCH_FULL;
	out[4..8].copy_from_slice(&patch_flags.bits().to_be_bytes());
	out[8..12].copy_from_slice(&(data_offset as u32).to_be_bytes());
	out[0x10..0x14].copy_from_slice(&(security_offset as u32).to_be_bytes());
	out[0x14..0x18].copy_from_slice(&(OPTIONAL_HEADER_COUNT as u32).to_be_bytes());
	out[0x18..0x1C].copy_from_slice(&(OptionalHeaderKey::FileFormatInfo as u32).to_be_bytes());
	out[0x1C..0x20].copy_from_slice(&(file_format_offset as u32).to_be_bytes());
	out[0x20..0x24].copy_from_slice(&(OptionalHeaderKey::DeltaPatchDescriptor as u32).to_be_bytes());
	out[0x24..0x28].copy_from_slice(&(descriptor_offset as u32).to_be_bytes());

	out[security_offset..security_offset + 4].copy_from_slice(&(SECURITY_INFO_SIZE as u32).to_be_bytes());
	out[security_offset + 4..security_offset + 8].copy_from_slice(&image_size.to_be_bytes());
	let image_info = security_offset + IMAGE_INFO_OFFSET;
	out[image_info..image_info + 4].copy_from_slice(&0x174u32.to_be_bytes());
	let patch_image_flags = target.security_info.image_info.image_flags & ImageFlags::SMALL_PAGES;
	out[image_info + 4..image_info + 8].copy_from_slice(&patch_image_flags.bits().to_be_bytes());
	out[image_info + 8..image_info + 12].copy_from_slice(&target.security_info.image_info.load_address.0.to_be_bytes());
	out[security_offset + 0x17C..security_offset + 0x180].copy_from_slice(&u32::MAX.to_be_bytes());
	out[file_format_offset..file_format_offset + file_format.len()].copy_from_slice(file_format);
	out[descriptor_offset..descriptor_offset + descriptor.len()].copy_from_slice(descriptor);
	out[data_offset..].copy_from_slice(data);

	let parsed = Xex2::parse(&out)?;
	let header_hash = crate::hashes::compute_header_hash(&out, &parsed.header, &parsed.security_info)?;
	out[image_info + 0x5C..image_info + 0x70].copy_from_slice(&*header_hash);
	let digest = xecrypt::symmetric::xe_crypt_rot_sum_sha(&out[image_info..image_info + IMAGE_INFO_SIGNED_SIZE], &[]);
	let signature = xecrypt::xex_sig::sign_devkit(&digest).map_err(|_| Xex2Error::SigningFailed.into_report())?;
	out[security_offset + 8..security_offset + IMAGE_INFO_OFFSET].copy_from_slice(&signature);
	Ok(out)
}

fn sha1(bytes: &[u8]) -> Sha1Hash {
	Sha1Hash(Sha1::digest(bytes).into())
}
