use thiserror::Error;

#[derive(Error, Debug)]
pub enum StfsError {
	#[error("Invalid STFS package header")]
	InvalidHeader,
	#[error("I/O error: {0}")]
	Io(#[from] std::io::Error),
	#[error("Invalid package type")]
	InvalidPackageType,
	#[error("Read error at offset {offset:#X}: {message}")]
	ReadError { offset: usize, message: String },
	#[error("Invalid block reference: block {block:#X} (allocated: {allocated:#X})")]
	InvalidBlock { block: usize, allocated: usize },
	#[error("Corrupt file table: {0}")]
	CorruptFileTable(String),
	#[error("Source read failed: {0}")]
	SourceError(Box<dyn std::error::Error + Send + Sync>),
}
