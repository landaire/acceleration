use thiserror::Error;

#[derive(Debug, Error)]
pub enum SystemUpdateError {
	#[error("the update package could not be parsed")]
	Package(#[source] xcontent::XContentError),
	#[error("the update package signature does not verify")]
	PackageSignature,
	#[error(
		"the package has content type {content_type:#x} and installer type {installer_type:?}, not a system update"
	)]
	NotSystemUpdate { content_type: u32, installer_type: Option<u32> },
	#[error("package file {name} could not be read")]
	PackageFile { name: String, source: stfs::StfsError },
	#[error("delta patch {patch} could not be applied to {base}")]
	DeltaPatch { patch: String, base: String },
	#[error("{name} is a patched module, not a XEX file")]
	PatchedBase { name: String },
	#[error("{name} is not in the flash file set")]
	MissingFile { name: String },
	#[error("module {name} could not be loaded")]
	Module { name: String },
	#[error("resource \"{resource}\" of module {module} at {address:#010x} with size {size:#x} is outside the image")]
	ResourceOutsideImage { module: String, resource: String, address: u32, size: u32 },
}
