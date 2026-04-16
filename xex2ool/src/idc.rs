//! IDA Pro IDC script generation.
//!
//! Generates an IDC script that labels kernel import thunks with their
//! Microsoft names (e.g. `KeGetCurrentProcessType`, `XamLoaderLaunchTitle`)
//! and sets up the load address for the extracted basefile.

use std::fmt::Write;

use xex2::header::Xex2Header;
use xex2::imports::ImportLibrary;

pub fn generate_idc(header: &Xex2Header, load_address: u32, image_size: u32) -> String {
	let mut out = String::new();

	writeln!(out, "#include <idc.idc>").unwrap();
	writeln!(out).unwrap();
	writeln!(out, "static main() {{").unwrap();

	if let Some(entry) = header.entry_point() {
		writeln!(out, "  MakeFunction({:#010x}, BADADDR);", entry).unwrap();
		writeln!(out, "  MakeName({:#010x}, \"_start\");", entry).unwrap();
	}

	writeln!(out, "  SetSegmentAttr({:#010x}, SEGATTR_START, {:#010x});", load_address, load_address).unwrap();
	writeln!(out, "  SetSegmentAttr({:#010x}, SEGATTR_END, {:#010x});", load_address, load_address + image_size)
		.unwrap();

	if let Some(table) = header.import_table() {
		writeln!(out).unwrap();
		for lib in &table.libraries {
			generate_import_comments(&mut out, lib);
		}
	}

	writeln!(out, "}}").unwrap();
	out
}

fn generate_import_comments(out: &mut String, lib: &ImportLibrary) {
	let lib_label = lib.name.replace('.', "_");

	for (i, chunk) in lib.records.chunks(2).enumerate() {
		if chunk.len() < 2 {
			break;
		}
		let iat_addr = chunk[0];
		let thunk_addr = chunk[1];

		let label = format!("__imp_{}_{}", lib_label, i);
		let thunk_label = format!("{}_{}", lib_label, i);

		writeln!(out, "  MakeName({:#010x}, \"{}\");", iat_addr, label).unwrap();
		writeln!(out, "  MakeFunction({:#010x}, BADADDR);", thunk_addr).unwrap();
		writeln!(out, "  MakeName({:#010x}, \"{}\");", thunk_addr, thunk_label).unwrap();
	}
}
