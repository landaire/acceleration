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

impl AesKey {
	pub fn decrypt_as_file_key(&self) -> DecryptedKeys {
		DecryptedKeys { retail: decrypt_key_with(self, &RETAIL_KEY), devkit: decrypt_key_with(self, &DEVKIT_KEY) }
	}
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

pub fn verify_block_hash(block_data: &[u8], expected_hash: &[u8; 20]) -> Result<()> {
	let calculated = sha1_hash(block_data);
	if calculated != *expected_hash {
		return Err(Xex2Error::HashMismatch { block_index: 0 }.into_report());
	}
	Ok(())
}
