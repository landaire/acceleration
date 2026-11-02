mod common;

use common::Builder;
use common::Image;
use common::Record;
use common::bink;
use common::pattern;
use common::riff;
use fatx::ChainError;
use fatx::ClusterNumber;
use fatx::DeletedOrigin;
use fatx::DirectoryError;
use fatx::FatEntry;
use fatx::FatWidth;
use fatx::FatxError;
use fatx::GeometryError;
use fatx::Listing;
use fatx::PartitionKind;
use fatx::Volume;
use fatx::VolumeOptions;
use fatx::Warning;
use fatx::entry::InvalidEntryReason;
use fatx::forensic::Confidence;
use fatx::forensic::content::Agreement;
use fatx::forensic::content::ContentKind;
use fatx::forensic::content::DeclaredSize;
use fatx::forensic::matching::EntryReference;
use fatx::forensic::matching::MatchReason;
use fatx::forensic::orphan::OrphanOrigin;
use fatx::forensic::recover::RecoveryStrategy;
use fatx::forensic::scan::ScanOptions;
use fatx::volume::VolumeHeader;
use fatx::volume_file::CandidateRejection;
use fatx::volume_file::RootEvidence;
use fatx::volume_file::infer_partition_length;
use futures::executor::block_on;

const SMALL: u64 = 0x10_0000;
const CLUSTER: usize = 0x1000;

fn cluster(value: u32) -> ClusterNumber {
	ClusterNumber::new(value).unwrap()
}

fn open(image: &Image) -> Volume {
	Volume::open(image, 0..SMALL).unwrap()
}

fn paths(listing: &Listing) -> Vec<String> {
	let mut paths: Vec<String> = listing.entries().iter().map(|entry| entry.path().to_owned()).collect();
	paths.sort();
	paths
}

fn deleted_paths(listing: &Listing) -> Vec<(String, DeletedOrigin)> {
	let mut paths: Vec<_> = listing.deleted().iter().map(|entry| (entry.path(), entry.origin())).collect();
	paths.sort_by(|left, right| left.0.cmp(&right.0));
	paths
}

/// The first `length` bytes of a builder's image, as a volume file that holds only those.
fn file_prefix(builder: &Builder, length: u64) -> Image {
	let mut image = Image::new(length);
	image.write(0, &builder.image.read(0..length));
	image
}

/// Root spans clusters 1 and 10; /docs (2) ends with a 0x00 marker; /docs/deep (3) with 0xff.
fn nested_volume() -> (Builder, Vec<u8>) {
	let mut builder = Builder::new(SMALL, 8);
	builder.directory(&[1, 10]);
	for slot in 0..63 {
		let name = format!("empty{slot:02}");
		builder.record(1, slot, Record::file(&name, 0, 0));
	}
	builder.record(1, 63, Record::directory("docs", 2));
	let readme = pattern(1, 2 * CLUSTER + 0x123);
	builder.record(10, 0, Record::file("readme.txt", 5, readme.len() as u32));
	builder.file(&[5, 9, 6], &readme);

	builder.directory(&[2]);
	builder.record(2, 0, Record::directory("deep", 3));
	builder.write_cluster(2, 0x40, &[0; 0x40]);

	builder.directory(&[3]);
	builder.record(3, 0, Record::file("leaf.bin", 4, 100));
	builder.file(&[4], &pattern(9, 100));

	(builder, readme)
}

#[test]
fn fat16_lists_nested_directories_and_reads_chained_files() {
	let (builder, readme) = nested_volume();
	let volume = open(&builder.image);

	assert_eq!(volume.geometry().fat_width(), FatWidth::Fat16);
	assert_eq!(volume.geometry().file_area_offset(), 0x2000);
	assert_eq!(volume.geometry().serial_number().get(), common::SERIAL);

	let listing = volume.walk(&builder.image).unwrap().deny_warnings().unwrap();
	let mut expected: Vec<String> = (0..63).map(|slot| format!("/empty{slot:02}")).collect();
	expected.extend(["/docs", "/docs/deep", "/docs/deep/leaf.bin", "/readme.txt"].map(String::from));
	expected.sort();
	assert_eq!(paths(&listing), expected);
	assert!(listing.deleted().is_empty());

	let entry = listing.find("/readme.txt").unwrap();
	assert_eq!(volume.read_file(&builder.image, entry.record()).unwrap(), readme);
	assert_eq!(entry.record().modified().to_string(), "2010-09-09 19:00:42");
	let leaf = listing.find("/docs/deep/leaf.bin").unwrap();
	assert_eq!(volume.read_file(&builder.image, leaf.record()).unwrap(), pattern(9, 100));
}

