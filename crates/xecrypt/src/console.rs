//! The console certificate and console signature (`XE_CONSOLE_CERTIFICATE` and
//! `XE_CONSOLE_SIGNATURE`, `kernel/inc/xekeys.h`). The keyvault holds the console's own
//! certificate; CON packages and the hard disk's utility drive block carry a certificate with a
//! signature made by the console's private key (`XeKeysConsolePrivateKeySign`).

use std::fmt;

use bitflags::bitflags;
use rsa::BigUint;
use rsa::Pkcs1v15Sign;
use rsa::RsaPublicKey;
use sha1::Digest;
use sha1::Sha1;
use xenon_types::ConsoleId;
use xenon_types::FixedText;

use crate::error::ConsoleSignatureError;

pub const CONSOLE_CERTIFICATE_SIZE: usize = 0x1a8;
pub const CONSOLE_SIGNATURE_SIZE: usize = 0x228;
/// The certificate bytes its own signature covers: every field before it.
const CERTIFICATE_SIGNED_LENGTH: usize = 0xa8;

bitflags! {
	/// `XE_CONSOLE_CERTIFICATE.Privileges`, the u16 at certificate offset 0x16.
	///
	/// Covered by the certificate signature. The kernel passes the whole certificate to
	/// `XeKeysSetKey` without testing bits of this field. One test keyvault has 0x4000.
	#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
	pub struct CertificatePrivileges: u16 {
		const _ = !0;
	}
}

#[cfg(feature = "serde")]
impl serde::Serialize for CertificatePrivileges {
	fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
		s.serialize_u16(self.bits())
	}
}

/// `XE_CONSOLE_CERTIFICATE.ConsoleType`, the u32 at certificate offset 0x18.
///
/// Part of the signed console certificate. The HV and kernel verify consistency of this field
/// between the local certificate and any incoming certificate during
/// `XeKeysConsoleSignatureVerification`:
///
/// - **Retail path** (HV flags bit 15 clear): bits 0-23 must match exactly between the two
///   certificates.
/// - **Devkit path** (HV flags bit 15 set): bits 0-23 and bit 30 (TESTKIT) must match. Bits
///   24-29 may differ. Bit 31 (RECOVERY_GENERATED) is explicitly ignored in the comparison.
///
/// | Bits  | Mask         | Name               | Description |
/// |-------|--------------|--------------------|-------------|
/// | 0-1   | 0x0000_0003  | Console kind       | 1=devkit, 2=retail |
/// | 2-23  | 0x00FF_FFFC  | Reserved           | Must match between certs |
/// | 24-26 | 0x0700_0000  | (variable)         | May differ on devkit; must match on retail |
/// | 27    | 0x0800_0000  | ODD auth type      | Checked in `HvxKeysInitialize`. May be cleared based on OddAuthType bit 2. |
/// | 28    | 0x1000_0000  | (variable)         | Checked in `HvxKeysInitialize` (0x77d8). May differ on devkit. |
/// | 29    | 0x2000_0000  | (reserved)         | May differ on devkit |
/// | 30    | 0x4000_0000  | Testkit            | Console is a test kit. Must match between certs on both retail and devkit. |
/// | 31    | 0x8000_0000  | Recovery generated | `XE_CONSOLE_TYPE_KEY_3A`: key 0x3a signs the certificate instead of 0x3c. Ignored in cert comparison. |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ConsoleType(pub u32);

impl ConsoleType {
	/// Console kind mask (bits 0-1).
	pub const KIND_MASK: u32 = 0x0000_0003;
	/// Bit 27: ODD authentication type. Checked and potentially cleared in
	/// `HvxKeysInitialize` based on OddAuthType bit 2.
	pub const ODD_AUTH_TYPE: u32 = 0x0800_0000;
	/// Bit 30: Console is a test kit.
	pub const TESTKIT: u32 = 0x4000_0000;
	/// Bit 31: `XE_CONSOLE_TYPE_KEY_3A`. The certificate was recovery-generated and key 0x3a
	/// signs it.
	pub const RECOVERY_GENERATED: u32 = 0x8000_0000;

	pub fn kind(&self) -> ConsoleKind {
		match self.0 & Self::KIND_MASK {
			1 => ConsoleKind::Devkit,
			2 => ConsoleKind::Retail,
			_ => ConsoleKind::Unknown,
		}
	}

