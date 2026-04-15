pub mod bytes;

use std::io::Write;

use crate::error::StfsError;
use crate::file_table::StfsFileEntry;
use crate::package::StfsPackage;

pub trait StfsPackageReader {
	fn package(&self) -> &StfsPackage;
	fn extract_file<W: Write>(&self, writer: &mut W, entry: &StfsFileEntry) -> Result<(), StfsError>;
}
