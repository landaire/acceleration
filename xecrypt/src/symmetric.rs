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
