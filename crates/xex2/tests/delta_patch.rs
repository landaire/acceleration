//! Delta patches built from synthetic modules: a base XEX, the patched XEX
//! it should become, and a development-signed XEXP that turns one into the
//! other.

use sha1::Digest;
use sha1::Sha1;
use xenon_types::VirtualAddress;
use xex2::Xex2;
use xex2::builder::EncryptionTarget;
use xex2::builder::Xex2Builder;
use xex2::delta::FILE_HEADER_LIMIT;
use xex2::delta::apply_delta_patch;
use xex2::delta::create_delta_patch;
use xex2::error::Report;
use xex2::error::Xex2Error;
use xex2::writer::TargetCompression;

const IMAGE_SIZE: usize = 0x1_0000;
const LOAD_ADDRESS: u32 = 0x8200_0000;
const PATCH_HEADER_SIZE: usize = 0x1000;
const SECURITY_OFFSET: usize = 0x28;
const FILE_FORMAT_OFFSET: usize = 0x1AC;
const DESCRIPTOR_OFFSET: usize = 0x1D0;

fn base_image() -> Vec<u8> {
	let mut image: Vec<u8> = (0..IMAGE_SIZE).map(|i| (i * 7 + i / 251) as u8).collect();
	// A DOS header whose PE header offset is 0 and a COFF header with no
	// sections, which the builder accepts.
	image[..0x200].fill(0);
	image[..2].copy_from_slice(b"MZ");
	image
}

/// The patched image: a copied range, a range of new bytes and a zeroed
/// range, each produced by one image delta record.
fn target_image() -> Vec<u8> {
	let mut image = base_image();
	image.copy_within(0x1000..0x1100, 0x4000);
	for (offset, byte) in image[0x6000..0x6800].iter_mut().enumerate() {
		*byte = (offset % 13) as u8 ^ 0x5A;
	}
	image[0x9000..0x9100].fill(0);
	image
}

fn build(image: Vec<u8>) -> Vec<u8> {
	Xex2Builder::new(image)
		.entry_point(VirtualAddress(LOAD_ADDRESS + 0x1000))
		.load_address(VirtualAddress(LOAD_ADDRESS))
		.encrypt(EncryptionTarget::Devkit)
		.compress()
		.build()
		.unwrap()
}

fn be(value: u32) -> [u8; 4] {
	value.to_be_bytes()
}

fn record(source: u32, target: u32, size: u16, data: Option<&[u8]>) -> Vec<u8> {
	let compressed_size = match data {
		Some(data) => data.len() as u16,
		None if source == target => 0,
		None => 1,
	};
	let mut bytes = Vec::new();
	bytes.extend_from_slice(&be(source));
	bytes.extend_from_slice(&be(target));
	bytes.extend_from_slice(&size.to_be_bytes());
	bytes.extend_from_slice(&compressed_size.to_be_bytes());
	if let Some(data) = data {
		bytes.extend_from_slice(data);
	}
	bytes
}

fn lzx_frame(data: &[u8]) -> Vec<u8> {
	lzxc::Encoder::new(lzxc::WindowSize::KB32).encode_chunk(data)
}

/// The development session key of a builder output: the file key unwrapped
/// with the zero master key.
fn session_key(xex: &Xex2) -> [u8; 16] {
	let mut key = *xex.security_info.image_info.file_key;
	xecrypt::symmetric::xe_crypt_aes_ecb_decrypt(&[0; 16], &mut key);
	key
}

/// Parts of the patch that the corruption tests change.
struct PatchSpec {
	base_signature_digest: [u8; 20],
	header_source_size: u32,
	target_header_size: u32,
	image_records: Vec<u8>,
	sign: bool,
}

struct Fixture {
	base: Vec<u8>,
	target: Vec<u8>,
}

impl Fixture {
	fn new() -> Self {
		Self { base: build(base_image()), target: build(target_image()) }
	}

	fn spec(&self) -> PatchSpec {
		let base_xex = Xex2::parse(&self.base).unwrap();
		let target_xex = Xex2::parse(&self.target).unwrap();

		let mut image_records = record(0x1000, 0x4000, 0x100, None);
		image_records.extend(record(0x6000, 0x6000, 0x800, Some(&lzx_frame(&target_image()[0x6000..0x6800]))));
		image_records.extend(record(0x9000, 0x9000, 0x100, None));

		PatchSpec {
			base_signature_digest: Sha1::digest(base_xex.security_info.rsa_signature).into(),
			header_source_size: base_xex.header.data_offset.min(target_xex.header.data_offset),
			target_header_size: target_xex.header.data_offset,
			image_records,
			sign: true,
		}
	}

