use aes::cipher::{BlockDecryptMut, KeyIvInit};
use sha1::{Digest, Sha1};

use crate::error::{Result, Xex2Error};
use rootcause::IntoReport;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

pub const RETAIL_KEY: [u8; 16] = [
	0x20, 0xB1, 0x85, 0xA5, 0x9D, 0x28, 0xFD, 0xC3, 0x40, 0x58, 0x3F, 0xBB, 0x08, 0x96, 0xBF, 0x91,
];

pub const DEVKIT_KEY: [u8; 16] = [0u8; 16];

pub fn decrypt_file_key(file_key: &[u8; 16]) -> ([u8; 16], [u8; 16]) {
	let retail = decrypt_key_with(file_key, &RETAIL_KEY);
	let devkit = decrypt_key_with(file_key, &DEVKIT_KEY);
	(retail, devkit)
}

fn decrypt_key_with(file_key: &[u8; 16], key: &[u8; 16]) -> [u8; 16] {
	let iv = [0u8; 16];
	let mut buf = *file_key;
	let dec = Aes128CbcDec::new(key.into(), &iv.into());
	dec.decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut buf)
		.expect("AES-128-CBC decrypt of 16-byte key should not fail");
	buf
}

pub fn decrypt_data(data: &[u8], session_key: &[u8; 16]) -> Vec<u8> {
	let iv = [0u8; 16];
	let aligned_len = data.len() & !0xF;
	let mut buf = data[..aligned_len].to_vec();
	let dec = Aes128CbcDec::new(session_key.into(), &iv.into());
	dec.decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut buf)
		.expect("AES-128-CBC decrypt should not fail on aligned data");
	if aligned_len < data.len() {
		buf.extend_from_slice(&data[aligned_len..]);
	}
	buf
}

pub fn sha1_hash(data: &[u8]) -> [u8; 20] {
	let mut hasher = Sha1::new();
	hasher.update(data);
	hasher.finalize().into()
}

pub fn verify_block_hash(block_data: &[u8], expected_hash: &[u8; 20]) -> Result<()> {
	let calculated = sha1_hash(block_data);
	if calculated != *expected_hash {
		return Err(Xex2Error::HashMismatch { block_index: 0 }.into_report());
	}
	Ok(())
}
