//! The device an image holds, the evidence for it, and every partition of its layout.

use std::io;
use std::io::Write;

use xenon_disk::ByteCount;
use xenon_disk::DetectError;
use xenon_disk::DetectOptions;
use xenon_disk::Detection;
use xenon_disk::DevkitSlot;
use xenon_disk::DevkitTable;
use xenon_disk::JoshBlock;
use xenon_disk::LogoBitmap;
use xenon_disk::LogoBitmapError;
use xenon_disk::Partition;
use xenon_disk::PartitionMap;
use xenon_disk::SecuritySector;
use xenon_disk::VolumeKind;
use xenon_disk::detect;

use crate::error::DiskToolError;
use crate::session::Session;
use crate::source::VolumeSource;

pub fn run<S, W>(
	source: &S,
	image_length: u64,
	options: DetectOptions,
	session: &Session<'_>,
	out: &mut W,
) -> Result<(), DiskToolError>
where
	S: VolumeSource,
	W: Write,
{
	let detection = match detect(source, ByteCount::new(image_length), options) {
		Ok(detection) => detection,
		Err(DetectError::LoneVolume { length }) => {
			session.check_source(source)?;
			return writeln!(
				out,
				"the image is one FATX volume of {length:#x} bytes, not a device with a partition layout"
			)
			.map_err(DiskToolError::WriteListing);
		}
		Err(error) => return Err(error.into()),
	};
	session.check_source(source)?;
	let detection = session.resolve(detection)?;

	print(&detection, out).map_err(DiskToolError::WriteListing)
}

fn print<W>(detection: &Detection, out: &mut W) -> io::Result<()>
where
	W: Write,
{
	writeln!(out, "device: {}", detection.kind())?;
	for alternative in detection.alternatives() {
		writeln!(out, "also matches: {alternative}")?;
	}
	for evidence in detection.evidence() {
		writeln!(out, "evidence: {evidence}")?;
	}
	if let Ok(table) = detection.devkit_table() {
		print_devkit_table(table, out)?;
	}
	if let Some(block) = detection.josh_block() {
		print_josh_block(block, out)?;
	}
	if let Some(sector) = detection.security_sector() {
		print_security_sector(sector, detection.logo_bitmap(), out)?;
	}

	print_map(detection.map(), |partition| detection.has_volume_header(partition), out)
}

/// The device size and a table of the partitions; `has_volume_header` marks those that start
/// with a FATX volume header.
pub(crate) fn print_map<W, F>(map: &PartitionMap, has_volume_header: F, out: &mut W) -> io::Result<()>
where
	W: Write,
	F: Fn(&Partition) -> bool,
{
	writeln!(
		out,
		"length: {:#x} bytes, {:#x} sectors of {:#x} bytes",
		map.length().get(),
		map.sector_count(),
		map.sector_size()
	)?;
	writeln!(out, "{:<24} {:>14} {:>14} {:>14}  {:<6} FLAGS", "PARTITION", "START", "END", "LENGTH", "VOLUME")?;
	for partition in map.partitions() {
		let range = partition.range();
		let volume = if has_volume_header(partition) { "XTAF" } else { "-" };
		let mut flags = Vec::new();
		if partition.is_raw_only() {
			flags.push("raw-only");
		}
		if partition.volume() == VolumeKind::Alternate {
			flags.push("alternate");
		}
		let line = format!(
			"{:<24} {:>#14x} {:>#14x} {:>#14x}  {volume:<6} {}",
			partition.name(),
			range.start,
			range.end,
			range.end - range.start,
			flags.join(",")
		);
		writeln!(out, "{}", line.trim_end())?;
	}

	Ok(())
}

fn print_devkit_table<W>(table: &DevkitTable, out: &mut W) -> io::Result<()>
where
	W: Write,
{
	writeln!(out, "devkit table: minimum {:#010x}", table.minimum())?;
	for (index, entry) in table.entries().iter().enumerate() {
		if !entry.is_assigned() {
			continue;
		}

		let slot = DevkitSlot::from_index(index).map_or("not used by the kernel", DevkitSlot::name);
		writeln!(out, "  entry {index}: sector {} count {} ({slot})", entry.start, entry.count)?;
	}

	Ok(())
}

fn print_josh_block<W>(block: &JoshBlock, out: &mut W) -> io::Result<()>
where
	W: Write,
{
	let certificate = &block.console_signature.certificate;
	writeln!(out, "Josh block at 0x800 (utility drive cache table of titles; not read by the kernel):")?;
	writeln!(out, "  console id: {}", certificate.console_id)?;
	writeln!(out, "  console part number: {}", certificate.console_part_number)?;
	let console_type = certificate.console_type;
	writeln!(out, "  console type: {:#010x} ({:?})", console_type.0, console_type.kind())?;
	writeln!(out, "  manufacturing date: {}", certificate.manufacturing_date)?;
	let verification = block.verify_console_signature();
	writeln!(out, "  certificate: {}", verification.certificate)?;
	match verification.signature {
		Ok(()) => writeln!(out, "  console signature: verified with the certificate's key")?,
		Err(error) => writeln!(out, "  console signature: {error}")?,
	}
	writeln!(out, "  version: {}", block.version)?;
	writeln!(out, "  last assigned cache: {}", block.last_assigned)?;
	for (index, (entry, owner)) in block.caches.iter().zip(block.owners).enumerate() {
		let owner = owner.map_or_else(|| "none".to_owned(), |title| title.to_string());
		let state = if entry.is_zero() { "zero" } else { "in use" };
		writeln!(out, "  Cache{index}: owner {owner}, entry {state}")?;
	}

	Ok(())
}

fn print_security_sector<W>(
	sector: &SecuritySector,
	logo: Option<Result<&LogoBitmap, &LogoBitmapError>>,
	out: &mut W,
) -> io::Result<()>
where
	W: Write,
{
	writeln!(out, "security sector:")?;
	writeln!(out, "  serial number: {}", sector.serial_number)?;
	writeln!(out, "  firmware revision: {}", sector.firmware_revision)?;
	writeln!(out, "  model number: {}", sector.model_number)?;
	writeln!(out, "  user addressable sectors: {}", sector.user_addressable_sectors)?;
	writeln!(out, "  logo bitmap digest: {}", hex(&sector.logo_digest))?;
	match logo {
		Some(Ok(logo)) => {
			let kind = if logo.is_png() { "PNG image" } else { "image of unknown format" };
			writeln!(out, "  logo: {kind} of {:#x} bytes", logo.image().len())?;
		}
		Some(Err(error)) => writeln!(out, "  logo: {error}")?,
		None => {
			// The logo region is read with a security sector that reads as one.
		}
	}
	if sector.is_signature_absent() {
		writeln!(out, "  signature: absent (all zero)")?;
	} else {
		writeln!(out, "  signature: present, not verified; signed digest {}", hex(sector.signed_digest()))?;
	}

	Ok(())
}

fn hex(bytes: &[u8]) -> String {
	bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