#[test]
fn fat32_follows_32_bit_links() {
	const LARGE: u64 = 0x2000_0000;

	let mut builder = Builder::new(LARGE, 8);
	let data = pattern(3, CLUSTER + 10);
	builder.record(1, 0, Record::file("big.bin", 3, data.len() as u32));
	builder.file(&[3, 0x1_f000], &data);
	let volume = Volume::open(&builder.image, 0..LARGE).unwrap();

	assert_eq!(volume.geometry().fat_width(), FatWidth::Fat32);
	let listing = volume.walk(&builder.image).unwrap().deny_warnings().unwrap();
	let entry = listing.find("/big.bin").unwrap();
	assert_eq!(volume.read_file(&builder.image, entry.record()).unwrap(), data);
}

#[test]
fn volume_may_sit_inside_a_larger_source() {
	let (builder, readme) = nested_volume();
	let mut source = Image::new(SMALL + 0x8000);
	source.write(0x8000, &builder.image.read(0..SMALL));

	let volume = Volume::open(&source, 0x8000..0x8000 + SMALL).unwrap();
	let listing = volume.walk(&source).unwrap().deny_warnings().unwrap();

	assert_eq!(volume.read_file(&source, listing.find("/readme.txt").unwrap().record()).unwrap(), readme);
}

#[test]
fn rejects_volumes_the_driver_would_not_mount() {
	let mut builder = Builder::new(SMALL, 8);
	builder.image.write(0, b"FATX");
	assert!(matches!(
		Volume::open(&builder.image, 0..SMALL),
		Err(FatxError::Geometry(GeometryError::BadSignature { signature })) if &signature == b"FATX"
	));

	let builder = Builder::new(SMALL, 8);
	assert!(matches!(
		Volume::open(&builder.image, 0..8),
		Err(FatxError::ImageTooShort { required: 0x10, available: 8 })
	));
}

#[test]
fn integrity_failures_are_warnings() {
	let mut builder = Builder::new(SMALL, 8);
	builder.record(1, 0, Record::file("a:b", 0, 0));
	builder.record(1, 1, Record::directory("lost", 0x7fff));
	builder.record(1, 2, Record::file("short.bin", 20, 3 * CLUSTER as u32));
	builder.file(&[20, 21], &pattern(5, 2 * CLUSTER));
	builder.record(1, 3, Record::file("owner.bin", 25, 2 * CLUSTER as u32));
	builder.file(&[25, 26], &pattern(6, 2 * CLUSTER));
	builder.record(1, 4, Record::file("shared.bin", 26, CLUSTER as u32));
	builder.record(1, 5, Record::file("good.bin", 30, 5));
	builder.file(&[30], b"hello");
	let volume = open(&builder.image);

	let (listing, warnings) = volume.walk(&builder.image).unwrap().into_parts();

	assert_eq!(paths(&listing), ["/good.bin", "/lost", "/owner.bin", "/shared.bin", "/short.bin"]);
	assert_eq!(
		warnings,
		[
			Warning::InvalidEntry {
				location: fatx::EntryLocation { cluster: cluster(1), offset: 0 },
				reason: InvalidEntryReason::IllegalCharacter { byte: b':' },
			},
			Warning::UnreadableDirectory {
				path: "/lost".to_owned(),
				error: DirectoryError::Chain(ChainError::InvalidCluster { cluster: cluster(0x7fff) }),
			},
			Warning::BrokenChain {
				path: "/short.bin".to_owned(),
				error: ChainError::TooShort { first: cluster(20), needed: 3, available: 2 },
			},
			Warning::CrossLinkedCluster { path: "/shared.bin".to_owned(), cluster: cluster(26) },
		]
	);
	assert!(volume.walk(&builder.image).unwrap().deny_warnings().is_err());
	assert!(matches!(
		volume.read_file(&builder.image, listing.find("/short.bin").unwrap().record()),
		Err(FatxError::Chain(ChainError::TooShort { .. }))
	));
}

