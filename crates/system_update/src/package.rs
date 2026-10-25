use rootcause::IntoReport;
use rootcause::Report;
use stfs::SliceReader;
use stfs::StfsFileEntry;
use stfs::StfsPackage;
use xcontent::ContentType;
use xcontent::InstallerType;
use xcontent::XContentHeader;
use xcontent::XContentPackage;
use xcontent::XboxFilesystem;

use crate::error::SystemUpdateError;

/// A signed system update package (content type Installer, installer type
/// `SUPD`) over the caller's package bytes, usually a memory map.
pub struct UpdatePackage<'a> {
	data: &'a [u8],
	header: XContentHeader,
	filesystem: StfsPackage,
}

/// A file in the root of an update package.
#[derive(Debug, Clone)]
pub struct PackageFile {
	entry: StfsFileEntry,
}

impl PackageFile {
	pub fn name(&self) -> &str {
		&self.entry.name
	}

	pub fn size(&self) -> usize {
		self.entry.file_size
	}
}

impl<'a> UpdatePackage<'a> {
	/// Parse the package and verify its LIVE or PIRS signature.
	pub fn parse(data: &'a [u8]) -> Result<Self, Report<SystemUpdateError>> {
		let package = XContentPackage::parse(data).map_err(|e| SystemUpdateError::Package(e).into_report())?;
		package.verify_signature(data).map_err(|_| SystemUpdateError::PackageSignature.into_report())?;

		let metadata = &package.header.metadata;
		let is_system_update = metadata.content_type == ContentType::Installer
			&& matches!(metadata.installer_type, Some(InstallerType::SystemUpdate));
		if !is_system_update {
			return Err(SystemUpdateError::NotSystemUpdate {
				content_type: metadata.content_type as u32,
				installer_type: metadata.installer_type.map(|installer| installer as u32),
			}
			.into_report());
		}

		let XboxFilesystem::Stfs(filesystem) = package.inner_file_system;

		Ok(Self { data, header: package.header, filesystem })
	}

	pub fn header(&self) -> &XContentHeader {
		&self.header
	}

	/// The files in the package root, in file table order.
	pub fn files(&self) -> impl Iterator<Item = PackageFile> {
		self.filesystem
			.file_table
			.walk_files()
			.into_iter()
			.filter(|walk| !walk.path.contains('/'))
			.map(|walk| PackageFile { entry: walk.entry })
	}

	pub fn file(&self, name: &str) -> Option<PackageFile> {
		self.files().find(|file| file.name() == name)
	}

	/// Copy a file out of the package. STFS interleaves hash tables with the
	/// data, so the contents are not one slice of the package.
	pub fn read(&self, file: &PackageFile) -> Result<Vec<u8>, Report<SystemUpdateError>> {
		let mut contents = Vec::with_capacity(file.size());
		self.filesystem
			.extract_file(&SliceReader(self.data), &mut contents, &file.entry)
			.map_err(|source| SystemUpdateError::PackageFile { name: file.name().to_string(), source }.into_report())?;

		Ok(contents)
	}
}
