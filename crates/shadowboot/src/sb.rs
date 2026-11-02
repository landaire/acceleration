//! The console binding of SB: the sequence and digest at SB+0x20..0x40 that
//! `HvxKeysSaveBootLoader` writes (`HV_CB.Sequence`, `HV_CB.Digest`). The digest is an HMAC under
//! the CPU key of the console's NAND SMC, so a bound SB halts on any other console. SB 14352
//! skips the check when the binding is zero. The binding is outside the signed part of SB, which
//! covers the header and 0x140 onward.

use std::ops::Range;

use thiserror::Error;

use crate::se::hmac_sha1;
use crate::se::rc4;

const NONCE: Range<usize> = 0x10..0x20;
pub(crate) const BINDING: Range<usize> = 0x20..0x40;
const BINDING_SIZE: usize = BINDING.end - BINDING.start;

#[derive(Debug, Error)]
pub enum Error {
	#[error("boot stage \"SB\" is {size:#x} bytes, too short to hold the console binding at 0x20..0x40")]
	TooShort { size: usize },
}

/// Whether `sb` carries a console binding.
pub(crate) fn is_bound(sb: &[u8]) -> Result<bool, Error> {
	let keystream = keystream(sb)?;
	let bound = sb[BINDING].iter().zip(keystream).any(|(stored, key)| stored ^ key != 0);

	Ok(bound)
}

/// `sb` with a zero console binding, encrypted with its unchanged key. RC4 encrypts the binding
/// with the first bytes of the key stream from 0x20, so zero plaintext there is that key stream.
pub(crate) fn unbound(sb: &[u8]) -> Result<Vec<u8>, Error> {
	let keystream = keystream(sb)?;
	let mut unbound = sb.to_vec();
	unbound[BINDING].copy_from_slice(&keystream);

	Ok(unbound)
}

/// The RC4 key stream over the binding: SB is encrypted from 0x20 with
/// `HMAC-SHA1(boot ROM CB key, nonce)[..16]` (`HvpShadowBootVerifyCb`).
fn keystream(sb: &[u8]) -> Result<[u8; BINDING_SIZE], Error> {
	if sb.len() < BINDING.end {
		return Err(Error::TooShort { size: sb.len() });
	}

	let digest = hmac_sha1(&xecrypt::symmetric::BOOT_ROM_CB_KEY, &sb[NONCE]);
	let mut keystream = [0u8; BINDING_SIZE];
	rc4(&digest[..16], &mut keystream);

	Ok(keystream)
}

/// The decrypted SB.
#[cfg(test)]
pub(crate) fn plaintext(sb: &[u8]) -> Vec<u8> {
	let digest = hmac_sha1(&xecrypt::symmetric::BOOT_ROM_CB_KEY, &sb[NONCE]);
	let mut bytes = sb.to_vec();
	rc4(&digest[..16], &mut bytes[BINDING.start..]);
	bytes
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Unbinding zeroes only the plaintext binding: the stage decrypts to the same bytes
	/// elsewhere.
	#[test]
	fn unbinding_zeroes_only_the_binding() {
		let mut source = vec![0u8; 0x100];
		source[..2].copy_from_slice(b"SB");
		source[NONCE].copy_from_slice(&[0x5a; 16]);
		source[BINDING].copy_from_slice(&[0xc3; BINDING_SIZE]);
		for (offset, byte) in source.iter_mut().enumerate().skip(BINDING.end) {
			*byte = offset as u8;
		}
		// RC4 is its own inverse, so decrypting the plaintext encrypts it.
		let encrypted = plaintext(&source);
		assert!(is_bound(&encrypted).unwrap());

		let unbound = unbound(&encrypted).unwrap();
		assert!(!is_bound(&unbound).unwrap());
		let decrypted = plaintext(&unbound);
		assert!(decrypted[BINDING].iter().all(|byte| *byte == 0));
		assert_eq!(decrypted[..BINDING.start], source[..BINDING.start]);
		assert_eq!(decrypted[BINDING.end..], source[BINDING.end..]);
		assert_eq!(super::unbound(&unbound).unwrap(), unbound);
		assert!(matches!(super::unbound(b"SB"), Err(Error::TooShort { size: 2 })));
	}
}
