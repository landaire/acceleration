//! XEX2 header signatures (`XeCryptBnQwBeSig`).
//!
//! XEX headers are signed with a proprietary RSA padding, not PKCS#1. The
//! console formats a 256-byte block from the RotSumSha digest and a 10-byte
//! salt, RC4-whitens it, clears the top bit, reverses it qword-wise, and
//! RSA-signs the result. The salt "XBOX360XEX" is present verbatim in the
//! kernel; a devkit kernel verifies this signature before mapping a title.
//!
//! Reference: emoose's ExCrypt `ExCryptBnQwBeSigFormat` / `...SigVerify`.

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

/// Reverse the order of the sixteen 8-byte qwords in a 256-byte buffer.
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

/// Build an RSA private key from big-endian P and Q factors (exponent 3).
pub fn private_key_from_pq(p_be: &[u8], q_be: &[u8]) -> Option<RsaPrivateKey> {
	RsaPrivateKey::from_p_q(
		BigUint::from_bytes_be(p_be),
		BigUint::from_bytes_be(q_be),
		BigUint::from(3u32),
	)
	.ok()
}

/// Sign a 20-byte RotSumSha digest, producing the 256-byte XEX signature blob
/// as stored in `SecurityInfo`.
pub fn sign(hash: &[u8; 20], salt: &[u8; 10], key: &RsaPrivateKey) -> [u8; 256] {
	let natural = natural_block(hash, salt);
	let r = BigUint::from_bytes_be(&natural).modpow(key.d(), key.n());
	qword_reverse(&to_bytes_256(&r))
}

/// Verify a stored XEX signature blob against a digest and public key.
pub fn verify(sig: &[u8; 256], hash: &[u8; 20], salt: &[u8; 10], key: &RsaPublicKey) -> bool {
	let a = BigUint::from_bytes_be(&qword_reverse(sig));
	let r = a.modpow(key.e(), key.n());
	to_bytes_256(&r) == natural_block(hash, salt)
}

#[cfg(test)]
mod tests {
	use super::*;
	use rsa::rand_core::OsRng;

	#[test]
	fn sign_verify_roundtrip() {
		// A freshly generated 2048-bit key exercises the padding and byte
		// orientation without any external key or file.
		let sk = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
		let pk = sk.to_public_key();
		let hash = [0x5Au8; 20];
		let sig = sign(&hash, SALT_XEX, &sk);
		assert!(verify(&sig, &hash, SALT_XEX, &pk));
		let mut altered = hash;
		altered[0] ^= 1;
		assert!(!verify(&sig, &altered, SALT_XEX, &pk));
	}
}
