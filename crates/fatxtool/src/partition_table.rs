//! Writes a devkit partition table to sector 0 of an image file. Only regular files are
//! written, so a block device or a link to one is refused.

use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::path::Path;

use clap::Subcommand;
use xenon_disk::ByteCount;
use xenon_disk::DevkitRequest;
use xenon_disk::DevkitSlot;
use xenon_disk::DevkitTable;
use xenon_disk::DiskSectors;
use xenon_disk::MemoryDumpSize;
use xenon_disk::RequestSize;
use xenon_disk::SectorCount;
use xenon_disk::devkit::DEVKIT_TABLE_MINIMUM;

use crate::error::DiskToolError;
use crate::error::NumberError;
use crate::number::parse_number;
use crate::number::parse_u32;
use crate::output::open_writable_image;
use crate::partitions::print_map;

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum PartitionTableCommand {
	/// Write a devkit partition table to sector 0 of an image file. The partitions are placed in
	/// the order given, from sector 0x400 as on a retail disk.
	Write {
		/// NAME=SIZE: a devkit partition (Partition1, SystemPartition, DumpPartition, PixDump,
		/// AltFlash, Cache0 or Cache1) and its size in bytes, or `rest` for the remaining sectors
		#[arg(required = true, value_name = "NAME=SIZE", value_parser = parse_request)]
		partitions: Vec<DevkitRequest>,
		/// The table's minimum field
		#[arg(long, value_parser = parse_u32, default_value_t = DEVKIT_TABLE_MINIMUM)]
		minimum: u32,
		/// Alignment of each partition start in bytes, a power of two of at least one sector
		#[arg(long, value_parser = parse_number, default_value = "0x1000")]
		align: u64,
	},
}

pub fn run<W>(
	command: &PartitionTableCommand,
	image: &Path,
	memory: MemoryDumpSize,
	out: &mut W,
) -> Result<(), DiskToolError>
where
	W: Write,
{
	match command {
		PartitionTableCommand::Write { partitions, minimum, align } => {
			write_table(image, partitions, *minimum, *align, memory, out)
		}
	}
}

fn write_table<W>(
	image: &Path,
	requests: &[DevkitRequest],
	minimum: u32,
	align: u64,
	memory: MemoryDumpSize,
	out: &mut W,
) -> Result<(), DiskToolError>
where
	W: Write,
{
	let alignment = SectorCount::from_bytes(ByteCount::new(align))
		.filter(|sectors| sectors.get().is_power_of_two())
		.ok_or(DiskToolError::InvalidAlignment { alignment: align })?;

	let mut file = open_writable_image(image)?;
	let length = file.metadata().map_err(|source| DiskToolError::OpenImage { path: image.to_owned(), source })?.len();
	let disk = SectorCount::from_bytes(ByteCount::new(length)).ok_or(DiskToolError::ImageNotWholeSectors { length })?;
	let table = DevkitTable::pack(minimum, disk, alignment, requests)?;

	let written =
		file.seek(SeekFrom::Start(0)).and_then(|_| file.write_all(&table.to_bytes())).and_then(|()| file.sync_all());
	written.map_err(|source| DiskToolError::WriteTable { path: image.to_owned(), source })?;

	let map = table.partition_map(DiskSectors::whole(disk), memory).ignore_warnings();
	print_map(&map, |_| false, out).map_err(DiskToolError::WriteListing)
}

/// `NAME=SIZE`, with a size in bytes or `rest`.
fn parse_request(text: &str) -> Result<DevkitRequest, String> {
	let (name, size) = text.split_once('=').ok_or_else(|| format!("{text:?} is not NAME=SIZE"))?;
	let slot = DevkitSlot::from_name(name).ok_or_else(|| {
		let names: Vec<&str> = DevkitSlot::ALL.iter().map(|slot| slot.name()).collect();
		format!("{name:?} is not a devkit partition; the partitions are {}", names.join(", "))
	})?;
	let size = if size.eq_ignore_ascii_case("rest") {
		RequestSize::Rest
	} else {
		RequestSize::Bytes(ByteCount::new(parse_number(size).map_err(|error: NumberError| error.to_string())?))
	};

	Ok(DevkitRequest { slot, size })
}