	fn patch(&self, spec: &PatchSpec) -> Vec<u8> {
		let base_xex = Xex2::parse(&self.base).unwrap();
		let target_xex = Xex2::parse(&self.target).unwrap();
		let target_headers = &self.target[..target_xex.header.data_offset as usize];

		// The base session key, wrapped with the patched module's.
		let mut base_image_key = session_key(&base_xex);
		xecrypt::symmetric::xe_crypt_aes_ecb_encrypt(&session_key(&target_xex), &mut base_image_key);

		let mut descriptor = Vec::new();
		descriptor.extend_from_slice(&be(0));
		descriptor.extend_from_slice(&be(0x2000_0100));
		descriptor.extend_from_slice(&be(0x2000_0000));
		descriptor.extend_from_slice(&spec.base_signature_digest);
		descriptor.extend_from_slice(&base_image_key);
		descriptor.extend_from_slice(&be(spec.target_header_size));
		descriptor.extend_from_slice(&be(0));
		descriptor.extend_from_slice(&be(spec.header_source_size));
		descriptor.extend_from_slice(&be(0));
		descriptor.extend_from_slice(&be(0));
		descriptor.extend_from_slice(&be(IMAGE_SIZE as u32));
		descriptor.extend_from_slice(&be(0));
		descriptor.extend(record(0, 0, target_headers.len() as u16, Some(&lzx_frame(target_headers))));
		let descriptor_size = descriptor.len() as u32;
		descriptor[..4].copy_from_slice(&be(descriptor_size));

		let mut block = vec![0u8; 24];
		block.extend_from_slice(&spec.image_records);
		block.resize(block.len().next_multiple_of(0x800), 0);

		let mut file_format = Vec::new();
		file_format.extend_from_slice(&be(0x24));
		file_format.extend_from_slice(&[0, 0, 0, 3]);
		file_format.extend_from_slice(&be(0x8000));
		file_format.extend_from_slice(&be(block.len() as u32));
		file_format.extend_from_slice(&Sha1::digest(&block));

		let mut patch = vec![0u8; PATCH_HEADER_SIZE];
		patch[..4].copy_from_slice(b"XEX2");
		patch[4..8].copy_from_slice(&be(0x31));
		patch[8..12].copy_from_slice(&be(PATCH_HEADER_SIZE as u32));
		patch[0x10..0x14].copy_from_slice(&be(SECURITY_OFFSET as u32));
		patch[0x14..0x18].copy_from_slice(&be(2));
		patch[0x18..0x1C].copy_from_slice(&be(0x3FF));
		patch[0x1C..0x20].copy_from_slice(&be(FILE_FORMAT_OFFSET as u32));
		patch[0x20..0x24].copy_from_slice(&be(0x5FF));
		patch[0x24..0x28].copy_from_slice(&be(DESCRIPTOR_OFFSET as u32));

		let security = &mut patch[SECURITY_OFFSET..];
		security[0..4].copy_from_slice(&be(0x184));
		security[4..8].copy_from_slice(&be(IMAGE_SIZE as u32));
		let image_info = &mut security[0x108..0x17C];
		image_info[0..4].copy_from_slice(&be(0x174));
		image_info[8..12].copy_from_slice(&be(LOAD_ADDRESS));

		patch[FILE_FORMAT_OFFSET..FILE_FORMAT_OFFSET + file_format.len()].copy_from_slice(&file_format);
		patch[DESCRIPTOR_OFFSET..DESCRIPTOR_OFFSET + descriptor.len()].copy_from_slice(&descriptor);

		let parsed = Xex2::parse(&patch).unwrap();
		let header_hash = xex2::hashes::compute_header_hash(&patch, &parsed.header, &parsed.security_info).unwrap();
		let image_info_start = SECURITY_OFFSET + 0x108;
		patch[image_info_start + 0x5C..image_info_start + 0x70].copy_from_slice(&*header_hash);
		if spec.sign {
			let digest =
				xecrypt::symmetric::xe_crypt_rot_sum_sha(&patch[image_info_start..image_info_start + 0x74], &[]);
			let signature = xecrypt::xex_sig::sign_devkit(&digest).unwrap();
			patch[SECURITY_OFFSET + 8..SECURITY_OFFSET + 0x108].copy_from_slice(&signature);
		}

		patch.extend_from_slice(&block);
		patch
	}
}

fn error(base: &[u8], patch: &[u8], limit: u32) -> Report {
	apply_delta_patch(base, patch, limit).unwrap_err()
}

#[test]
fn rebuilds_the_patched_headers_and_image() {
	let fixture = Fixture::new();
	let patch = fixture.patch(&fixture.spec());

	let patched = apply_delta_patch(&fixture.base, &patch, FILE_HEADER_LIMIT).unwrap();

	let target_xex = Xex2::parse(&fixture.target).unwrap();
	assert_eq!(patched.headers, fixture.target[..target_xex.header.data_offset as usize]);
	assert!(patched.image == target_image());
	assert_eq!(patched.machine, xex2::writer::TargetMachine::Devkit);
}