/// Root: live keep.bin (3); deleted movie.bik (2..=4, 3 now keep.bin's); deleted intact.wav
/// (10..=11, free); deleted frag.png stored in 20 and 22, with 21 now live.png's.
fn deleted_volume() -> (Builder, Vec<u8>, Vec<u8>, Vec<u8>) {
	let mut builder = Builder::new(SMALL, 8);
	let movie = bink(2 * CLUSTER + 0x80);
	let wav = riff(CLUSTER + 0x200);
	let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
	png.extend(pattern(0x70, CLUSTER + 0x50));

	builder.record(1, 0, Record::file("movie.bik", 2, movie.len() as u32).deleted());
	builder.data(&[2, 3, 4], &movie);
	builder.record(1, 1, Record::file("keep.bin", 3, 0x10));
	builder.file(&[3], &pattern(0xaa, 0x10));
	builder.record(1, 2, Record::file("intact.wav", 10, wav.len() as u32).deleted());
	builder.data(&[10, 11], &wav);
	builder.record(1, 3, Record::file("frag.png", 20, png.len() as u32).deleted());
	builder.data(&[20, 22], &png);
	builder.record(1, 4, Record::file("live.png", 21, 8));
	builder.file(&[21], b"\x89PNG\r\n\x1a\n");

	(builder, movie, wav, png)
}

#[test]
fn deleted_entries_keep_their_metadata() {
	let (builder, movie, _, _) = deleted_volume();
	let volume = open(&builder.image);
	let listing = volume.walk(&builder.image).unwrap().deny_warnings().unwrap();

	assert_eq!(paths(&listing), ["/keep.bin", "/live.png"]);
	let deleted = &listing.deleted()[0];
	assert_eq!(deleted.path(), "/movie.bik");
	assert_eq!(deleted.parent(), "/");
	assert_eq!(deleted.origin(), DeletedOrigin::LiveDirectory);
	assert_eq!(deleted.record().first_cluster(), Some(cluster(2)));
	assert_eq!(deleted.record().size() as usize, movie.len());
	assert!(deleted.record().is_marked_deleted());
	assert_eq!(deleted.record().created().to_string(), "2010-09-09 19:00:42");
}

#[test]
fn intact_deleted_file_recovers_with_high_confidence() {
	let (builder, _, wav, _) = deleted_volume();
	let volume = open(&builder.image);
	let listing = volume.walk(&builder.image).unwrap().ignore_warnings();
	let record = listing.deleted().iter().find(|entry| entry.record().name() == "intact.wav").unwrap().record();

	for strategy in [RecoveryStrategy::Contiguous, RecoveryStrategy::ContiguousFree] {
		let recovery = volume.recover(&builder.image, record, strategy).unwrap();

		assert!(recovery.plan.is_intact());
		assert_eq!(recovery.segments.len(), 1);
		assert_eq!(recovery.segments[0].bytes, wav);
		assert_eq!(recovery.content.detected.as_ref().unwrap().kind, ContentKind::Riff);
		assert_eq!(recovery.content.kind, Agreement::Agrees);
		assert_eq!(recovery.content.size, Agreement::Agrees);
		assert_eq!(recovery.confidence, Confidence::High);
	}
}

#[test]
fn chunked_recovery_matches_the_buffered_one() {
	const CHUNK: u64 = 0x300;

	let (builder, movie, _, _) = deleted_volume();
	let volume = open(&builder.image);
	let listing = volume.walk(&builder.image).unwrap().ignore_warnings();
	let record = listing.deleted()[0].record();

	for strategy in [RecoveryStrategy::Contiguous, RecoveryStrategy::ContiguousFree] {
		let mut reader = volume.recovery_reader(record, strategy, CHUNK).unwrap();
		let mut data = vec![0; movie.len()];
		while let Some(read) = reader.next_read() {
			assert!(read.range.end - read.range.start <= CHUNK);
			let bytes = builder.image.read(read.range.clone());
			reader.accept(&bytes).unwrap();
			let start = read.file_offset as usize;
			data[start..start + bytes.len()].copy_from_slice(&bytes);
		}
		let assessment = reader.finish();

		let buffered = volume.recover(&builder.image, record, strategy).unwrap();
		let mut expected = vec![0; movie.len()];
		for segment in &buffered.segments {
			let start = segment.file_offset as usize;
			expected[start..start + segment.bytes.len()].copy_from_slice(&segment.bytes);
		}
		assert_eq!(data, expected);
		assert_eq!(
			(assessment.plan, assessment.content, assessment.confidence),
			(buffered.plan, buffered.content, buffered.confidence)
		);
	}
}

