use aes::cipher::BlockModeDecrypt;
use aes::cipher::BlockModeEncrypt;
use aes::cipher::KeyIvInit;
use aes::cipher::block_padding::NoPadding;
use sha1::Digest;
use sha1::Sha1;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;
type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

pub fn xe_crypt_sha(data: &[u8]) -> [u8; 20] {
	let mut hasher = Sha1::new();
	hasher.update(data);
	hasher.finalize().into()
}

pub fn xe_crypt_rot_sum(state: &mut [u64; 4], data: &[u8]) {
	for chunk in data.chunks_exact(8) {
		let qw = u64::from_be_bytes(chunk.try_into().unwrap());
		let sum = state[1].wrapping_add(qw);
		let carry = if sum < qw { 1u64 } else { 0 };
		state[0] = state[0].wrapping_add(carry);
		state[1] = sum.rotate_left(29);
		let diff = state[3].wrapping_sub(qw);
		let borrow = if state[3] < qw { 1u64 } else { 0 };
		state[2] = state[2].wrapping_sub(borrow);
		state[3] = diff.rotate_left(31);
	}
}

pub fn xe_crypt_rot_sum_sha(data1: &[u8], data2: &[u8]) -> [u8; 20] {
	let mut rot = [0u64; 4];
	xe_crypt_rot_sum(&mut rot, data1);
	xe_crypt_rot_sum(&mut rot, data2);

	let mut hasher = Sha1::new();

	let rot_bytes: Vec<u8> = rot.iter().flat_map(|q| q.to_be_bytes()).collect();
	hasher.update(&rot_bytes);
	hasher.update(&rot_bytes);
	hasher.update(data1);
	hasher.update(data2);

	let inv_bytes: Vec<u8> = rot.iter().map(|q| !q).flat_map(|q| q.to_be_bytes()).collect();
	hasher.update(&inv_bytes);
	hasher.update(&inv_bytes);

	hasher.finalize().into()
}

pub fn xe_crypt_aes_cbc_decrypt(key: &[u8; 16], iv: &[u8; 16], data: &mut [u8]) {
	let aligned_len = data.len() & !0xF;
	if aligned_len == 0 {
		return;
	}
	Aes128CbcDec::new(key.into(), iv.into())
		.decrypt_padded::<NoPadding>(&mut data[..aligned_len])
		.expect("AES-128-CBC decrypt should not fail on aligned data");
}

pub fn xe_crypt_aes_cbc_encrypt(key: &[u8; 16], iv: &[u8; 16], data: &mut [u8]) {
	let aligned_len = data.len() & !0xF;
	if aligned_len == 0 {
		return;
	}
	Aes128CbcEnc::new(key.into(), iv.into())
		.encrypt_padded::<NoPadding>(&mut data[..aligned_len], aligned_len)
		.expect("AES-128-CBC encrypt should not fail on aligned data");
}

pub fn xe_crypt_aes_ecb_decrypt(key: &[u8; 16], block: &mut [u8; 16]) {
	let iv = [0u8; 16];
	xe_crypt_aes_cbc_decrypt(key, &iv, block);
}

pub fn xe_crypt_aes_ecb_encrypt(key: &[u8; 16], block: &mut [u8; 16]) {
	let iv = [0u8; 16];
	xe_crypt_aes_cbc_encrypt(key, &iv, block);
}
