//! The FATX commands of the `fatxtool` and `xflash` command-line tools: listing, extraction,
//! forensic scans, recovery of deleted files, and volume information.
//!
//! [`run`] runs a [`FatxCommand`] against a volume at a [`VolumeLocation`] of any
//! [`VolumeSource`]: an image file, a memory unit partition, or a volume file read from flash.
//! The commands never write to the source, and an output that would replace the input file is
//! refused.

pub mod command;
pub mod describe;
pub mod error;
pub mod extract;
pub mod info;
pub mod list;
pub mod location;
pub mod number;
pub mod output;
pub mod recover;
pub mod scan;
pub mod session;
pub mod source;

pub use command::FatxCommand;
pub use command::StrategyArgument;
pub use command::run;
pub use error::FatxToolError;
pub use error::NumberError;
pub use error::OutputError;
pub use location::VolumeLocation;
pub use number::parse_number;
pub use output::InputFile;
pub use output::create_output;
pub use session::Session;
pub use source::VolumeSource;
