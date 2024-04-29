use thiserror::Error;

#[derive(Error, Debug)]
pub enum StfsError {
	#[error("I/O error")]
	Io(#[from] std::io::Error),
	#[error("Invalid STFS volume descriptor")]
	InvalidVolumeDescriptor,
	#[error("I/O error (binrw)")]
	Binrw(#[from] binrw::Error),
}
