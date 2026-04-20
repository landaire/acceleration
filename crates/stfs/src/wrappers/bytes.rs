use std::io::Write;

use crate::error::StfsError;
use crate::file_table::StfsFileEntry;
use crate::io::SliceReader;
use crate::package::StfsPackage;
use crate::wrappers::StfsPackageReader;

pub struct BytesStfsReader<T> {
	source: T,
	package: StfsPackage,
}

impl<T: AsRef<[u8]>> BytesStfsReader<T> {
	pub fn open(source: T) -> Result<Self, StfsError> {
		let reader = SliceReader(source.as_ref());
		let package = StfsPackage::open(&reader)?;
		Ok(Self { source, package })
	}
}

impl<T: AsRef<[u8]>> StfsPackageReader for BytesStfsReader<T> {
	fn package(&self) -> &StfsPackage {
		&self.package
	}

	fn extract_file<W: Write>(&self, writer: &mut W, entry: &StfsFileEntry) -> Result<(), StfsError> {
		let reader = SliceReader(self.source.as_ref());
		self.package.extract_file(&reader, writer, entry)
	}
}