#[test]
fn overwritten_clusters_are_reported_not_recovered() {
	let (builder, movie, _, _) = deleted_volume();
	let volume = open(&builder.image);
	let listing = volume.walk(&builder.image).unwrap().ignore_warnings();
	let record = listing.deleted()[0].record();

	let contiguous = volume.recover(&builder.image, record, RecoveryStrategy::Contiguous).unwrap();
	assert_eq!(contiguous.plan.allocated, [cluster(3)]);
	assert_eq!(contiguous.confidence, Confidence::Low);
	assert_eq!(contiguous.segments[0].bytes.len(), movie.len());
	assert_ne!(contiguous.segments[0].bytes, movie);

	let free = volume.recover(&builder.image, record, RecoveryStrategy::ContiguousFree).unwrap();
	assert_eq!(free.plan.allocated, [cluster(3)]);
	assert_eq!(
		free.segments.iter().map(|segment| (segment.file_offset, segment.bytes.as_slice())).collect::<Vec<_>>(),
		[(0, &movie[..CLUSTER]), (2 * CLUSTER as u64, &movie[2 * CLUSTER..])]
	);
	assert_eq!(free.confidence, Confidence::Low);
}

#[test]
fn fragmented_file_cannot_be_recovered_past_its_first_fragment() {
	let (builder, _, _, png) = deleted_volume();
	let volume = open(&builder.image);
	let listing = volume.walk(&builder.image).unwrap().ignore_warnings();
	let record = listing.deleted().iter().find(|entry| entry.record().name() == "frag.png").unwrap().record();

	let recovery = volume.recover(&builder.image, record, RecoveryStrategy::ContiguousFree).unwrap();

	assert_eq!(recovery.plan.allocated, [cluster(21)]);
	assert_eq!(recovery.segments.len(), 1);
	assert_eq!(recovery.segments[0].bytes, png[..CLUSTER]);
	assert_eq!(recovery.content.kind, Agreement::Agrees);
	assert_eq!(recovery.confidence, Confidence::Low);
}

#[test]
fn deleted_directories_are_descended() {
	let mut builder = Builder::new(SMALL, 8);
	builder.record(1, 0, Record::directory("old", 30).deleted());
	builder.write_cluster(30, 0, &vec![0xff; CLUSTER]);
	builder.record(30, 0, Record::file("a.txt", 31, 5));
	builder.record(30, 1, Record::file("b.txt", 32, 5).deleted());
	builder.record(30, 2, Record::directory("sub", 40).deleted());
	builder.write_cluster(40, 0, &vec![0xff; CLUSTER]);
	builder.record(40, 0, Record::file("c.txt", 41, 5).deleted());
	let volume = open(&builder.image);

	let listing = volume.walk(&builder.image).unwrap().deny_warnings().unwrap();

	assert!(listing.entries().is_empty());
	assert_eq!(
		deleted_paths(&listing),
		[
			("/old".to_owned(), DeletedOrigin::LiveDirectory),
			("/old/a.txt".to_owned(), DeletedOrigin::DeletedDirectory),
			("/old/b.txt".to_owned(), DeletedOrigin::DeletedDirectory),
			("/old/sub".to_owned(), DeletedOrigin::DeletedDirectory),
			("/old/sub/c.txt".to_owned(), DeletedOrigin::DeletedDirectory),
		]
	);
	assert_eq!(listing.recovered_clusters(), [cluster(30), cluster(40)]);
}

/// Root: slack record after the end marker; deleted clip2.bik at 41. Free cluster 40 holds two
/// orphaned records, one for clip.bik at 41; 41 starts a Bink file; 60 starts an XEX2 file that
/// no record explains; live game.xex at 50 also starts with XEX2.
fn carving_volume() -> (Builder, Vec<u8>) {
	let mut builder = Builder::new(SMALL, 8);
	let clip = bink(CLUSTER + 0x300);
	let mut xex = b"XEX2".to_vec();
	xex.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0x30, 0, 0, 0, 0, 0, 0, 0, 0, 0x18]);
	xex.extend_from_slice(&[0, 0, 0, 0, 0, 0x12, 0x34, 0x56]);

	builder.record(1, 0, Record::file("clip2.bik", 41, clip.len() as u32).deleted());
	builder.record(1, 1, Record::file("game.xex", 50, xex.len() as u32));
	builder.write_cluster(1, 0x80, &[0; 0x40]);
	builder.record(1, 3, Record::file("stale.dat", 0, 0));
	builder.file(&[50], &xex);

	builder.record(40, 0, Record::file("clip.bik", 41, clip.len() as u32));
	builder.record(40, 1, Record::file("note.txt", 0, 0).deleted());
	builder.data(&[41, 42], &clip);
	builder.data(&[60], &xex);

	(builder, clip)
}