	pub fn is_devkit(&self) -> bool {
		self.0 & Self::KIND_MASK == 1
	}

	pub fn is_retail(&self) -> bool {
		self.0 & Self::KIND_MASK == 2
	}

	pub fn is_testkit(&self) -> bool {
		self.0 & Self::TESTKIT != 0
	}

	pub fn is_recovery_generated(&self) -> bool {
		self.0 & Self::RECOVERY_GENERATED != 0
	}

	/// The hypervisor key that signs the certificate (`XeKeysConsoleSignatureVerification`).
	pub fn certificate_key(&self) -> CertificateKey {
		if self.is_recovery_generated() { CertificateKey::Key3A } else { CertificateKey::Key3C }
	}
}

/// The kind of console a certificate names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ConsoleKind {
	Devkit,
	Retail,
	Unknown,
}

/// The hypervisor public key that signs a console certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum CertificateKey {
	/// `XEKEYS_KEY_PUBLIC_KEY_3A`, for recovery-generated certificates. Retail hypervisors hold
	/// it as zero and this crate does not have it.
	Key3A,
	/// `XEKEYS_KEY_PUBLIC_KEY_3C`.
	Key3C,
}

impl fmt::Display for CertificateKey {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Key3A => f.write_str("0x3a"),
			Self::Key3C => f.write_str("0x3c"),
		}
	}
}

/// What checking a certificate's own signature with its authority key found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum CertificateStatus {
	Verified {
		key: CertificateKey,
	},
	/// The key the certificate's console type selects is not in this crate, so the certificate
	/// is unchecked.
	KeyUnavailable {
		key: CertificateKey,
	},
	/// The authority key does not verify the certificate.
	Invalid {
		key: CertificateKey,
	},
}

impl CertificateStatus {
	pub const fn is_verified(&self) -> bool {
		matches!(self, Self::Verified { .. })
	}
}

impl fmt::Display for CertificateStatus {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Verified { key } => write!(f, "verified with key {key}"),
			Self::KeyUnavailable { key } => write!(f, "not verified: key {key} is not available"),
			Self::Invalid { key } => write!(f, "invalid: key {key} does not verify it"),
		}
	}
}

/// The two checks of `XeKeysConsoleSignatureVerification` that need no console state: the
/// certificate against its authority key, and the signature against the certificate's key. The
/// console type comparison with the verifying console's own certificate and the revocation list
/// are not made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ConsoleSignatureVerification {
	pub certificate: CertificateStatus,
	#[cfg_attr(feature = "serde", serde(serialize_with = "serialize_signature_result"))]
	pub signature: Result<(), ConsoleSignatureError>,
}

impl ConsoleSignatureVerification {
	/// Whether both the certificate and the signature verified.
	pub const fn is_verified(&self) -> bool {
		self.certificate.is_verified() && self.signature.is_ok()
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum ConsoleRevision {
	/// Original Xbox 360 (2005). 90nm CPU/GPU.
	Xenon,
	/// Second revision (2007). 80nm GPU, HDMI added.
	Zephyr,
	/// Third revision (2007). 65nm CPU.
	Falcon,
	/// Fourth revision (2008). 65nm CPU, 65nm GPU.
	Jasper,
	/// Xbox 360 S "Slim" (2010). CPU+GPU combined into single die.
	Trinity,
	/// Xbox 360 S revision (2011). Smaller combined die.
	Corona,
	/// Xbox 360 E (2013). Final hardware revision.
	Winchester,
	Unknown,
}

/// `XE_CONSOLE_CERTIFICATE`: the console's identity and RSA-1024 public key, signed by a
/// hypervisor key (see [`ConsoleCertificate::verify`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ConsoleCertificate {
	pub cert_size: u16,
	pub console_id: ConsoleId,
	pub console_part_number: FixedText<11>,
	#[cfg_attr(feature = "serde", serde(serialize_with = "serialize_hex"))]
	pub reserved: [u8; 4],
	pub privileges: CertificatePrivileges,
	pub console_type: ConsoleType,
	pub manufacturing_date: FixedText<8>,
	pub public_exponent: u32,
	/// The modulus as XeCrypt stores it: 64-bit big-endian words, least significant first.
	#[cfg_attr(feature = "serde", serde(serialize_with = "serialize_hex"))]
	pub public_modulus: [u8; 0x80],
	/// RSA-2048 signature over the first 0xa8 bytes by [`ConsoleType::certificate_key`].
	#[cfg_attr(feature = "serde", serde(serialize_with = "serialize_hex"))]
	pub signature: [u8; 0x100],
}

impl ConsoleCertificate {
	pub fn parse(bytes: &[u8; CONSOLE_CERTIFICATE_SIZE]) -> Self {
		Self {
			cert_size: u16::from_be_bytes(array(bytes, 0x00)),
			console_id: ConsoleId(array(bytes, 0x02)),
			console_part_number: FixedText::new(array(bytes, 0x07)),
			reserved: array(bytes, 0x12),
			privileges: CertificatePrivileges::from_bits_retain(u16::from_be_bytes(array(bytes, 0x16))),
			console_type: ConsoleType(u32::from_be_bytes(array(bytes, 0x18))),
			manufacturing_date: FixedText::new(array(bytes, 0x1c)),
			public_exponent: u32::from_be_bytes(array(bytes, 0x24)),
			public_modulus: array(bytes, 0x28),
			signature: array(bytes, CERTIFICATE_SIGNED_LENGTH),
		}
	}

