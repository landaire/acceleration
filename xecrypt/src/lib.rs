use bitflags::bitflags;
use byteorder::BigEndian;
use byteorder::ReadBytesExt;
use rsa::BigUint;
use rsa::Pkcs1v15Sign;
use rsa::RsaPrivateKey;
use rsa::RsaPublicKey;
use std::io::Cursor;
use std::io::Read;

mod error;
mod keys;
pub mod keyvault;
pub mod symmetric;

#[cfg(feature = "serde")]
use serde::Serialize;

pub use error::Error;

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub enum XContentSignatureType {
	Console,
	Live,
	Pirs,
}

impl XContentSignatureType {
	pub fn parse(data: &[u8; 4]) -> Option<Self> {
		match data {
			b"CON " => Some(Self::Console),
			b"LIVE" => Some(Self::Live),
			b"PIRS" => Some(Self::Pirs),
			_ => None,
		}
	}
}

impl std::fmt::Display for XContentSignatureType {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let description = match self {
			XContentSignatureType::Console => "Console (CON)",
			XContentSignatureType::Live => "Xbox LIVE Strong Signature (LIVE)",
			XContentSignatureType::Pirs => "Offline Strong Signature (PIRS)",
		};
		f.write_str(description)
	}
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub enum XContentKeyMaterial {
	Certificate(Certificate),
	Signature(Vec<u8>, Vec<u8>),
}

impl XContentKeyMaterial {
	pub fn parse(cursor: &mut Cursor<&[u8]>, signature_type: XContentSignatureType) -> std::io::Result<Self> {
		let start_pos = cursor.position();
		let result = if signature_type == XContentSignatureType::Console {
			Self::Certificate(Certificate::parse(cursor)?)
		} else {
			let mut sig = vec![0u8; 256];
			cursor.read_exact(&mut sig)?;
			let mut reserved = vec![0u8; 256];
			cursor.read_exact(&mut reserved)?;
			Self::Signature(sig, reserved)
		};
		// Pad to 0x228 bytes total
		cursor.set_position(start_pos + 0x228);
		Ok(result)
	}
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct Certificate {
	pub pubkey_cert_size: u16,
	pub owner_console_id: [u8; 5],
	pub owner_console_part_number: String,
	pub console_type_flags: Option<ConsoleTypeFlags>,
	pub date_generation: String,
	pub public_exponent: u32,
	pub public_modulus: Vec<u8>,
	pub certificate_signature: Vec<u8>,
	pub signature: Vec<u8>,
}

impl Certificate {
	pub fn parse(cursor: &mut Cursor<&[u8]>) -> std::io::Result<Self> {
		let pubkey_cert_size = cursor.read_u16::<BigEndian>()?;
		let mut owner_console_id = [0u8; 5];
		cursor.read_exact(&mut owner_console_id)?;

		let mut part_number_raw = [0u8; 0x11 * 2];
		cursor.read_exact(&mut part_number_raw)?;
		let owner_console_part_number = read_wide_string(&part_number_raw);

		let console_type_raw = cursor.read_u32::<BigEndian>()?;
		let console_type_flags = ConsoleTypeFlags::from_bits(console_type_raw);

		let mut date_gen_raw = [0u8; 8];
		cursor.read_exact(&mut date_gen_raw)?;
		let date_generation = String::from_utf8_lossy(
			&date_gen_raw[..date_gen_raw.iter().position(|b| *b == 0).unwrap_or(date_gen_raw.len())],
		)
		.into_owned();

		let public_exponent = cursor.read_u32::<BigEndian>()?;
		let mut public_modulus = vec![0u8; 0x80];
		cursor.read_exact(&mut public_modulus)?;
		let mut certificate_signature = vec![0u8; 0x100];
		cursor.read_exact(&mut certificate_signature)?;
		let mut signature = vec![0u8; 0x80];
		cursor.read_exact(&mut signature)?;

		Ok(Certificate {
			pubkey_cert_size,
			owner_console_id,
			owner_console_part_number,
			console_type_flags,
			date_generation,
			public_exponent,
			public_modulus,
			certificate_signature,
			signature,
		})
	}
}

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(Serialize))]
pub struct ConsoleTypeFlags(u32);