#[test]
fn scan_finds_orphans_carves_signatures_and_matches_them() {
	let (builder, _) = carving_volume();
	let volume = open(&builder.image);
	let listing = volume.walk(&builder.image).unwrap().deny_warnings().unwrap();

	assert_eq!(paths(&listing), ["/game.xex"]);
	assert_eq!(listing.slack().len(), 1);
	assert_eq!(listing.slack()[0].orphan().record.name(), "stale.dat");
	assert_eq!(listing.slack()[0].orphan().origin, OrphanOrigin::DirectorySlack { directory: cluster(1) });

	let report = volume.scan(&builder.image, &listing, ScanOptions::new().with_read_size(0x3000)).unwrap();

	assert_eq!(u64::from(volume.image_cluster_count()) - report.scanned_clusters, 2);
	let orphans: Vec<_> = report
		.orphans
		.iter()
		.map(|orphan| (orphan.record.name(), orphan.record.location().offset, orphan.confidence))
		.collect();
	assert_eq!(orphans, [("clip.bik", 0, Confidence::High), ("note.txt", 0x40, Confidence::High)]);
	assert_eq!(report.orphans[0].first_cluster_free, Some(true));

	let carved: Vec<_> =
		report.carved.iter().map(|found| (found.cluster, found.content.kind, found.explained)).collect();
	assert_eq!(carved, [(cluster(41), ContentKind::Bink, true), (cluster(60), ContentKind::Xex2, false)]);
	assert_eq!(report.carved[0].content.size, DeclaredSize::Exact(CLUSTER as u64 + 0x300));

	let matches: Vec<_> =
		report.matches.iter().map(|found| (found.entry, found.reasons.clone(), found.confidence)).collect();
	let reasons = vec![MatchReason::FirstCluster, MatchReason::SizeAgrees, MatchReason::KindAgrees];
	assert!(matches.contains(&(EntryReference::Deleted(0), reasons.clone(), Confidence::High)));
	assert!(matches.contains(&(EntryReference::FreeOrphan(0), reasons, Confidence::High)));
	assert_eq!(matches.len(), 2);

	assert_eq!(report.deleted.len(), 1);
	assert_eq!(report.deleted[0].confidence, Confidence::High);
	assert_eq!(report.deleted[0].content.as_ref().unwrap().size, Agreement::Agrees);
}

#[test]
fn cyclic_chains_are_errors() {
	let mut builder = Builder::new(SMALL, 8);
	builder.record(1, 0, Record::file("loop.bin", 20, 3 * CLUSTER as u32));
	builder.set_fat(20, 21);
	builder.set_fat(21, 20);
	let volume = open(&builder.image);

	let (listing, warnings) = volume.walk(&builder.image).unwrap().into_parts();

	let cycle = ChainError::Cycle { first: cluster(20) };
	assert_eq!(warnings, [Warning::BrokenChain { path: "/loop.bin".to_owned(), error: cycle }]);
	assert_eq!(volume.chain(cluster(20)), Err(cycle));
	assert!(matches!(
		volume.read_file(&builder.image, listing.find("/loop.bin").unwrap().record()),
		Err(FatxError::Chain(error)) if error == cycle
	));
}

#[test]
fn recovered_directories_do_not_repeat_shared_clusters() {
	// Deleted /a fills cluster 30 and runs on into free cluster 31, which deleted /b names as
	// its first cluster.
	let mut builder = Builder::new(SMALL, 8);
	builder.record(1, 0, Record::directory("a", 30).deleted());
	builder.record(1, 1, Record::directory("b", 31).deleted());
	for slot in 0..64 {
		let name = format!("a{slot:02}");
		builder.record(30, slot, Record::file(&name, 0, 0).deleted());
	}
	builder.write_cluster(31, 0, &vec![0xff; CLUSTER]);
	builder.record(31, 0, Record::file("shared.txt", 0, 0).deleted());
	let volume = open(&builder.image);

	let listing = volume.walk(&builder.image).unwrap().deny_warnings().unwrap();

	let shared: Vec<String> = listing
		.deleted()
		.iter()
		.filter(|entry| entry.record().name() == "shared.txt")
		.map(|entry| entry.path())
		.collect();
	assert_eq!(shared, ["/a/shared.txt"]);
	assert_eq!(listing.recovered_clusters(), [cluster(30), cluster(31)]);
}

