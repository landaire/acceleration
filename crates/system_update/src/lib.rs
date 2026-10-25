//! Applies an Xbox 360 system update package to the files of a system flash
//! volume.
//!
//! The caller supplies the flash files, usually memory maps of a base
//! system's files or of files already updated, and optionally a system update
//! package. [`FlashFiles::apply_update`] installs the package's `$flash_`
//! files and applies its delta patches through [`xex2::delta`];
//! [`FlashFiles::module`] loads a module and gives access to its resources.
//!
//! ```no_run
//! use rootcause::Report;
//! use system_update::FlashFile;
//! use system_update::FlashFiles;
//! use system_update::SystemUpdateError;
//! use system_update::UpdatePackage;
//!
//! fn install(xam: &[u8], font: &[u8], package: &[u8]) -> Result<(), Report<SystemUpdateError>> {
//!     let mut files = FlashFiles::new();
//!     files.insert("xam.xex", FlashFile::Supplied(xam));
//!     files.insert("xenonclatin.xtt", FlashFile::Supplied(font));
//!     files.apply_update(&UpdatePackage::parse(package)?)?;
//!
//!     let xam = files.module("xam.xex")?;
//!     let shared_resources = xam.resource("shrdres")?;
//!     let font = files.get("XenonCLatin.xtt").and_then(|file| file.bytes());
//!     # let _ = (shared_resources, font);
//!     Ok(())
//! }
//! ```

mod error;
mod flash;
mod package;

pub use error::SystemUpdateError;
pub use flash::AppliedUpdate;
pub use flash::FlashFile;
pub use flash::FlashFiles;
pub use flash::FlashName;
pub use flash::Module;
pub use package::PackageFile;
pub use package::UpdatePackage;
