//! XEX2 header signatures (`XeCryptBnQwBeSig`).
//!
//! XEX headers use proprietary RSA padding rather than PKCS#1. The signer
//! formats a 256-byte block from the RotSumSha digest and a 10-byte salt,
//! RC4-whitens it, clears the top bit, and reverses it qword-wise. The
//! console verifies with Montgomery arithmetic that is never converted out of
//! the Montgomery domain, so it accepts `s` when `s^e * R^-(e-1) mod n` equals
//! the block, where `R = 2^2048`. The development hypervisor contains the
//! corresponding public key.

use rsa::BigUint;
use rsa::RsaPrivateKey;
use rsa::RsaPublicKey;
use rsa::traits::PrivateKeyParts;
use rsa::traits::PublicKeyParts;
use sha1::Digest;
use sha1::Sha1;

/// Salt for a normal XEX signature. (Revocation-required images use
/// "XBOX360REV"; not needed here.)
pub const SALT_XEX: &[u8; 10] = b"XBOX360XEX";

fn rc4(key: &[u8], data: &mut [u8]) {
	let mut s: [u8; 256] = core::array::from_fn(|i| i as u8);
	let mut j = 0u8;
	for i in 0..256 {
		j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
		s.swap(i, j as usize);
	}
	let (mut a, mut b) = (0u8, 0u8);
	for byte in data.iter_mut() {
		a = a.wrapping_add(1);
		b = b.wrapping_add(s[a as usize]);
		s.swap(a as usize, b as usize);
		*byte ^= s[(s[a as usize].wrapping_add(s[b as usize])) as usize];
	}
}

/// Reverse the order of the 32 8-byte qwords in a 256-byte buffer.
fn qword_reverse(b: &[u8; 256]) -> [u8; 256] {
	let mut out = [0u8; 256];
	for i in 0..32 {
		out[i * 8..i * 8 + 8].copy_from_slice(&b[(31 - i) * 8..(31 - i) * 8 + 8]);
	}
	out
}

fn to_bytes_256(v: &BigUint) -> [u8; 256] {
	let bytes = v.to_bytes_be();
	let mut out = [0u8; 256];
	let n = bytes.len().min(256);
	out[256 - n..].copy_from_slice(&bytes[bytes.len() - n..]);
	out
}

/// The natural (pre-qword-reversal) formatted block: padding, the `0x01`
/// marker, salt, inner SHA-1, and `0xBC` trailer, RC4-whitened with the top
/// bit cleared so the value is less than the modulus.
fn natural_block(hash: &[u8; 20], salt: &[u8; 10]) -> [u8; 256] {
	let mut b = [0u8; 256];
	b[0xE0] = 0x01;
	b[0xE1..0xEB].copy_from_slice(salt);
	b[0xFF] = 0xBC;
	let mut h = Sha1::new();
	h.update(&b[0x00..0x08]);
	h.update(hash);
	h.update(salt);
	let inner: [u8; 20] = h.finalize().into();
	b[0xEB..0xFF].copy_from_slice(&inner);
	rc4(&inner, &mut b[0x00..0xEB]);
	b[0] &= 0x7F;
	b
}

/// `R^(e-1) mod n`, the factor the console's Montgomery verifier removes.
fn montgomery_factor(exponent: &BigUint, modulus: &BigUint) -> BigUint {
	let radix = BigUint::from(1u32) << 2048usize;
	radix.modpow(&(exponent - 1u32), modulus)
}

/// Build an RSA private key from big-endian P and Q factors (exponent 3).
pub fn private_key_from_pq(p_be: &[u8], q_be: &[u8]) -> Option<RsaPrivateKey> {
	RsaPrivateKey::from_p_q(BigUint::from_bytes_be(p_be), BigUint::from_bytes_be(q_be), BigUint::from(3u32)).ok()
}

