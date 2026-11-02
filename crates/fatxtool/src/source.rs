use std::convert::Infallible;
use std::fmt;

use fatx::ReadAt;
use fatx::io::SliceSource;
use xenon_io::FileSource;

/// A source of volume bytes whose reads can fail integrity checks without failing, such as a
/// memory unit read through the flash ECC.
pub trait VolumeSource: ReadAt {
	type Warning: fmt::Display + fmt::Debug + Send + Sync + 'static;

	/// The failed checks of the reads since the last call.
	fn take_warnings(&self) -> Vec<Self::Warning>;
}

impl<T> VolumeSource for SliceSource<T>
where
	T: AsRef<[u8]>,
{
	type Warning = Infallible;

	fn take_warnings(&self) -> Vec<Infallible> {
		// Bytes in memory have no integrity checks.
		Vec::new()
	}
}

impl VolumeSource for FileSource {
	type Warning = Infallible;

	fn take_warnings(&self) -> Vec<Infallible> {
		// An image file has no integrity checks of its own.
		Vec::new()
	}
}
