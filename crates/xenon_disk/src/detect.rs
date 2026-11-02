//! Identifies the device a source holds from its length and a few reads: the devkit table in
//! sector 0, the security sector, and the `XTAF` header at the start of each partition the
//! candidate layouts place.

use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::fmt;
use std::ops::Range;

use xenon_io::AsyncReadAt;
use xenon_io::Checked;
use xenon_io::Machine;
use xenon_io::ReadAt;
use xenon_io::Step;
use xenon_io::drive;
use xenon_io::drive_async;
use xenon_io::expect_len;

use crate::device::DeviceKind;
use crate::devkit::DEVKIT_TABLE_SIZE;
use crate::devkit::DevkitTable;
use crate::devkit::MemoryDumpSize;
use crate::error::DetectError;
use crate::error::DevkitTableError;
use crate::error::DiskWarning;
use crate::error::LogoBitmapError;
use crate::josh::JOSH_BLOCK_OFFSET;
use crate::josh::JOSH_BLOCK_SIZE;
use crate::josh::JOSH_BLOCK_VERSION;
use crate::josh::JoshBlock;
use crate::layout;
use crate::layout::DiskSectors;
use crate::partition::Partition;
use crate::partition::PartitionMap;
use crate::security::LOGO_BITMAP_OFFSET;
use crate::security::LOGO_BITMAP_SIZE;
use crate::security::LogoBitmap;
use crate::security::SECURITY_SECTOR_OFFSET;
use crate::security::SECURITY_SECTOR_SIZE;
use crate::security::SecuritySector;
use crate::units::ByteCount;
use crate::units::ByteOffset;
use crate::units::SectorCount;

const VOLUME_SIGNATURE: [u8; 4] = *b"XTAF";
/// Kinds whose layout follows from the length alone, in the order ties are broken. A transfer
/// cable image is a retail disk image, so it is never detected as such.
const LENGTH_LAYOUTS: [DeviceKind; 5] = [
	DeviceKind::UsbMemoryUnit,
	DeviceKind::SfcMemoryUnit,
	DeviceKind::UsbBuiltInMemoryUnit,
	DeviceKind::MmcMemoryUnit,
	DeviceKind::UsbStorage,
];

/// Inputs of the layouts that a source does not hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetectOptions {
	pub memory_dump: MemoryDumpSize,
}

/// A fact that supports a detected layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
	/// Sector 0 holds a devkit table the kernel accepts.
	DevkitTable { minimum: u32 },
	/// The sector at 0x2000 reads as a security sector.
	SecuritySector { user_addressable_sectors: SectorCount },
	/// The bytes at 0x800 start with the `Josh` magic, as on a console hard disk.
	JoshBlock,
	/// A partition of the layout starts with a FATX volume header.
	VolumeHeader { partition: &'static str, offset: ByteOffset },
}

impl fmt::Display for Evidence {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::DevkitTable { minimum } => {
				write!(f, "sector 0 holds a devkit partition table with minimum {minimum:#010x}")
			}
			Self::SecuritySector { user_addressable_sectors } => {
				write!(f, "the security sector at 0x2000 limits the disk to {user_addressable_sectors} sectors")
			}
			Self::JoshBlock => write!(f, "a block at 0x800 starts with Josh"),
			Self::VolumeHeader { partition, offset } => write!(f, "{partition} at {offset} starts with XTAF"),
		}
	}
}

/// The device a source holds and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
	map: PartitionMap,
	evidence: Vec<Evidence>,
	/// Kinds whose layouts have as much evidence.
	alternatives: Vec<DeviceKind>,
	devkit_table: Result<DevkitTable, Option<DevkitTableError>>,
	security_sector: Option<SecuritySector>,
	logo_bitmap: Option<Result<LogoBitmap, LogoBitmapError>>,
	josh_block: Option<JoshBlock>,
	volume_headers: BTreeSet<u64>,
}

impl Detection {
	pub const fn kind(&self) -> DeviceKind {
		self.map.device()
	}

	pub const fn map(&self) -> &PartitionMap {
		&self.map
	}

	pub fn evidence(&self) -> &[Evidence] {
		&self.evidence
	}

	pub fn alternatives(&self) -> &[DeviceKind] {
		&self.alternatives
	}

	/// The devkit table of sector 0, or why sector 0 is not one; `None` when the source is
	/// shorter than a sector.
	pub fn devkit_table(&self) -> Result<&DevkitTable, Option<&DevkitTableError>> {
		self.devkit_table.as_ref().map_err(Option::as_ref)
	}

	/// The sector at 0x2000, when it reads as a security sector.
	pub fn security_sector(&self) -> Option<&SecuritySector> {
		self.security_sector.as_ref()
	}

	/// The contents of the logo bitmap region, when the security sector reads as one.
	pub fn logo_bitmap(&self) -> Option<Result<&LogoBitmap, &LogoBitmapError>> {
		self.logo_bitmap.as_ref().map(Result::as_ref)
	}

	/// The block at 0x800, when it starts with the `Josh` magic.
	pub fn josh_block(&self) -> Option<&JoshBlock> {
		self.josh_block.as_ref()
	}

