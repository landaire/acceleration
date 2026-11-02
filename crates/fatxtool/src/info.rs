//! The volume header, geometry and allocation.

use std::io;
use std::io::Write;

use fatx::ClusterNumber;
use fatx::PartitionKind;
use fatx::Volume;
use fatx::volume::SECTOR_SIZE;

use crate::describe;
use crate::error::FatxToolError;

pub fn run<W>(volume: &Volume, out: &mut W) -> Result<(), FatxToolError>
where
	W: Write,
{
	print(volume, out).map_err(FatxToolError::WriteListing)
}

fn print<W>(volume: &Volume, out: &mut W) -> io::Result<()>
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

	Ok(())
}