	/// Checks the certificate's own signature with the authority key its console type selects, as
	/// `XeKeysConsoleSignatureVerification` does.
	pub fn verify(&self) -> CertificateStatus {
		let key = self.console_type.certificate_key();
		let authority = match key {
			CertificateKey::Key3C => crate::RsaKeyKind::Console
				.public_key(crate::ConsoleKind::Retail)
				.expect("the console certificate authority key is a valid RSA key"),
			CertificateKey::Key3A => return CertificateStatus::KeyUnavailable { key },
		};

		let digest: [u8; 20] = Sha1::digest(self.signed_bytes()).into();
		if xecrypt_pkcs1_verify(&authority, &digest, &self.signature) {
			CertificateStatus::Verified { key }
		} else {
			CertificateStatus::Invalid { key }
		}
	}

	/// The bytes the certificate's own signature covers, as stored.
	pub fn signed_bytes(&self) -> [u8; CERTIFICATE_SIGNED_LENGTH] {
		let mut bytes = [0u8; CERTIFICATE_SIGNED_LENGTH];
		bytes[0x00..0x02].copy_from_slice(&self.cert_size.to_be_bytes());
		bytes[0x02..0x07].copy_from_slice(&self.console_id.0);
		bytes[0x07..0x12].copy_from_slice(self.console_part_number.bytes());
		bytes[0x12..0x16].copy_from_slice(&self.reserved);
		bytes[0x16..0x18].copy_from_slice(&self.privileges.bits().to_be_bytes());
		bytes[0x18..0x1c].copy_from_slice(&self.console_type.0.to_be_bytes());
		bytes[0x1c..0x24].copy_from_slice(self.manufacturing_date.bytes());
		bytes[0x24..0x28].copy_from_slice(&self.public_exponent.to_be_bytes());
		bytes[0x28..0xa8].copy_from_slice(&self.public_modulus);

		bytes
	}

	pub fn revision(&self) -> ConsoleRevision {
		revision_from_part_number(&self.console_part_number.to_string())
	}

	/// The console's RSA-1024 public key.
	pub fn public_key(&self) -> Result<RsaPublicKey, ConsoleSignatureError> {
		// Reversing the order of the words gives the big-endian number.
		let mut modulus = [0u8; 0x80];
		let words = self.public_modulus.as_chunks::<8>().0.iter().rev();
		for (target, source) in modulus.as_chunks_mut::<8>().0.iter_mut().zip(words) {
			*target = *source;
		}

		RsaPublicKey::new(BigUint::from_bytes_be(&modulus), BigUint::from(self.public_exponent))
			.map_err(|_| ConsoleSignatureError::InvalidPublicKey { exponent: self.public_exponent })
	}
}

/// `XE_CONSOLE_SIGNATURE`: a certificate and an RSA-1024 signature made with the private key of
/// the console it names.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ConsoleSignature {
	pub certificate: ConsoleCertificate,
	/// Stored least significant byte first, as XeCrypt stores signatures.
	#[cfg_attr(feature = "serde", serde(serialize_with = "serialize_hex"))]
	pub signature: [u8; 0x80],
}