#[test]
fn generated_patch_reconstructs_target() {
	let fixture = Fixture::new();
	let patch = create_delta_patch(&fixture.base, &fixture.target).unwrap();

	let patched = apply_delta_patch(&fixture.base, &patch, FILE_HEADER_LIMIT).unwrap();

	let target_xex = Xex2::parse(&fixture.target).unwrap();
	assert_eq!(patched.headers, fixture.target[..target_xex.header.data_offset as usize]);
	assert_eq!(patched.image, target_image());
	assert_eq!(patched.machine, xex2::writer::TargetMachine::Devkit);
	let patch_xex = Xex2::parse(&patch).unwrap();
	assert_eq!(patch_xex.security_info.image_info.allowed_media_types.bits(), u32::MAX);
}

#[test]
fn generated_multiblock_patch_reconstructs_target() {
	let mut base_image = vec![0u8; 0x8_0000];
	base_image[..2].copy_from_slice(b"MZ");
	let mut target_image = base_image.clone();
	let mut state = 0x1234_5678u32;
	for byte in &mut target_image[0x200..] {
		state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
		*byte = (state >> 24) as u8;
	}
	let base = build(base_image);
	let target = build(target_image.clone());

	let patch = create_delta_patch(&base, &target).unwrap();
	let patched = apply_delta_patch(&base, &patch, FILE_HEADER_LIMIT).unwrap();

	assert_eq!(patched.image, target_image);
}

#[test]
fn rejects_uncompressed_base() {
	let fixture = Fixture::new();
	let mut uncompressed = Vec::new();
	Xex2::parse(&fixture.base)
		.unwrap()
		.rebuild(&fixture.base)
		.target_compression(TargetCompression::Uncompressed)
		.write_to(&mut uncompressed)
		.unwrap();

	let error = create_delta_patch(&uncompressed, &fixture.target).unwrap_err();

	assert!(matches!(error.current_context(), Xex2Error::InvalidCompressionFormat(0)), "{error}");
}

#[test]
fn rejects_a_patch_for_another_base() {
	let fixture = Fixture::new();
	let spec = PatchSpec { base_signature_digest: [0; 20], ..fixture.spec() };

	let error = error(&fixture.base, &fixture.patch(&spec), FILE_HEADER_LIMIT);

	assert!(matches!(error.current_context(), Xex2Error::BaseSignatureDigestMismatch), "{error}");
}

#[test]
fn rejects_a_header_copy_past_the_base_headers() {
	let fixture = Fixture::new();
	let spec = PatchSpec { header_source_size: 0x10_0000, ..fixture.spec() };

	let error = error(&fixture.base, &fixture.patch(&spec), FILE_HEADER_LIMIT);

	assert!(matches!(error.current_context(), Xex2Error::DeltaHeaderRange { .. }), "{error}");
}

#[test]
fn rejects_patched_headers_above_the_limit() {
	let fixture = Fixture::new();
	let spec = fixture.spec();
	let requested = spec.target_header_size;

	let error = error(&fixture.base, &fixture.patch(&spec), requested - 0x800);

	assert!(
		matches!(error.current_context(), Xex2Error::DeltaHeaderTooLarge { requested: r, .. } if *r == requested),
		"{error}"
	);
}

#[test]
fn rejects_an_unsigned_patch() {
	let fixture = Fixture::new();
	let spec = PatchSpec { sign: false, ..fixture.spec() };

	let error = error(&fixture.base, &fixture.patch(&spec), FILE_HEADER_LIMIT);

	assert!(matches!(error.current_context(), Xex2Error::SignatureInvalid { .. }), "{error}");
}

#[test]
fn rejects_a_corrupt_data_block() {
	let fixture = Fixture::new();
	let mut patch = fixture.patch(&fixture.spec());
	patch[PATCH_HEADER_SIZE + 30] ^= 1;

	let error = error(&fixture.base, &patch, FILE_HEADER_LIMIT);

	assert!(matches!(error.current_context(), Xex2Error::DataBlockHashMismatch { block_index: 0 }), "{error}");
}

#[test]
fn rejects_an_image_that_does_not_match_the_page_descriptors() {
	let fixture = Fixture::new();
	let mut spec = fixture.spec();
	spec.image_records.extend(record(0x9000, 0xA000, 0x10, None));

	let error = error(&fixture.base, &fixture.patch(&spec), FILE_HEADER_LIMIT);

	assert!(matches!(error.current_context(), Xex2Error::PageDescriptorMismatch { .. }), "{error}");
}
