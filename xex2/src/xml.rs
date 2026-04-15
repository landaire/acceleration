use std::fmt::Write;

use crate::header::EncryptionType;
use crate::Xex2;

pub fn generate_xml(xex: &Xex2) -> String {
	let mut out = String::new();
	let header = &xex.header;
	let security = &xex.security_info;

	writeln!(out, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>").unwrap();
	writeln!(out, "<xex>").unwrap();

	writeln!(out, "  <module_flags>{:#010x}</module_flags>", header.module_flags.0).unwrap();

	let basefile_type = if header.module_flags.is_dll() {
		"dll"
	} else if header.module_flags.is_patch() {
		"patch"
	} else {
		"exe"
	};
	writeln!(out, "  <basefile_type>{}</basefile_type>", basefile_type).unwrap();

	let machine = if let Ok(fmt) = header.file_format_info(xex.raw()) {
		match fmt.encryption_type {
			EncryptionType::Normal => "retail",
			EncryptionType::None => "devkit",
		}
	} else {
		"unknown"
	};
	writeln!(out, "  <machine>{}</machine>", machine).unwrap();

	if let Some(exec) = header.execution_info() {
		writeln!(out, "  <title_id>{:08X}</title_id>", exec.title_id).unwrap();
		writeln!(out, "  <media_id>{:08X}</media_id>", exec.media_id).unwrap();
		writeln!(out, "  <version>{:#010x}</version>", exec.version).unwrap();
		writeln!(out, "  <disc_number>{}</disc_number>", exec.disc_number).unwrap();
		writeln!(out, "  <disc_count>{}</disc_count>", exec.disc_count).unwrap();
	}

	writeln!(out, "  <game_regions>{:#010x}</game_regions>", security.image_info.game_regions).unwrap();
	writeln!(out, "  <allowed_media>{:#010x}</allowed_media>", security.image_info.allowed_media_types).unwrap();

	if let Some(path) = header.bounding_path() {
		writeln!(out, "  <bounding_path>{}</bounding_path>", escape_xml(&path)).unwrap();
	}

	if let Some(res) = header.resource_info() {
		writeln!(out, "  <resources>").unwrap();
		for r in &res.resources {
			writeln!(
				out,
				"    <resource name=\"{}\" address=\"{:#010x}\" size=\"{:#x}\"/>",
				escape_xml(&r.name),
				r.address,
				r.size
			)
			.unwrap();
		}
		writeln!(out, "  </resources>").unwrap();
	}

	if let Some(table) = header.import_table() {
		writeln!(out, "  <imports>").unwrap();
		for lib in &table.libraries {
			writeln!(
				out,
				"    <library name=\"{}\" version=\"{:#010x}\" min_version=\"{:#010x}\" imports=\"{}\"/>",
				escape_xml(&lib.name),
				lib.version,
				lib.version_min,
				lib.records.len() / 2
			)
			.unwrap();
		}
		writeln!(out, "  </imports>").unwrap();
	}

	writeln!(out, "</xex>").unwrap();
	out
}

fn escape_xml(s: &str) -> String {
	s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
