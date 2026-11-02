//! The FATX commands of the `fatxtool` and `xflash` command-line tools: listing, extraction,
//! forensic scans, recovery of deleted files, and volume information.
//!
//! [`run`] runs a [`FatxCommand`] against a volume at a [`VolumeLocation`] of any
//! [`VolumeSource`]: an image file, a memory unit partition, or a volume file read from flash.
//! These commands never write to the source, and an output that would replace the input file is
//! refused.
//!
//! [`partitions`] prints the layout of the device an image holds, [`select`] finds a volume by
//! its partition name, and [`partition_table`] writes a devkit partition table to an image file.
//!
//! [`write`] formats a volume and creates, replaces, deletes and moves its entries in a
//! [`write::WritableImage`]: a regular image file, never a device.

pub mod command;
pub mod describe;
pub mod error;
pub mod extract;
pub mod info;
pub mod list;
pub mod location;
pub mod number;
pub mod output;
pub mod partition_table;
pub mod partitions;
pub mod recover;
pub mod scan;
pub mod select;
pub mod session;
pub mod source;
pub mod usb_files;
pub mod write;

pub use command::FatxCommand;
pub use command::StrategyArgument;
pub use command::run;
pub use error::DiskToolError;
pub use error::FatxToolError;
pub use error::NumberError;
pub use error::OutputError;
pub use location::VolumeLocation;
pub use number::parse_number;
pub use output::InputFile;
pub use output::create_output;
pub use partition_table::PartitionTableCommand;
pub use select::SelectedPartition;
pub use select::select_partition;
pub use session::Session;
pub use source::VolumeSource;
pub use write::WriteCommand;
