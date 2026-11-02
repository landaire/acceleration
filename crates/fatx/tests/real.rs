//! Listings and forensic scans of real volumes. Each test reads its image path from an
//! environment variable:
//!
//! - `FATX_IMAGE`: an image holding FATX volumes, with `FATX_RANGES` naming their byte ranges
//!   as comma-separated `start..end` hex pairs (the whole image when unset).
//! - `FATX_FDF_IMAGE`: a flash `.fdf` volume file.
//!
//! Run with `cargo test -p fatx --test real -- --ignored --nocapture`.

use std::fs::File;
use std::ops::Range;

use fatx::FileSource;
use fatx::PartitionKind;
use fatx::Volume;
use fatx::VolumeOptions;
use fatx::forensic::scan::ScanOptions;

fn parse_hex(text: &str) -> u64 {
	u64::from_str_radix(text.trim().trim_start_matches("0x"), 16).expect("a hex number")
}

fn parse_ranges(text: &str) -> Vec<Range<u64>> {
	text.split(',')
		.map(|pair| {
			let (start, end) = pair.split_once("..").expect("a start..end pair");
			parse_hex(start)..parse_hex(end)
		})
		.collect()
}

/// Prints a volume's listing, deleted entries, warnings and forensic report.
fn report(source: &FileSource, range: Range<u64>, options: VolumeOptions) {
	let volume = Volume::open_with(source, range.clone(), options).expect("the volume opens");
	let geometry = volume.geometry();
	println!(
		"volume {range:#x?}: serial {} {:?} {:#x}-byte clusters, {:#x} clusters ({:#x} in the image), root {}",
		geometry.serial_number(),
		geometry.fat_width(),
		geometry.bytes_per_cluster(),
		geometry.cluster_count(),
		volume.image_cluster_count(),
		geometry.root_cluster()
	);

	let (listing, warnings) = volume.walk(source).expect("the walk reads the image").into_parts();
	let mut entries: Vec<_> = listing.entries().iter().collect();
	entries.sort_by(|left, right| left.path().cmp(right.path()));
	println!("live entries: {}", entries.len());
	for entry in entries {
		let record = entry.record();
		let kind = if record.is_directory() { "dir " } else { "file" };
		println!(
			"  {kind} {:>10} {} first {:?} attr {:#04x} {}",
			record.size(),
			record.modified(),
			record.first_cluster().map(|cluster| cluster.get()),
			record.attributes().bits(),
			entry.path()
		);
	}

	println!("deleted entries: {}", listing.deleted().len());
	for deleted in listing.deleted() {
		let record = deleted.record();
		let kind = if record.is_directory() { "dir " } else { "file" };
		println!(
			"  {kind} {:>10} created {} modified {} first {:?} {:?} {}",
			record.size(),
			record.created(),
			record.modified(),
			record.first_cluster().map(|cluster| cluster.get()),
			deleted.origin(),
			deleted.path()
		);
	}

	println!("directory slack records: {}", listing.slack().len());
	for slack in listing.slack() {
		let orphan = slack.orphan();
		println!(
			"  {} in {} at {} size {} {:?}",
			orphan.record.name(),
			slack.directory(),
			orphan.record.location(),
			orphan.record.size(),
			orphan.confidence
		);
	}

	println!("warnings: {}", warnings.len());
	for warning in &warnings {
		println!("  {warning}");
	}

	let scan = volume.scan(source, &listing, ScanOptions::new()).expect("the scan reads the image");
	println!("forensic scan: {} free clusters scanned", scan.scanned_clusters);
	for assessment in &scan.deleted {
		let deleted = &listing.deleted()[assessment.entry];
		let plan = match &assessment.plan {
			Ok(plan) => format!("{} clusters allocated, {} missing", plan.allocated.len(), plan.missing),
			Err(error) => error.to_string(),
		};
		let content = assessment
			.content
			.as_ref()
			.map(|content| (content.detected.as_ref().map(|detected| detected.kind), content.kind, content.size));
		println!("  deleted {}: {plan}; content {content:?}; {:?}", deleted.path(), assessment.confidence);
	}
	println!("  orphaned records: {}", scan.orphans.len());
	for orphan in &scan.orphans {
		println!(
			"    {} at {} size {} first {:?} deleted {} neighbors {} {:?}",
			orphan.record.name(),
			orphan.record.location(),
			orphan.record.size(),
			orphan.record.first_cluster().map(|cluster| cluster.get()),
			orphan.record.is_marked_deleted(),
			orphan.neighbors,
			orphan.confidence
		);
	}
	println!("  carved candidates: {}", scan.carved.len());
	for candidate in &scan.carved {
		println!(
			"    cluster {} offset {:#x} {:?} size {:?} {:?} explained {}",
			candidate.cluster,
			candidate.offset,
			candidate.content.kind,
			candidate.content.size,
			candidate.content.detail,
			candidate.explained
		);
	}
	println!("  matches: {}", scan.matches.len());
	for found in &scan.matches {
		println!("    candidate {} {:?} {:?} {:?}", found.candidate, found.entry, found.reasons, found.confidence);
	}
}

#[test]
#[ignore = "needs FATX_IMAGE"]
fn real_image_volumes() {
	let path = std::env::var("FATX_IMAGE").expect("FATX_IMAGE names an image");
	let file = File::open(path).expect("the image opens");
	let length = file.metadata().expect("the image has metadata").len();
	let ranges = match std::env::var("FATX_RANGES") {
		Ok(ranges) => parse_ranges(&ranges),
		Err(_) => std::iter::once(0..length).collect(),
	};
	let source = FileSource::new(file);

	for range in ranges {
		report(&source, range, VolumeOptions::new());
	}
}

#[test]
#[ignore = "needs FATX_FDF_IMAGE"]
fn real_fdf_volume() {
	let path = std::env::var("FATX_FDF_IMAGE").expect("FATX_FDF_IMAGE names a volume file");
	let file = File::open(path).expect("the volume file opens");
	let length = file.metadata().expect("the volume file has metadata").len();
	let source = FileSource::new(file);

	let inference = Volume::infer_partition_length(&source, 0..length, PartitionKind::Standard)
		.expect("the partition length is inferred");
	println!(
		"inferred {:?} FAT of {:#x} bytes; partition lengths {:#x?}; {:?}",
		inference.fat_width(),
		inference.fat_length(),
		inference.partition_lengths(),
		inference.evidence()
	);
	for rejected in inference.rejected() {
		println!("  rejected {:?} FAT of {:#x} bytes: {}", rejected.fat_width, rejected.fat_length, rejected.reason);
	}

	report(&source, 0..length, inference.volume_options());
}
