pub mod basefile;
pub mod crypto;
pub mod error;
pub mod header;

use crate::error::Result;
use crate::header::{SecurityInfo, Xex2Header};

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
}
