//! AES-128 key handling for XEX file encryption.
//!
//! Each XEX stores a per-file "image key" in its ImageInfo. The hypervisor
//! decrypts it with a master key to obtain the session key that decrypts
//! the payload. The master key depends on the console type and on the image
//! flags (hypervisor 17559 `0x29a68`).

use crate::header::AesKey;
use crate::opt::ImageFlags;
use crate::writer::TargetMachine;

pub const RETAIL_KEY: AesKey =
	AesKey([0x20, 0xB1, 0x85, 0xA5, 0x9D, 0x28, 0xFD, 0xC3, 0x40, 0x58, 0x3F, 0xBB, 0x08, 0x96, 0xBF, 0x91]);

/// Master key the retail hypervisor applies to images with
/// [`ImageFlags::SECOND_MASTER_KEY`] set (hypervisor 17559 offset `0x10cf8`).
pub const RETAIL_SECOND_KEY: AesKey =
	AesKey([0xA2, 0x6C, 0x10, 0xF7, 0x1F, 0xD9, 0x35, 0xE9, 0x8B, 0x99, 0x92, 0x2C, 0xE9, 0x32, 0x15, 0x72]);

pub const DEVKIT_KEY: AesKey = AesKey([0u8; 16]);

/// Master key that wraps the file key of an image with `image_flags` on
/// `machine`. `None` for a development console and an image that selects the
/// second master key; that key has not been recovered.
pub fn master_key(machine: TargetMachine, image_flags: ImageFlags) -> Option<AesKey> {
	let second = image_flags.intersects(ImageFlags::SECOND_MASTER_KEY);
	match (machine, second) {
		(TargetMachine::Retail, false) => Some(RETAIL_KEY),
		(TargetMachine::Retail, true) => Some(RETAIL_SECOND_KEY),
		(TargetMachine::Devkit, false) => Some(DEVKIT_KEY),
		(TargetMachine::Devkit, true) => None,
	}
}

/// Decrypt a wrapped 16-byte key: an AES-ECB block.
pub fn unwrap_key(wrapped: &AesKey, key: &AesKey) -> AesKey {
	let mut block = **wrapped;
	xecrypt::symmetric::xe_crypt_aes_ecb_decrypt(key, &mut block);
	AesKey(block)
}

pub fn decrypt_data(data: &[u8], session_key: &AesKey) -> Vec<u8> {
	let iv = [0u8; 16];
	let mut buf = data.to_vec();
	xecrypt::symmetric::xe_crypt_aes_cbc_decrypt(session_key, &iv, &mut buf);
	buf
}

/// Encrypt the raw session key under `master_key` to produce the
/// ImageInfo.file_key that the kernel would decrypt at load time.
///
/// Inverse of [`unwrap_key`].
pub fn wrap_file_key(session_key: &AesKey, master_key: &AesKey) -> AesKey {
	let iv = [0u8; 16];
	let mut buf = **session_key;
	xecrypt::symmetric::xe_crypt_aes_cbc_encrypt(master_key, &iv, &mut buf);
	AesKey(buf)
}

/// AES-128-CBC encrypt the payload using `session_key`. Mirror of [`decrypt_data`].
pub fn encrypt_data(data: &[u8], session_key: &AesKey) -> Vec<u8> {
	let iv = [0u8; 16];
	let mut buf = data.to_vec();
	xecrypt::symmetric::xe_crypt_aes_cbc_encrypt(session_key, &iv, &mut buf);
	buf
}

/// AES-128-CBC decryption of the image data in successive pieces, as the
/// loader reads it. Each piece decrypts its whole 16-byte blocks and leaves
/// trailing bytes unchanged, as `XeCryptAesCbc` (kernel 21256 `0x80150608`)
/// does.
pub struct CbcDecryptor {
	key: AesKey,
	chain: [u8; 16],
}

impl CbcDecryptor {
	pub fn new(key: AesKey) -> Self {
		Self { key, chain: [0; 16] }
	}

	pub fn decrypt(&mut self, data: &mut [u8]) {
		let whole = data.len() & !0xF;
		if whole == 0 {
			return;
		}

		let mut next_chain = [0u8; 16];
		next_chain.copy_from_slice(&data[whole - 16..whole]);
		xecrypt::symmetric::xe_crypt_aes_cbc_decrypt(&self.key, &self.chain, &mut data[..whole]);
		self.chain = next_chain;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn wrap_round_trips_with_unwrap() {
		let session = AesKey([0x42; 16]);
		let wrapped = wrap_file_key(&session, &RETAIL_KEY);
		assert_eq!(*unwrap_key(&wrapped, &RETAIL_KEY), *session);
	}

	#[test]
	fn encrypt_round_trips_with_decrypt() {
		let key = AesKey([0x77; 16]);
		let plaintext = vec![0u8; 256];
		let enc = encrypt_data(&plaintext, &key);
		assert_ne!(enc, plaintext);
		let dec = decrypt_data(&enc, &key);
		assert_eq!(dec, plaintext);
	}

	#[test]
	fn piecewise_decryption_matches_one_pass() {
		let key = AesKey([0x31; 16]);
		let plaintext: Vec<u8> = (0..0x60u8).collect();
		let ciphertext = encrypt_data(&plaintext, &key);

		let mut pieces = ciphertext.clone();
		let mut decryptor = CbcDecryptor::new(key);
		let (first, rest) = pieces.split_at_mut(0x20);
		decryptor.decrypt(first);
		decryptor.decrypt(rest);

		assert_eq!(pieces, plaintext);
	}
}
