//! The cipher of the SMC firmware in the boot region.
//!
//! Each byte is XORed with the low byte of a 64-bit key, which is then rotated left by 48 bits
//! and fed with the ciphertext byte times 0xfb. The hypervisor decrypts the firmware this way
//! when it rewrites the boot region for a console; the decrypted firmware ends in a zero word.

const SEED: u64 = 0x0079_004e_0075_0042;
const MULTIPLIER: u64 = 0xfb;
const KEY_MASK: u64 = 0xffff_ffff_00ff_00ff;
const FEED_HIGH_MASK: u64 = 0x00ff_0000;
const FEED_LOW_MASK: u64 = 0xff;

/// The key after a byte whose ciphertext is `ciphertext`.
const fn next_key(key: u64, ciphertext: u8) -> u64 {
	let product = ciphertext as u64 * MULTIPLIER;
	key.rotate_left(48).wrapping_add(((product << 8) & FEED_HIGH_MASK) + (product & FEED_LOW_MASK)) & KEY_MASK
}

/// Decrypts SMC firmware in place.
pub fn decrypt(bytes: &mut [u8]) {
	let mut key = SEED;
	for byte in bytes {
		let ciphertext = *byte;
		*byte = ciphertext ^ key as u8;
		key = next_key(key, ciphertext);
	}
}

/// Encrypts SMC firmware in place.
pub fn encrypt(bytes: &mut [u8]) {
	let mut key = SEED;
	for byte in bytes {
		let ciphertext = *byte ^ key as u8;
		*byte = ciphertext;
		key = next_key(key, ciphertext);
	}
}

/// Whether decrypted firmware ends in the zero word that the hypervisor requires.
pub fn has_zero_trailer(firmware: &[u8]) -> bool {
	firmware.len() >= 4 && firmware[firmware.len() - 4..] == [0; 4]
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The key feeds on the ciphertext in both directions; feeding encryption with the plaintext
	/// would make the two disagree from the second byte on.
	#[test]
	fn encryption_inverts_decryption() {
		let plaintext: Vec<u8> = (0..=255u8).chain([0; 4]).collect();
		let mut bytes = plaintext.clone();
		encrypt(&mut bytes);
		assert_ne!(bytes, plaintext);
		decrypt(&mut bytes);
		assert_eq!(bytes, plaintext);
	}
}
