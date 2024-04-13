use thiserror::Error;

#[derive(Error, Debug)]
pub enum StfsError {
	#[error("Invalid STFS package magic")]
	InvalidMagic,
	#[error("Invalid STFS package header")]
	InvalidHeader,
	#[error("Invalid package type")]
	InvalidPackageType,
	#[error("I/O error")]
	Io(#[from] std::io::Error),
	#[error("I/O error (binrw)")]
	Binrw(#[from] binrw::Error),
}
