//! Non-FFI-visible implementation layer. Caches the expensive derived
//! bits (imports, resources, optional-header values parsed out of their
//! raw blobs) up front so the thousand-ish small getters on the FFI side
//! don't have to re-decode on every call.

use std::sync::Arc;
use xex2::Xex2;
use xex2::header::{ExecutionInfo, FileFormatInfo};
use xex2::imports::ImportLibrary;
use xex2::opt::ResourceEntry;
use xex2::writer::RemoveLimits;

pub struct Xex2Inner {
	pub parsed: Xex2,
	pub bytes: Vec<u8>,
	/// Populated imports (may be empty). Arc so we can hand a shared
	/// view to `Xex2Imports` / per-library opaques without deep-cloning.
	pub imports: Arc<Vec<ImportLibrary>>,
	/// Populated resources (may be empty). Arc for the same reason.
	pub resources: Arc<Vec<ResourceEntry>>,
	pub exec_info: Option<ExecutionInfo>,
	pub file_format: Option<FileFormatInfo>,
	pub bounding_path: Option<String>,
}

pub fn parse(bytes: &[u8]) -> Result<Xex2Inner, String> {
	let parsed = Xex2::parse(bytes).map_err(|e| e.to_string())?;
	let imports = parsed.header.import_table().map(|t| t.libraries).unwrap_or_default();
	let resources = parsed.header.resource_info().map(|r| r.resources).unwrap_or_default();
	let exec_info = parsed.header.execution_info();
	let file_format = parsed.header.file_format_info().ok();
	let bounding_path = parsed.header.bounding_path();
	Ok(Xex2Inner {
		parsed,
		bytes: bytes.to_vec(),
		imports: Arc::new(imports),
		resources: Arc::new(resources),
		exec_info,
		file_format,
		bounding_path,
	})
}

pub fn extract_basefile(inner: &Xex2Inner) -> Result<Vec<u8>, String> {
	inner.parsed.extract_basefile(&inner.bytes).map_err(|e| e.to_string())
}

/// `Xex2::modify` consumes `self`, so re-parse from the cached bytes
/// rather than wiring `Clone` through the upstream crate.
pub fn modify(inner: &Xex2Inner, limits: &RemoveLimits) -> Result<Vec<u8>, String> {
	let fresh = Xex2::parse(&inner.bytes).map_err(|e| e.to_string())?;
	fresh.modify(&inner.bytes, limits).map_err(|e| e.to_string())
}