/// Sign a 20-byte RotSumSha digest, producing the 256-byte XEX signature blob
/// as stored in `SecurityInfo`.
pub fn sign(hash: &[u8; 20], salt: &[u8; 10], key: &RsaPrivateKey) -> [u8; 256] {
	let natural = BigUint::from_bytes_be(&natural_block(hash, salt));
	let scaled = natural * montgomery_factor(key.e(), key.n()) % key.n();
	let r = scaled.modpow(key.d(), key.n());
	qword_reverse(&to_bytes_256(&r))
}

/// Sign a normal XEX digest with the devkit executable key.
pub fn sign_devkit(hash: &[u8; 20]) -> Result<[u8; 256], crate::Error> {
	let key = crate::RsaKeyKind::Executable.private_key(crate::ConsoleKind::Devkit)?;
	Ok(sign(hash, SALT_XEX, &key))
}

/// Verify a stored XEX signature blob against a digest and public key.
pub fn verify(sig: &[u8; 256], hash: &[u8; 20], salt: &[u8; 10], key: &RsaPublicKey) -> bool {
	let a = BigUint::from_bytes_be(&qword_reverse(sig));
	let r = a.modpow(key.e(), key.n());
	let natural = BigUint::from_bytes_be(&natural_block(hash, salt));
	r == natural * montgomery_factor(key.e(), key.n()) % key.n()
}

/// Verify a normal XEX signature with the devkit executable key.
pub fn verify_devkit(sig: &[u8; 256], hash: &[u8; 20]) -> Result<bool, crate::Error> {
	let key = crate::RsaKeyKind::Executable.public_key(crate::ConsoleKind::Devkit)?;
	Ok(verify(sig, hash, SALT_XEX, &key))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Montgomery product `a * b * R^-1 mod n` by REDC, with `R = 2^2048`.
	fn redc_mul(a: &BigUint, b: &BigUint, n: &BigUint, n_prime: &BigUint) -> BigUint {
		let mask = (BigUint::from(1u32) << 2048usize) - 1u32;
		let t = a * b;
		let m = ((&t & &mask) * n_prime) & &mask;
		let u = (t + m * n) >> 2048usize;
		if &u >= n { u - n } else { u }
	}

	/// `-n^-1 mod 2^2048` by Newton iteration on the odd modulus.
	fn negative_inverse(n: &BigUint) -> BigUint {
		let radix = BigUint::from(1u32) << 2048usize;
		let mut x = BigUint::from(1u32);
		for _ in 0..12 {
			let correction = (&radix + 2u32 - (n * &x) % &radix) % &radix;
			x = x * correction % &radix;
		}
		(&radix - x) % radix
	}

	/// The console's check: square `e >> 1` times, then multiply by the stored
	/// value, all in the Montgomery domain, and compare with the block.
	fn console_accepts(sig: &[u8; 256], hash: &[u8; 20], key: &RsaPublicKey) -> bool {
		let n = key.n();
		let n_prime = negative_inverse(n);
		let s = BigUint::from_bytes_be(&qword_reverse(sig));
		let mut acc = s.clone();
		let mut bits = key.e() >> 1usize;
		while bits != BigUint::from(0u32) {
			acc = redc_mul(&acc, &acc, n, &n_prime);
			bits >>= 1usize;
		}
		acc = redc_mul(&acc, &s, n, &n_prime);
		acc == BigUint::from_bytes_be(&natural_block(hash, SALT_XEX))
	}

	#[test]
	fn devkit_signature_passes_console_montgomery_check() {
		let key = crate::RsaKeyKind::Executable.private_key(crate::ConsoleKind::Devkit).unwrap();
		let hash = [0x5au8; 20];
		let sig = sign(&hash, SALT_XEX, &key);
		assert!(console_accepts(&sig, &hash, &key.to_public_key()));
		assert!(!console_accepts(&sig, &[0xa5u8; 20], &key.to_public_key()));
	}
}