#[test]
fn async_operations_match_sync() {
	let (builder, _) = carving_volume();
	let volume = open(&builder.image);
	let listing = volume.walk(&builder.image).unwrap();
	let report = volume.scan(&builder.image, listing.value(), ScanOptions::new()).unwrap();
	let game = listing.value().find("/game.xex").unwrap().record();

	block_on(async {
		let async_volume = Volume::open_async(&builder.image, 0..SMALL).await.unwrap();
		assert_eq!(async_volume, volume);
		let async_listing = volume.walk_async(&builder.image).await.unwrap();
		assert_eq!(async_listing, listing);
		assert_eq!(volume.scan_async(&builder.image, listing.value(), ScanOptions::new()).await.unwrap(), report);
		assert_eq!(
			volume.read_file_async(&builder.image, game).await.unwrap(),
			volume.read_file(&builder.image, game).unwrap()
		);
	});
}

#[test]
fn volume_file_shorter_than_its_partition_infers_the_fat() {
	// As in a flash .fdf file: a FAT for a 0x1000000-byte partition in a 0x8000-byte file.
	let mut builder = Builder::new(0x0100_0000, 8);
	let data = pattern(5, 2 * CLUSTER + 0x40);
	builder.record(1, 0, Record::file("launch.xex", 2, data.len() as u32));
	builder.file(&[2, 3, 4], &data);
	builder.record(1, 1, Record::file("gone.bin", 5, 10).deleted());
	let length = builder.cluster_offset(5);
	let image = file_prefix(&builder, length);

	let inference = Volume::infer_partition_length(&image, 0..length, PartitionKind::Standard).unwrap();
	assert_eq!(inference.fat_width(), FatWidth::Fat16);
	assert_eq!(inference.fat_length(), 0x3000);
	assert_eq!(inference.partition_lengths(), 0x0100_0000..0x0180_0000);
	assert_eq!(inference.evidence(), &RootEvidence { fat_entry: FatEntry::Last, live_records: 1, deleted_records: 1 });
	let rejected: Vec<_> =
		inference.rejected().iter().map(|candidate| (candidate.fat_length, candidate.reason)).collect();
	assert_eq!(rejected, [(0x1000, CandidateRejection::EmptyRoot), (0x2000, CandidateRejection::EmptyRoot)]);

	let volume = Volume::open_with(&image, 0..length, inference.volume_options()).unwrap();
	assert_eq!(volume.geometry().file_area_offset(), 0x4000);
	let listing = volume.walk(&image).unwrap().deny_warnings().unwrap();
	assert_eq!(paths(&listing), ["/launch.xex"]);
	assert_eq!(volume.read_file(&image, listing.find("/launch.xex").unwrap().record()).unwrap(), data);

	// Every length in the range places the image's clusters the same way.
	let longest = VolumeOptions::new().with_partition_length(0x017f_f000);
	let longest = Volume::open_with(&image, 0..length, longest).unwrap();
	assert_eq!(longest.geometry().file_area_offset(), 0x4000);
	assert_eq!(longest.walk(&image).unwrap().deny_warnings().unwrap(), listing);

	// The file's own length gives a one-page FAT, which puts the root inside the real FAT.
	let unaware = Volume::open(&image, 0..length).unwrap();
	assert!(unaware.walk(&image).unwrap().ignore_warnings().entries().is_empty());

	let bytes = image.read(0..length);
	let header = VolumeHeader::parse(bytes[..0x10].try_into().unwrap()).unwrap();
	assert_eq!(infer_partition_length(&header, PartitionKind::Standard, &bytes).unwrap(), inference);
	assert_eq!(
		block_on(Volume::infer_partition_length_async(&image, 0..length, PartitionKind::Standard)).unwrap(),
		inference
	);
}

