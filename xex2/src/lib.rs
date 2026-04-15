pub mod basefile;
pub mod crypto;
pub mod error;
pub mod header;
pub mod idc;
pub mod imports;
pub mod kernel_exports;
pub mod opt;
pub mod writer;
pub mod xml;

use crate::error::Result;
use crate::header::SecurityInfo;
use crate::header::Xex2Header;

pub struct Xex2 {
	pub header: Xex2Header,
	pub security_info: SecurityInfo,
	raw: Vec<u8>,
}

impl Xex2 {
	pub fn parse(data: Vec<u8>) -> Result<Self> {
		let header = Xex2Header::parse(&data)?;
		let security_info = SecurityInfo::parse(&data, header.security_offset as usize)?;
		Ok(Xex2 { header, security_info, raw: data })
	}

	pub fn raw(&self) -> &[u8] {
		&self.raw
	}

	pub fn extract_basefile(&self) -> Result<Vec<u8>> {
		basefile::extract_basefile(&self.raw, &self.header, &self.security_info)
	}

	pub fn generate_idc(&self) -> String {
		idc::generate_idc(&self.header, self.security_info.image_info.load_address.0, self.security_info.image_size)
	}

	pub fn to_xml(&self) -> String {
		xml::generate_xml(self)
	}

	pub fn modify(&self, limits: &writer::RemoveLimits) -> Result<Vec<u8>> {
		writer::modify_xex(
			self,
			writer::TargetEncryption::Unchanged,
			writer::TargetCompression::Unchanged,
			writer::TargetMachine::Unchanged,
			limits,
		)
	}
}
