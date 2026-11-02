//! The volume header, geometry and allocation, and the partition length inference of a volume
//! file.

use std::io;
use std::io::Write;

use fatx::ClusterNumber;
use fatx::PartitionKind;
use fatx::Volume;
use fatx::volume::SECTOR_SIZE;

use crate::describe;
use crate::error::FatxToolError;
use crate::location::VolumeLocation;

pub fn run<W>(volume: &Volume, location: &VolumeLocation, out: &mut W) -> Result<(), FatxToolError>
where
	W: Write,
{
	print(volume, location, out).map_err(FatxToolError::WriteListing)
}

fn print<W>(volume: &Volume, location: &VolumeLocation, out: &mut W) -> io::Result<()>
where
	W: Write,
{
	let geometry = volume.geometry();
	let range = volume.range();
	let cluster_size = geometry.bytes_per_cluster();
	let free = (1..=geometry.cluster_count())
		.filter_map(ClusterNumber::new)
		.filter(|cluster| volume.fat().is_free(*cluster))
		.count();

	writeln!(out, "range: {:#x}..{:#x} ({:#x} bytes)", range.start, range.end, range.end - range.start)?;
	writeln!(out, "serial number: {}", geometry.serial_number())?;
	writeln!(out, "cluster size: {cluster_size:#x} bytes ({} sectors)", u64::from(cluster_size) / SECTOR_SIZE)?;
	writeln!(out, "root directory cluster: {}", geometry.root_cluster())?;
	writeln!(out, "partition length: {:#x}", geometry.partition_length())?;
	if geometry.kind() == PartitionKind::Alternate {
		writeln!(out, "partition kind: alternate, in a growable file")?;
	}
	writeln!(
		out,
		"FAT: {} at {:#x}, {:#x} bytes",
		describe::fat_width(geometry.fat_width()),
		geometry.fat_offset(),
		geometry.fat_length()
	)?;
	writeln!(out, "file area: {:#x}", geometry.file_area_offset())?;
	writeln!(
		out,
		"clusters: {} in the partition, {} in the image",
		geometry.cluster_count(),
		volume.image_cluster_count()
	)?;
	writeln!(out, "free clusters: {free}")?;

	let Some(inference) = location.inference() else {
		return Ok(());
	};

	let lengths = inference.partition_lengths();
	writeln!(
		out,
		"inferred partition lengths: {:#x}..{:#x} ({} with a {:#x}-byte FAT)",
		lengths.start,
		lengths.end,
		describe::fat_width(inference.fat_width()),
		inference.fat_length()
	)?;
	let evidence = inference.evidence();
	writeln!(
		out,
		"inference evidence: root cluster FAT entry {}; {} live and {} deleted records in the root directory",
		describe::fat_entry(evidence.fat_entry),
		evidence.live_records,
		evidence.deleted_records
	)?;
	for rejected in inference.rejected() {
		writeln!(
			out,
			"rejected {} with a {:#x}-byte FAT: {}",
			describe::fat_width(rejected.fat_width),
			rejected.fat_length,
			rejected.reason
		)?;
	}

	Ok(())
}