	/// Whether `partition` of the detected map starts with a FATX volume header.
	pub fn has_volume_header(&self, partition: &Partition) -> bool {
		self.volume_headers.contains(&partition.start().get())
	}
}

/// Detects the device of a `length`-byte source.
pub fn detect<R>(
	source: &R,
	length: ByteCount,
	options: DetectOptions,
) -> Result<Checked<Detection, DiskWarning>, DetectError>
where
	R: ReadAt,
{
	drive(Detect::new(length, options), source)
}

/// [`detect`] with asynchronous reads.
pub async fn detect_async<R>(
	source: &R,
	length: ByteCount,
	options: DetectOptions,
) -> Result<Checked<Detection, DiskWarning>, DetectError>
where
	R: AsyncReadAt,
{
	drive_async(Detect::new(length, options), source).await
}

/// The detection machine. It reads sector 0, the Josh block and the security sector, then the
/// first bytes of every partition of every layout that fits the length.
#[derive(Debug)]
pub struct Detect {
	length: ByteCount,
	options: DetectOptions,
	phase: Phase,
	pending: VecDeque<Probe>,
	current: Option<Probe>,

	devkit_table: Result<DevkitTable, Option<DevkitTableError>>,
	security: Option<(SecuritySector, Box<[u8; LOGO_BITMAP_SIZE]>)>,
	josh: Option<JoshBlock>,
	candidates: Vec<Candidate>,
	volume_headers: BTreeSet<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
	Metadata,
	VolumeHeaders,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Probe {
	DevkitTable,
	/// The security sector and the logo bitmap after it.
	SecuritySector,
	JoshBlock,
	VolumeHeader(u64),
}

impl Probe {
	fn range(self) -> Range<u64> {
		match self {
			Self::DevkitTable => 0..DEVKIT_TABLE_SIZE as u64,
			Self::SecuritySector => SECURITY_SECTOR_OFFSET..LOGO_BITMAP_OFFSET + LOGO_BITMAP_SIZE as u64,
			Self::JoshBlock => JOSH_BLOCK_OFFSET..JOSH_BLOCK_OFFSET + JOSH_BLOCK_SIZE as u64,
			Self::VolumeHeader(offset) => offset..offset + VOLUME_SIGNATURE.len() as u64,
		}
	}
}

#[derive(Debug)]
struct Candidate {
	map: PartitionMap,
	warnings: Vec<DiskWarning>,
	evidence: Vec<Evidence>,
	/// The kernel takes this layout whatever the other evidence: a devkit table that verifies.
	decisive: bool,
}

impl Detect {
	pub fn new(length: ByteCount, options: DetectOptions) -> Self {
		let pending = [Probe::DevkitTable, Probe::JoshBlock, Probe::SecuritySector]
			.into_iter()
			.filter(|probe| probe.range().end <= length.get())
			.collect();

		Self {
			length,
			options,
			phase: Phase::Metadata,
			pending,
			current: None,
			devkit_table: Err(None),
			security: None,
			josh: None,
			candidates: Vec::new(),
			volume_headers: BTreeSet::new(),
		}
	}

	/// Builds the layouts that fit the length and queues a read of each partition start.
	fn plan_volume_probes(&mut self) {
		self.candidates = self.candidates();

		let offsets: BTreeSet<u64> = self
			.candidates
			.iter()
			.flat_map(|candidate| candidate.map.partitions())
			.map(|partition| partition.start().get())
			// Offset 0 tells a lone volume from a source of unknown layout.
			.chain([0])
			.filter(|offset| offset.saturating_add(VOLUME_SIGNATURE.len() as u64) <= self.length.get())
			.collect();
		self.pending.extend(offsets.into_iter().map(Probe::VolumeHeader));
		self.phase = Phase::VolumeHeaders;
	}

	fn candidates(&self) -> Vec<Candidate> {
		let mut candidates = Vec::new();

		let security = self.security.as_ref().filter(|(sector, _)| sector.is_plausible());
		if let Some(device) = SectorCount::from_bytes(self.length) {
			let mut evidence = Vec::new();
			let mut warnings = Vec::new();
			if let Some((sector, logo)) = security {
				evidence.push(Evidence::SecuritySector { user_addressable_sectors: sector.user_addressable_sectors });
				warnings.extend(sector.check(logo, device));
			}
			if self.josh.is_some() {
				evidence.push(Evidence::JoshBlock);
			}

			// Each failed check fails authentication, and the kernel then keeps the device count.
			let authenticated = security.filter(|_| warnings.is_empty());
			let disk = DiskSectors::new(device, authenticated.map(|(sector, _)| sector.user_addressable_sectors));
			// The checks titles make on the Josh block; they do not affect the kernel's layout.
			if let Some(block) = &self.josh {
				let verification = block.verify_console_signature();
				if !verification.certificate.is_verified() {
					warnings.push(DiskWarning::JoshCertificate(verification.certificate));
				}
				if let Err(error) = verification.signature {
					warnings.push(DiskWarning::JoshConsoleSignature(error));
				}
				if block.version != JOSH_BLOCK_VERSION {
					warnings.push(DiskWarning::JoshVersion { version: block.version, expected: JOSH_BLOCK_VERSION });
				}
			}

			if let Ok(table) = &self.devkit_table {
				let (map, table_warnings) = table.partition_map(disk, self.options.memory_dump).into_parts();
				let mut table_evidence = evidence.clone();
				table_evidence.insert(0, Evidence::DevkitTable { minimum: table.minimum() });
				candidates.push(Candidate {
					map,
					warnings: warnings.iter().cloned().chain(table_warnings).collect(),
					evidence: table_evidence,
					decisive: true,
				});
			}
			if let Ok(map) = layout::retail_hdd(disk) {
				candidates.push(Candidate { map, warnings, evidence, decisive: false });
			}
		}

		for kind in LENGTH_LAYOUTS {
			if let Ok(map) = layout::for_device(kind, self.length) {
				candidates.push(Candidate { map, warnings: Vec::new(), evidence: Vec::new(), decisive: false });
			}
		}

		candidates
	}