#[test]
fn volume_file_with_a_32_bit_fat_infers_the_width() {
	let mut builder = Builder::new(0x1000_0000, 8);
	builder.record(1, 0, Record::file("note.txt", 2, 5));
	builder.file(&[2], b"hello");
	let length = builder.cluster_offset(3);
	let image = file_prefix(&builder, length);

	let inference = Volume::infer_partition_length(&image, 0..length, PartitionKind::Standard).unwrap();
	assert_eq!(inference.fat_width(), FatWidth::Fat32);
	assert_eq!(inference.fat_length(), 0x41000);
	assert_eq!(inference.partition_lengths(), 0x1000_0000..0x1040_0000);
	assert!(inference.rejected().iter().all(|candidate| candidate.fat_length < 0x41000));

	let volume = Volume::open_with(&image, 0..length, inference.volume_options()).unwrap();
	let listing = volume.walk(&image).unwrap().deny_warnings().unwrap();
	assert_eq!(volume.read_file(&image, listing.find("/note.txt").unwrap().record()).unwrap(), b"hello");
}

#[test]
fn volume_file_with_an_empty_root_is_not_inferred() {
	let builder = Builder::new(0x0100_0000, 8);
	let length = builder.cluster_offset(2);
	let image = file_prefix(&builder, length);

	let error = Volume::infer_partition_length(&image, 0..length, PartitionKind::Standard).unwrap_err();
	let FatxError::PartitionLengthNotInferred { rejected } = error else {
		panic!("unexpected error: {error}");
	};
	let lengths: Vec<_> = rejected.iter().map(|candidate| candidate.fat_length).collect();
	assert_eq!(lengths, [0x1000, 0x2000, 0x3000]);
	assert!(rejected.iter().all(|candidate| candidate.reason == CandidateRejection::EmptyRoot));
}

#[test]
fn inference_rejects_cluster_0_for_directories_and_files_with_data() {
	let mut builder = Builder::new(0x0100_0000, 8);
	builder.record(1, 0, Record::directory("saves", 0));
	builder.record(1, 1, Record::file("empty.txt", 0, 0));
	let length = builder.cluster_offset(2);
	let image = file_prefix(&builder, length);

	let error = Volume::infer_partition_length(&image, 0..length, PartitionKind::Standard).unwrap_err();
	let FatxError::PartitionLengthNotInferred { rejected } = error else {
		panic!("unexpected error: {error}");
	};
	let location = fatx::EntryLocation { cluster: cluster(1), offset: 0 };
	assert!(rejected.iter().any(|candidate| candidate.fat_length == 0x3000
		&& candidate.reason == CandidateRejection::MissingFirstCluster { location }));

	// An empty file alone is evidence.
	builder.record(1, 0, Record::file("other.txt", 0, 0));
	let image = file_prefix(&builder, length);
	let inference = Volume::infer_partition_length(&image, 0..length, PartitionKind::Standard).unwrap();
	assert_eq!(inference.fat_length(), 0x3000);
}

#[test]
fn alternate_partition_has_a_32_bit_fat_and_16_kib_clusters() {
	// 0x401 FAT entries, which a standard partition would hold in 16 bits.
	const PARTITION: u64 = 0x0100_0000;

	let mut builder = Builder::alternate(PARTITION);
	builder.record(1, 0, Record::file("save.bin", 2, 5));
	builder.file(&[2], b"hello");
	let alternate = VolumeOptions::new().with_kind(PartitionKind::Alternate);

	let volume = Volume::open_with(&builder.image, 0..PARTITION, alternate).unwrap();
	assert_eq!(volume.geometry().fat_width(), FatWidth::Fat32);
	assert_eq!(volume.geometry().file_area_offset(), 0x3000);
	let listing = volume.walk(&builder.image).unwrap().deny_warnings().unwrap();
	assert_eq!(volume.read_file(&builder.image, listing.find("/save.bin").unwrap().record()).unwrap(), b"hello");

	let standard = Volume::open(&builder.image, 0..PARTITION).unwrap();
	assert_eq!(standard.geometry().fat_width(), FatWidth::Fat16);
	assert_ne!(standard.geometry().file_area_offset(), 0x3000);

	let length = builder.cluster_offset(3);
	let image = file_prefix(&builder, length);
	let inference = Volume::infer_partition_length(&image, 0..length, PartitionKind::Alternate).unwrap();
	assert_eq!(
		(inference.kind(), inference.fat_width(), inference.fat_length()),
		(PartitionKind::Alternate, FatWidth::Fat32, 0x2000)
	);
	assert_eq!(inference.volume_options().kind(), PartitionKind::Alternate);

	let small_clusters = Builder::new(PARTITION, 8);
	assert!(matches!(
		Volume::open_with(&small_clusters.image, 0..PARTITION, alternate),
		Err(FatxError::Geometry(GeometryError::AlternateClusterSize { bytes_per_cluster: 0x1000, required: 0x4000 }))
	));
}

