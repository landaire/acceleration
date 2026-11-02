//! Parser for Xbox 360 XContent packages (CON, LIVE, PIRS).
//!
//! XContent packages wrap an [STFS](https://docs.rs/stfs) filesystem in a
//! signed header containing the content type, title metadata, license
//! entries, display names in all supported languages, and thumbnail images.
//!
//! The package magic identifies the signature type:
//! - `"CON "` -- console-signed (by a specific Xbox 360's keyvault)
//! - `"LIVE"` -- Microsoft-signed (LIVE marketplace content)
//! - `"PIRS"` -- Microsoft-signed (offline content like patches)
//!
//! # Example
//!
//! ```no_run
//! use xcontent::XContentPackage;
//!
//! let data = std::fs::read("savegame.bin")?;
//! let package = XContentPackage::parse(&data)?;
//!
//! println!("Title:   {:?}", package.header.metadata.title_name);
//! println!("Type:    {:?}", package.header.metadata.content_type);
//! println!("Version: {}", package.header.metadata.version);
//!
//! // A LIVE/PIRS signature is tried with the retail and devkit keys; a CON signature reports
//! // its certificate and package signature checks separately.
//! match package.verify_signature(&data) {
//!     Ok(verification) if verification.is_verified() => println!("Verified: {verification:?}"),
//!     Ok(verification) => println!("Not verified: {verification:?}"),
//!     Err(e) => println!("Signature invalid: {}", e),
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod error;
mod parse;

pub use crate::error::XContentError;
pub use crate::parse::*;
pub use xecrypt;