	fn decide(&mut self) -> Result<Checked<Detection, DiskWarning>, DetectError> {
		let mut scored: Vec<(usize, Candidate)> = std::mem::take(&mut self.candidates)
			.into_iter()
			.map(|mut candidate| {
				for partition in candidate.map.partitions() {
					if !partition.is_raw_only() && self.volume_headers.contains(&partition.start().get()) {
						candidate
							.evidence
							.push(Evidence::VolumeHeader { partition: partition.name(), offset: partition.start() });
					}
				}
				let score = if candidate.decisive { usize::MAX } else { candidate.evidence.len() };
				(score, candidate)
			})
			.collect();

		let best = scored.iter().map(|(score, _)| *score).max().filter(|score| *score > 0);
		let position = best.and_then(|best| scored.iter().position(|(score, _)| *score == best));
		let Some((best, position)) = best.zip(position) else {
			if self.volume_headers.contains(&0) {
				return Err(DetectError::LoneVolume { length: self.length });
			}
			return Err(DetectError::Unrecognized { length: self.length });
		};

		// A volume image starts with a header too, so a header at offset 0 alone names no device.
		let (_, best_candidate) = &scored[position];
		let only_offset_zero = best_candidate
			.evidence
			.iter()
			.all(|evidence| matches!(evidence, Evidence::VolumeHeader { offset, .. } if offset.get() == 0));
		if only_offset_zero {
			return Err(DetectError::LoneVolume { length: self.length });
		}

		let alternatives = scored
			.iter()
			.filter(|(score, _)| *score == best)
			.skip(1)
			.map(|(_, candidate)| candidate.map.device())
			.collect();
		let (_, chosen) = scored.swap_remove(position);

		let security = self.security.take().filter(|(sector, _)| sector.is_plausible());
		let detection = Detection {
			map: chosen.map,
			evidence: chosen.evidence,
			alternatives,
			devkit_table: std::mem::replace(&mut self.devkit_table, Err(None)),
			logo_bitmap: security.as_ref().map(|(_, logo)| LogoBitmap::parse(logo)),
			security_sector: security.map(|(sector, _)| sector),
			josh_block: self.josh.take(),
			volume_headers: std::mem::take(&mut self.volume_headers),
		};

		Ok(Checked::new(detection, chosen.warnings))
	}
}

impl Machine for Detect {
	type Output = Checked<Detection, DiskWarning>;
	type Error = DetectError;

	fn step(&mut self) -> Result<Step<Self::Output>, DetectError> {
		loop {
			if let Some(probe) = self.current {
				return Ok(Step::Read(probe.range()));
			}

			if let Some(probe) = self.pending.pop_front() {
				self.current = Some(probe);
				continue;
			}

			match self.phase {
				Phase::Metadata => self.plan_volume_probes(),
				Phase::VolumeHeaders => return self.decide().map(Step::Done),
			}
		}
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), DetectError> {
		let probe = self.current.take().expect("Detect fed without a requested read");
		expect_len(&probe.range(), bytes)?;

		match probe {
			Probe::DevkitTable => {
				let sector: &[u8; DEVKIT_TABLE_SIZE] = bytes.try_into().expect("length checked");
				self.devkit_table = DevkitTable::parse(sector).map_err(Some);
			}
			Probe::SecuritySector => {
				let (sector, logo) = bytes.split_at(SECURITY_SECTOR_SIZE);
				let sector = SecuritySector::parse(sector.try_into().expect("length checked"));
				let logo: Box<[u8; LOGO_BITMAP_SIZE]> = Box::new(logo.try_into().expect("length checked"));
				self.security = Some((sector, logo));
			}
			Probe::JoshBlock => {
				// Bytes without the magic are not a Josh block; most devices have none.
				self.josh = JoshBlock::parse(bytes.try_into().expect("length checked")).ok();
			}
			Probe::VolumeHeader(offset) => {
				if bytes == VOLUME_SIGNATURE {
					self.volume_headers.insert(offset);
				}
			}
		}

		Ok(())
	}
}