#[test]
fn chain_past_the_file_size_is_checked_whole() {
	let mut builder = Builder::new(SMALL, 8);
	builder.record(1, 0, Record::file("tail.bin", 20, 5));
	builder.data(&[20], b"hello");
	builder.set_fat(20, 21);
	builder.set_fat(21, 20);
	builder.record(1, 1, Record::file("free.bin", 30, 5));
	builder.data(&[30], b"world");
	builder.set_fat(30, 31);
	builder.record(1, 2, Record::file("owner.bin", 40, 5));
	builder.file(&[40, 41], b"owner");
	builder.record(1, 3, Record::file("thief.bin", 42, 5));
	builder.file(&[42, 41], b"thief");
	let volume = open(&builder.image);

	let (listing, warnings) = volume.walk(&builder.image).unwrap().into_parts();

	assert_eq!(
		warnings,
		[
			Warning::BrokenChain { path: "/tail.bin".to_owned(), error: ChainError::Cycle { first: cluster(20) } },
			Warning::BrokenChain {
				path: "/free.bin".to_owned(),
				error: ChainError::InvalidLink { cluster: cluster(31), next: 0 },
			},
			Warning::CrossLinkedCluster { path: "/thief.bin".to_owned(), cluster: cluster(41) },
		]
	);
	// The driver reads only the clusters the size needs.
	assert_eq!(volume.read_file(&builder.image, listing.find("/tail.bin").unwrap().record()).unwrap(), b"hello");
}

#[test]
fn names_that_make_the_path_too_long_are_skipped() {
	let mut builder = Builder::new(SMALL, 8);
	let names = ["a", "b", "c", "d", "e"].map(|letter| letter.repeat(42));
	let mut parent = 1;
	for (index, name) in names.iter().enumerate() {
		let cluster = index as u32 + 2;
		builder.record(parent, 0, Record::directory(name, cluster));
		builder.directory(&[cluster]);
		parent = cluster;
	}
	// Five 42-byte names and their separators make a 215-byte path; 250 - 1 - 215 bytes remain.
	let fits = "f".repeat(34);
	let overlong = "g".repeat(35);
	builder.record(parent, 0, Record::file(&fits, 0, 0));
	builder.record(parent, 1, Record::file(&overlong, 0, 0));
	let volume = open(&builder.image);

	let (listing, warnings) = volume.walk(&builder.image).unwrap().into_parts();

	let deepest = format!("/{}", names.join("/"));
	assert!(listing.find(&format!("{deepest}/{fits}")).is_some());
	assert!(listing.find(&format!("{deepest}/{overlong}")).is_none());
	assert_eq!(
		warnings,
		[Warning::InvalidEntry {
			location: fatx::EntryLocation { cluster: cluster(parent), offset: 0x40 },
			reason: InvalidEntryReason::PathTooLong { length: 35, maximum: 34 },
		}]
	);
}

#[cfg(feature = "vfs")]
#[test]
fn vfs_serves_live_files() {
	use std::io::Read;

	use fatx::fatx_vfs::FatxVfs;
	use vfs::FileSystem;

	let (builder, readme) = nested_volume();
	let volume = open(&builder.image);
	let listing = volume.walk(&builder.image).unwrap().deny_warnings().unwrap();
	let filesystem = FatxVfs::new(builder.image.clone(), volume, listing);

	let mut children: Vec<String> = filesystem.read_dir("/docs").unwrap().collect();
	children.sort();
	assert_eq!(children, ["deep"]);

	let mut data = Vec::new();
	filesystem.open_file("/readme.txt").unwrap().read_to_end(&mut data).unwrap();
	assert_eq!(data, readme);

	let metadata = filesystem.metadata("/readme.txt").unwrap();
	assert_eq!(metadata.len, readme.len() as u64);
	assert_eq!(metadata.modified, Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_284_058_842)));
}
