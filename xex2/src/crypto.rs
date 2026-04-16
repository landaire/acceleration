//! AES-128-CBC key handling for XEX file encryption.
//!
//! Each XEX stores a per-file "image key" in its ImageInfo. This key is
//! encrypted with either the retail or devkit master key. Decrypting the
//! image key yields the session key used to decrypt the payload.

use crate::error::Result;
use crate::error::Xex2Error;
use crate::header::AesKey;
use rootcause::IntoReport;

pub use xecrypt::symmetric::xe_crypt_sha as sha1_hash;

pub const RETAIL_KEY: AesKey =
	AesKey([0x20, 0xB1, 0x85, 0xA5, 0x9D, 0x28, 0xFD, 0xC3, 0x40, 0x58, 0x3F, 0xBB, 0x08, 0x96, 0xBF, 0x91]);

pub const DEVKIT_KEY: AesKey = AesKey([0u8; 16]);

pub struct DecryptedKeys {
	pub retail: AesKey,
	pub devkit: AesKey,
}

pub fn decrypt_file_key(file_key: &AesKey) -> DecryptedKeys {
	DecryptedKeys { retail: decrypt_key_with(file_key, &RETAIL_KEY), devkit: decrypt_key_with(file_key, &DEVKIT_KEY) }
}

fn decrypt_key_with(file_key: &AesKey, key: &AesKey) -> AesKey {
	let iv = [0u8; 16];
	let mut buf = file_key.0;
	xecrypt::symmetric::xe_crypt_aes_cbc_decrypt(&key.0, &iv, &mut buf);
	AesKey(buf)
}

pub fn decrypt_data(data: &[u8], session_key: &AesKey) -> Vec<u8> {
	let iv = [0u8; 16];
	let mut buf = data.to_vec();
	xecrypt::symmetric::xe_crypt_aes_cbc_decrypt(&session_key.0, &iv, &mut buf);
	buf
}

/// Encrypt the raw session key under `master_key` to produce the
/// ImageInfo.file_key that the kernel would decrypt at load time.
///
/// Inverse of [`decrypt_file_key`] + session-key selection.
pub fn wrap_file_key(session_key: &AesKey, master_key: &AesKey) -> AesKey {
	let iv = [0u8; 16];
	let mut buf = session_key.0;
	xecrypt::symmetric::xe_crypt_aes_cbc_encrypt(&master_key.0, &iv, &mut buf);
	AesKey(buf)
}

/// AES-128-CBC encrypt the payload using `session_key`. Mirror of [`decrypt_data`].
pub fn encrypt_data(data: &[u8], session_key: &AesKey) -> Vec<u8> {
	let iv = [0u8; 16];
	let mut buf = data.to_vec();
	xecrypt::symmetric::xe_crypt_aes_cbc_encrypt(&session_key.0, &iv, &mut buf);
	buf
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn wrap_round_trips_with_decrypt() {
		let session = AesKey([0x42; 16]);
		let wrapped = wrap_file_key(&session, &RETAIL_KEY);
		let unwrapped = decrypt_file_key(&wrapped);
		assert_eq!(unwrapped.retail.0, session.0);
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
}

pub fn verify_block_hash(block_data: &[u8], expected_hash: &[u8; 20]) -> Result<()> {
	let calculated = sha1_hash(block_data);
	if calculated != *expected_hash {
		return Err(Xex2Error::HashMismatch { block_index: 0 }.into_report());
	}
	Ok(())
}
