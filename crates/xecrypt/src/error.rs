use thiserror::Error;

use crate::ConsoleKind;
use crate::RsaKeyKind;

#[derive(Debug, Error)]
pub enum Error {
	#[error("No private key exists for key kind {0:?} and console kind {1:?}")]
	NoPrivateKey(RsaKeyKind, ConsoleKind),
	#[error("RSA operation failed")]
	RsaError(#[from] rsa::Error),
	#[error("a {length}-byte digest is not a SHA-1 digest")]
	DigestLength { length: usize },
}

/// A console signature that does not verify against its certificate's key.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleSignatureError {
	#[error("the certificate's public key with exponent {exponent:#x} is not a usable RSA key")]
	InvalidPublicKey { exponent: u32 },
	#[error("the signature does not match the signed bytes")]
	Mismatch,
}