impl ConsoleSignature {
	pub fn parse(bytes: &[u8; CONSOLE_SIGNATURE_SIZE]) -> Self {
		Self {
			certificate: ConsoleCertificate::parse(&array(bytes, 0)),
			signature: array(bytes, CONSOLE_CERTIFICATE_SIZE),
		}
	}

	/// Checks the certificate with its authority key and the signature over `digest` with the
	/// certificate's public key (see [`ConsoleSignatureVerification`]).
	pub fn verify(&self, digest: &[u8; 20]) -> ConsoleSignatureVerification {
		let signature = self.certificate.public_key().and_then(|key| {
			if xecrypt_pkcs1_verify(&key, digest, &self.signature) {
				Ok(())
			} else {
				Err(ConsoleSignatureError::Mismatch)
			}
		});

		ConsoleSignatureVerification { certificate: self.certificate.verify(), signature }
	}
}

/// `XeKeysPkcs1Verify`: PKCS #1 v1.5 over a signature stored least significant byte first, with
/// one of the three encodings `XeCryptBnDwLePkcs1Verify` accepts: a SHA-1 DigestInfo with NULL
/// parameters, one without them, or the bare digest. Console certificates use the bare digest.
fn xecrypt_pkcs1_verify(key: &RsaPublicKey, digest: &[u8; 20], stored_signature: &[u8]) -> bool {
	const WITH_NULL: [u8; 15] =
		[0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04, 0x14];
	const WITHOUT_NULL: [u8; 13] = [0x30, 0x1f, 0x30, 0x07, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x04, 0x14];

	let mut signature = stored_signature.to_vec();
	signature.reverse();

	[&WITH_NULL[..], &WITHOUT_NULL[..], &[][..]].into_iter().any(|prefix| {
		let scheme = Pkcs1v15Sign { hash_len: Some(digest.len()), prefix: prefix.into() };
		key.verify(scheme, digest, &signature).is_ok()
	})
}

pub(crate) fn revision_from_part_number(pn: &str) -> ConsoleRevision {
	if pn.starts_with("X803") || pn.starts_with("X800") {
		if pn.contains("955") || pn.contains("953") {
			return ConsoleRevision::Xenon;
		}
		if pn.contains("885") || pn.contains("878") {
			return ConsoleRevision::Zephyr;
		}
	}
	if pn.starts_with("X804") || pn.starts_with("X810") {
		return ConsoleRevision::Falcon;
	}
	if pn.starts_with("X811") || pn.starts_with("X812") || pn.starts_with("X815") {
		return ConsoleRevision::Jasper;
	}
	if pn.starts_with("X816") || pn.starts_with("X818") {
		return ConsoleRevision::Trinity;
	}
	if pn.starts_with("X819") || pn.starts_with("X820") || pn.starts_with("X850") {
		return ConsoleRevision::Corona;
	}
	if pn.starts_with("X851") || pn.starts_with("X852") || pn.starts_with("X86") {
		return ConsoleRevision::Winchester;
	}
	if pn.starts_with("004") {
		return ConsoleRevision::Xenon;
	}
	ConsoleRevision::Unknown
}

fn array<const N: usize>(bytes: &[u8], offset: usize) -> [u8; N] {
	let mut field = [0; N];
	field.copy_from_slice(&bytes[offset..offset + N]);

	field
}

#[cfg(feature = "serde")]
fn serialize_hex<S, const N: usize>(bytes: &[u8; N], serializer: S) -> Result<S::Ok, S::Error>
where
	S: serde::Serializer,
{
	struct Hex<'a>(&'a [u8]);

	impl std::fmt::Display for Hex<'_> {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
		}
	}

	serializer.collect_str(&Hex(bytes))
}

/// The signature result as `verified` or the error text.
#[cfg(feature = "serde")]
fn serialize_signature_result<S>(result: &Result<(), ConsoleSignatureError>, serializer: S) -> Result<S::Ok, S::Error>
where
	S: serde::Serializer,
{
	match result {
		Ok(()) => serializer.serialize_str("verified"),
		Err(error) => serializer.collect_str(error),
	}
}