bitflags! {
	impl ConsoleTypeFlags: u32 {
		const DEVKIT = 0x1;
		const RETAIL = 0x2;
		const TESTKIT = 0x40000000;
		const RECOVERY_GENERATED = 0x80000000;
	}
}

fn read_wide_string(data: &[u8]) -> String {
	let mut chars = Vec::new();
	for chunk in data.chunks(2) {
		if chunk.len() < 2 {
			break;
		}
		let c = ((chunk[0] as u16) << 8) | chunk[1] as u16;
		if c == 0 {
			break;
		}
		chars.push(c);
	}
	String::from_utf16_lossy(&chars)
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum RsaKeyKind {
	Executable,
	Pirs,
	Live,
	Console,
	Dashboard,
	Manufacturing,
	XMacs,
}

impl RsaKeyKind {
	pub fn private_key(&self, console_kind: ConsoleKind) -> Result<RsaPrivateKey, crate::Error> {
		match console_kind {
			ConsoleKind::Devkit => match self {
				RsaKeyKind::Executable | RsaKeyKind::Pirs => {
					use crate::keys::devkit::xex::*;
					Ok(RsaPrivateKey::from_p_q(
						BigUint::from_slice_native(P.as_slice()),
						BigUint::from_slice_native(Q.as_slice()),
						PUB_EXPONENT.into(),
					)?)
				}
				RsaKeyKind::Live => {
					use crate::keys::devkit::live::*;
					Ok(RsaPrivateKey::from_p_q(
						BigUint::from_slice_native(P.as_slice()),
						BigUint::from_slice_native(Q.as_slice()),
						PUB_EXPONENT.into(),
					)?)
				}
				RsaKeyKind::Console => todo!(),
				RsaKeyKind::Dashboard => todo!(),
				RsaKeyKind::Manufacturing => todo!(),
				RsaKeyKind::XMacs => todo!(),
			},
			ConsoleKind::Retail => match self {
				RsaKeyKind::Executable
				| RsaKeyKind::Pirs
				| RsaKeyKind::Live
				| RsaKeyKind::Dashboard
				| RsaKeyKind::Manufacturing => Err(crate::error::Error::NoPrivateKey(*self, ConsoleKind::Retail)),
				RsaKeyKind::XMacs => todo!(),
				RsaKeyKind::Console => todo!(),
			},
		}
	}

	pub fn public_key(&self, console_kind: ConsoleKind) -> Result<RsaPublicKey, crate::Error> {
		match console_kind {
			ConsoleKind::Devkit => Ok(self.private_key(console_kind)?.to_public_key()),
			ConsoleKind::Retail => match self {
				RsaKeyKind::Executable => todo!(),
				RsaKeyKind::Pirs => todo!(),
				RsaKeyKind::Live => {
					use crate::keys::retail::live::*;
					Ok(RsaPublicKey::new(BigUint::from_slice_native(MODULUS.as_slice()), PUB_EXPONENT.into())?)
				}
				RsaKeyKind::Console => todo!(),
				RsaKeyKind::Dashboard => todo!(),
				RsaKeyKind::Manufacturing => todo!(),
				RsaKeyKind::XMacs => todo!(),
			},
		}
	}

	pub fn verify_signature(&self, console_kind: ConsoleKind, sig: &[u8], hash: &[u8]) -> Result<(), crate::Error> {
		let key = self.public_key(console_kind)?;
		let scheme = pkcs1v15_sha1_scheme();
		let standard_signature = raw_signature_to_standard(sig);
		Ok(key.verify(scheme, hash, &standard_signature)?)
	}

	pub fn sign(&self, console_kind: ConsoleKind, digest: &[u8]) -> Result<Vec<u8>, crate::Error> {
		let key = self.private_key(console_kind)?;
		let scheme = pkcs1v15_sha1_scheme();
		let signature = key.sign(scheme, digest)?;
		Ok(standard_signature_to_raw(signature.as_slice()))
	}
}

impl From<XContentSignatureType> for RsaKeyKind {
	fn from(value: XContentSignatureType) -> Self {
		match value {
			XContentSignatureType::Console => RsaKeyKind::Console,
			XContentSignatureType::Live => RsaKeyKind::Live,
			XContentSignatureType::Pirs => RsaKeyKind::Pirs,
		}
	}
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ConsoleKind {
	Devkit,
	Retail,
}

pub fn verify_xcontent_strong_signature(
	signature_kind: XContentSignatureType,
	signature: &[u8],
	hash: &[u8],
) -> rsa::Result<ConsoleKind> {
	let key_kind: RsaKeyKind = signature_kind.into();
	for console_kind in [ConsoleKind::Retail, ConsoleKind::Devkit] {
		if key_kind.verify_signature(console_kind, signature, hash).is_ok() {
			return Ok(console_kind);
		}
	}
	Err(rsa::Error::Verification)
}

pub fn verify_xcontent_signature(
	signature_kind: XContentSignatureType,
	key_material: &XContentKeyMaterial,
	hash: &[u8],
) -> Result<ConsoleKind, crate::Error> {
	let key_kind: RsaKeyKind = signature_kind.into();
	if key_kind == RsaKeyKind::Console {
		todo!()
	} else if let XContentKeyMaterial::Signature(sig, _reserved) = key_material {
		for console_kind in [ConsoleKind::Retail, ConsoleKind::Devkit] {
			if key_kind.verify_signature(console_kind, sig, hash).is_ok() {
				return Ok(console_kind);
			}
		}
	} else {
		panic!("Key material variant cannot satisfy signature kind {:?}", signature_kind);
	}
	Err(rsa::Error::Verification.into())
}

fn pkcs1v15_sha1_scheme() -> Pkcs1v15Sign {
	// SHA-1 AlgorithmIdentifier DER prefix for PKCS#1 v1.5 signatures
	const SHA1_DER_PREFIX: &[u8] =
		&[0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04, 0x14];
	Pkcs1v15Sign { hash_len: Some(20), prefix: SHA1_DER_PREFIX.into() }
}

fn standard_signature_to_raw(sig: &[u8]) -> Vec<u8> {
	let mut sig = sig.to_vec();
	sig.reverse();
	sig
}

pub fn raw_signature_to_standard(sig: &[u8]) -> Vec<u8> {
	let sig_bignum = BigUint::from_bytes_le(sig);
	sig_bignum.to_bytes_be()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn known_devkit_live_sig() -> (&'static [u8], [u8; 20]) {
		const SIG_RAW: [u8; 0x100] = [
			0x65u8, 0x0B, 0x89, 0xAB, 0xAC, 0x11, 0x6A, 0xBE, 0x5C, 0x8E, 0xF3, 0xAC, 0xF3, 0x37, 0x07, 0x40, 0xB3,
			0x31, 0x3F, 0xE2, 0x42, 0xE4, 0x95, 0x54, 0xBE, 0xD0, 0x7E, 0x54, 0x7E, 0xFD, 0xBB, 0x13, 0x95, 0xFB, 0x7F,
			0xAB, 0x41, 0xEE, 0x76, 0x26, 0x94, 0xDA, 0xAF, 0x1E, 0x68, 0xDE, 0xAC, 0xAD, 0x8D, 0x49, 0xD6, 0xC3, 0xF5,
			0x1F, 0x0F, 0xD7, 0x03, 0x97, 0x9C, 0x40, 0x96, 0xC7, 0xF6, 0xE8, 0x3E, 0x69, 0x2A, 0x25, 0x26, 0x10, 0xD4,
			0x8D, 0x68, 0x3F, 0xCD, 0x68, 0x01, 0x83, 0xC4, 0xF2, 0xF0, 0x00, 0xC2, 0x03, 0x68, 0xE9, 0x5D, 0x76, 0x2A,
			0x03, 0xA4, 0xFE, 0xEF, 0xF8, 0xBD, 0xC7, 0x5A, 0xB9, 0x68, 0x88, 0x1C, 0x93, 0x7B, 0x95, 0xAB, 0x0F, 0xA0,
			0x1E, 0xFB, 0x3B, 0x0D, 0x69, 0x70, 0x2F, 0x12, 0x22, 0x27, 0x7A, 0x15, 0x9A, 0xB1, 0x22, 0x9A, 0x79, 0xC8,
			0xEB, 0x08, 0xF3, 0xB0, 0x19, 0x13, 0x53, 0x41, 0xE3, 0xD0, 0xD2, 0xCE, 0x8B, 0xCD, 0xBF, 0xEB, 0xE2, 0x0A,
			0x58, 0x44, 0xAA, 0x08, 0x76, 0x96, 0xCA, 0xA6, 0x8B, 0x05, 0x6D, 0x70, 0xBA, 0xE5, 0xC2, 0xBA, 0x1A, 0x4C,
			0x1A, 0xE4, 0xD3, 0x45, 0xE2, 0x74, 0xFB, 0x2D, 0x1A, 0xB5, 0x54, 0xA9, 0xBD, 0x44, 0x63, 0xA4, 0x55, 0xDF,
			0x0F, 0x03, 0x14, 0x14, 0xC8, 0x6F, 0x26, 0x5D, 0x85, 0x9C, 0x26, 0x60, 0x81, 0x45, 0xCC, 0x3B, 0x29, 0x14,
			0xCE, 0xC7, 0xA7, 0x81, 0x77, 0x4F, 0x11, 0x0E, 0xB5, 0xAD, 0x78, 0x06, 0x34, 0x7E, 0x3B, 0x21, 0x77, 0x1F,
			0xF7, 0x92, 0x3D, 0xC0, 0xE5, 0x1A, 0xB5, 0xA9, 0x4F, 0x7C, 0xA8, 0x39, 0x14, 0x64, 0x86, 0x16, 0x4E, 0xC5,
			0x80, 0x66, 0x3A, 0x8D, 0x6C, 0x1A, 0x51, 0x3A, 0x4A, 0xCD, 0xBD, 0x8D, 0xA9, 0x63, 0xFC, 0xD2, 0xDD, 0x41,
			0xA1, 0xD3, 0x04, 0x82, 0x96,
		];

		let hash = [
			0xb6, 0x74, 0x4c, 0x85, 0x9a, 0xb7, 0x68, 0xcc, 0xea, 0x41, 0x65, 0x13, 0x2e, 0x0c, 0x9c, 0x7a, 0x3c, 0xa5,
			0xdf, 0x2b,
		];

		(SIG_RAW.as_slice(), hash)
	}

	#[test]
	fn verify_xcontent_signature_works() {
		let (sig, hash) = known_devkit_live_sig();
		assert_eq!(
			ConsoleKind::Devkit,
			verify_xcontent_strong_signature(XContentSignatureType::Live, sig, hash.as_slice())
				.expect("failed to verify known devkit LIVE signature")
		);
	}

	#[test]
	fn devkit_live_key_validates_known_sig() {
		let key = RsaKeyKind::Live;
		let (sig, hash) = known_devkit_live_sig();
		key.verify_signature(ConsoleKind::Devkit, sig, &hash).expect("signature verification failed");
	}

	#[test]
	fn verify_round_trip_signature_conversion_works() {
		let (sig, _) = known_devkit_live_sig();
		assert_eq!(standard_signature_to_raw(raw_signature_to_standard(sig).as_slice()), sig);
	}

	#[test]
	fn verify_signing_works() {
		let (sig, hash) = known_devkit_live_sig();
		let key = RsaKeyKind::Live;
		let digest = key.sign(ConsoleKind::Devkit, &hash).expect("signing failed");
		assert_eq!(digest.as_slice(), sig);
	}
}
