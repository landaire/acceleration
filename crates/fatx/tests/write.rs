//! Writes on synthetic images: format, then change, then read back with the reader and the
//! forensic listing.

// These tests use the sparse image and the data patterns, not the volume builder.
#[allow(dead_code)]
mod common;

use std::fmt::Write as _;
use std::io;
use std::ops::Range;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use common::Image;
use common::pattern;
use common::riff;
use fatx::Change;
use fatx::ClusterNumber;
use fatx::DeletedOrigin;
use fatx::EntryInfo;
use fatx::FatEntry;
use fatx::FatWidth;
use fatx::FatxError;
use fatx::FileAttributes;
use fatx::FormatOptions;
use fatx::GeometryError;
use fatx::Listing;
use fatx::NameError;
use fatx::PartitionKind;
use fatx::PathError;
use fatx::ReadAt;
use fatx::Request;
use fatx::SerialNumber;
use fatx::Timestamp;
use fatx::UpdateError;
use fatx::Volume;
use fatx::VolumeOptions;
use fatx::Warning;
use fatx::WriteAt;
use fatx::WriteMode;
use fatx::forensic::Confidence;
use fatx::forensic::recover::RecoveryStrategy;
use fatx::io::MemorySource;
use futures::executor::block_on;

const LENGTH: u64 = 0x0100_0000;

fn now() -> Timestamp {
	Timestamp::from_fields(2026, 11, 1, 12, 34, 56).unwrap()
}

fn later() -> Timestamp {
	Timestamp::from_fields(2026, 11, 2, 8, 0, 0).unwrap()
}

fn formatted(options: FormatOptions) -> (MemorySource, Volume) {
	let source = MemorySource::zeroed(LENGTH as usize);
	let volume = Volume::format(&source, 0..LENGTH, options).unwrap();

	(source, volume)
}

fn standard() -> (MemorySource, Volume) {
	formatted(FormatOptions::new(SerialNumber::new(0x51ec_f2c0)))
}

fn reopen<S>(source: &S, kind: PartitionKind) -> Volume
where
	S: ReadAt,
{
	Volume::open_with(source, 0..LENGTH, VolumeOptions::new().with_kind(kind)).unwrap()
}

fn mkdir(volume: &mut Volume, source: &MemorySource, path: &str) -> Change {
	volume.update(source, Request::create_directory(path, now()).unwrap()).unwrap()
}

fn put(volume: &mut Volume, source: &MemorySource, path: &str, data: &[u8]) -> Change {
	volume.update(source, Request::write_file(path, data, WriteMode::Replace, now()).unwrap()).unwrap()
}

/// A listing as text: one line per live entry, then the deleted ones.
fn describe(listing: &Listing) -> String {
	let mut text = String::new();
	for entry in listing.entries() {
		let record = entry.record();
		let first = record.first_cluster().map_or(0, ClusterNumber::get);
		writeln!(
			text,
			"{} attributes {:#04x} first {first:#x} size {:#x} created {} modified {}",
			entry.path(),
			record.attributes().bits(),
			record.size(),
			record.created(),
			record.modified(),
		)
		.unwrap();
	}
	for deleted in listing.deleted() {
		let record = deleted.record();
		let first = record.first_cluster().map_or(0, ClusterNumber::get);
		writeln!(text, "deleted {} first {first:#x} size {:#x}", deleted.path(), record.size()).unwrap();
	}

	text
}

fn read(volume: &Volume, source: &MemorySource, listing: &Listing, path: &str) -> Vec<u8> {
	volume.read_file(source, listing.find(path).unwrap().record()).unwrap()
}

fn free_clusters(volume: &Volume) -> usize {
	(1..=volume.image_cluster_count())
		.filter(|cluster| volume.fat().is_free(ClusterNumber::new(*cluster).unwrap()))
		.count()
}

/// Builds a small tree; returns the file contents.
fn populate(volume: &mut Volume, source: &MemorySource) -> (Vec<u8>, Vec<u8>) {
	let save = pattern(0x31, 0x9000);
	let wav = riff(0x5000);
	mkdir(volume, source, "/Content");
	mkdir(volume, source, "/Content/0000000000000000");
	put(volume, source, "/Content/0000000000000000/save.bin", &save);
	put(volume, source, "/name.wav", &wav);
	put(volume, source, "/empty", b"");

	(save, wav)
}

#[test]
fn format_writes_what_xam_writes() {
	let (source, volume) = standard();
	let image = source.into_inner();

	assert_eq!(&image[..4], b"XTAF");
	assert_eq!(&image[4..16], [0x51, 0xec, 0xf2, 0xc0, 0, 0, 0, 0x20, 0, 0, 0, 1]);
	assert_eq!(&image[16..18], [0, 0]);
	assert!(image[18..0x1000].iter().all(|byte| *byte == 0xff));
	assert_eq!(volume.geometry().fat_width(), FatWidth::Fat16);
	assert_eq!(&image[0x1000..0x1004], [0xff, 0xf8, 0xff, 0xff]);
	insta::assert_debug_snapshot!(volume.geometry());
}

#[test]
fn round_trip_fat16() {
	let (source, mut volume) = standard();
	let (save, wav) = populate(&mut volume, &source);

	let reopened = reopen(&source, PartitionKind::Standard);
	assert_eq!(reopened, volume);
	let listing = reopened.walk(&source).unwrap().deny_warnings().unwrap();
	assert_eq!(read(&reopened, &source, &listing, "/Content/0000000000000000/save.bin"), save);
	assert_eq!(read(&reopened, &source, &listing, "/name.wav"), wav);
	assert_eq!(read(&reopened, &source, &listing, "/empty"), b"");
	insta::assert_snapshot!(describe(&listing));
}

#[test]
fn round_trip_alternate_fat32() {
	let options = FormatOptions::new(SerialNumber::new(7)).with_kind(PartitionKind::Alternate);
	let (source, mut volume) = formatted(options);
	assert_eq!(volume.geometry().fat_width(), FatWidth::Fat32);
	let (save, _) = populate(&mut volume, &source);

	let reopened = reopen(&source, PartitionKind::Alternate);
	assert_eq!(reopened, volume);
	let listing = reopened.walk(&source).unwrap().deny_warnings().unwrap();
	assert_eq!(read(&reopened, &source, &listing, "/Content/0000000000000000/save.bin"), save);
	assert_eq!(&source.to_vec()[0x1000..0x1008], [0xff, 0xff, 0xff, 0xf8, 0xff, 0xff, 0xff, 0xff]);
}

/// A sparse writable image for a partition too large to hold in memory.
#[derive(Debug)]
struct Sparse(Mutex<Image>);

impl ReadAt for Sparse {
	fn read_at(&self, _context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		Ok(self.0.lock().unwrap().read(range))
	}
}

impl WriteAt for Sparse {
	fn write_at(&self, offset: u64, data: &[u8]) -> io::Result<()> {
		// Zero pages stay unallocated.
		if data.iter().any(|byte| *byte != 0) {
			self.0.lock().unwrap().write(offset, data);
		}
		Ok(())
	}

	fn flush(&self) -> io::Result<()> {
		Ok(())
	}
}

#[test]
fn round_trip_standard_fat32() {
	// 0xfff0 clusters of 4 KiB make the FAT 32-bit.
	let length = 0xfff1 * 0x1000;
	let source = Sparse(Mutex::new(Image::new(length)));
	let options = FormatOptions::new(SerialNumber::new(9)).with_cluster_size(0x1000);
	let mut volume = Volume::format(&source, 0..length, options).unwrap();
	assert_eq!(volume.geometry().fat_width(), FatWidth::Fat32);

	let data = pattern(0x77, 0x2345);
	volume.update(&source, Request::create_directory("/a", now()).unwrap()).unwrap();
	volume.update(&source, Request::write_file("/a/b", &data, WriteMode::Replace, now()).unwrap()).unwrap();

	let reopened = Volume::open(&source, 0..length).unwrap();
	let listing = reopened.walk(&source).unwrap().deny_warnings().unwrap();
	assert_eq!(reopened.read_file(&source, listing.find("/a/b").unwrap().record()).unwrap(), data);
	assert_eq!(listing.find("/a/b").unwrap().record().first_cluster(), ClusterNumber::new(3));
}

#[test]
fn deleted_file_is_listed_and_recovers_with_high_confidence() {
	let (source, mut volume) = standard();
	let (_, wav) = populate(&mut volume, &source);
	let free_before = free_clusters(&volume);

	let change = volume.update(&source, Request::remove("/NAME.WAV").unwrap()).unwrap();
	let Change::Removed { path, record } = &change else { panic!("expected a removal, got {change:?}") };
	assert_eq!(path, "/name.wav");
	assert!(record.is_marked_deleted());
	assert_eq!(free_clusters(&volume), free_before + 2);

	let reopened = reopen(&source, PartitionKind::Standard);
	let listing = reopened.walk(&source).unwrap().deny_warnings().unwrap();
	assert!(listing.find("/name.wav").is_none());
	let deleted = listing.deleted().iter().find(|entry| entry.path() == "/name.wav").unwrap();
	assert_eq!(deleted.origin(), DeletedOrigin::LiveDirectory);
	assert_eq!(deleted.record().size() as usize, wav.len());

	let recovery = reopened.recover(&source, deleted.record(), RecoveryStrategy::ContiguousFree).unwrap();
	assert_eq!(recovery.confidence, Confidence::High);
	assert_eq!(recovery.segments[0].bytes, wav);
}

#[test]
fn removing_a_directory_needs_it_empty() {
	let (source, mut volume) = standard();
	populate(&mut volume, &source);

	let error = volume.update(&source, Request::remove("/Content").unwrap()).unwrap_err();
	assert!(matches!(&error, FatxError::Update(UpdateError::DirectoryNotEmpty { path }) if path == "/Content"));

	volume.update(&source, Request::remove("/Content/0000000000000000/save.bin").unwrap()).unwrap();
	volume.update(&source, Request::remove("/Content/0000000000000000").unwrap()).unwrap();
	volume.update(&source, Request::remove("/Content").unwrap()).unwrap();
	let listing = reopen(&source, PartitionKind::Standard).walk(&source).unwrap().deny_warnings().unwrap();
	insta::assert_snapshot!(describe(&listing));
}

#[test]
fn full_volume_is_a_typed_error_and_writes_nothing() {
	let (source, mut volume) = standard();
	let free = free_clusters(&volume) as u64;
	let image = source.to_vec();
	let too_large = vec![0x5a; (free as usize + 1) * 0x4000];

	let error = volume
		.update(&source, Request::write_file("/big", &too_large, WriteMode::Replace, now()).unwrap())
		.unwrap_err();

	assert!(
		matches!(error, FatxError::Update(UpdateError::DiskFull { needed, available }) if needed == free + 1 && available == free)
	);
	assert_eq!(source.to_vec(), image);
	assert_eq!(free_clusters(&volume) as u64, free);

	let fits = vec![0x5a; free as usize * 0x4000];
	put(&mut volume, &source, "/big", &fits);
	assert_eq!(free_clusters(&volume), 0);
	let error = volume.update(&source, Request::create_directory("/more", now()).unwrap()).unwrap_err();
	assert!(matches!(error, FatxError::Update(UpdateError::DiskFull { needed: 1, available: 0 })));
}

#[test]
fn directory_holds_at_most_4096_records() {
	let (source, mut volume) = standard();
	mkdir(&mut volume, &source, "/d");
	for index in 0..4096 {
		put(&mut volume, &source, &format!("/d/{index}"), b"");
	}

	let error =
		volume.update(&source, Request::write_file("/d/x", b"", WriteMode::Replace, now()).unwrap()).unwrap_err();
	assert!(matches!(&error, FatxError::Update(UpdateError::DirectoryFull { path, maximum: 4096 }) if path == "/d"));

	// A deleted record's slot takes the next record.
	volume.update(&source, Request::remove("/d/17").unwrap()).unwrap();
	let change = put(&mut volume, &source, "/d/x", b"");
	assert_eq!(
		change.record().location(),
		fatx::EntryLocation { cluster: ClusterNumber::new(2).unwrap(), offset: 17 * 0x40 }
	);
	let listing = reopen(&source, PartitionKind::Standard).walk(&source).unwrap().deny_warnings().unwrap();
	assert_eq!(listing.entries().len(), 4097);
}

#[test]
fn names_follow_the_driver_rules() {
	let (source, mut volume) = standard();
	mkdir(&mut volume, &source, "/Content");

	let invalid = |path: &str| Request::create_directory(path, now()).unwrap_err();
	let reason = |name: &str, reason| UpdateError::InvalidPath(PathError { name: name.to_owned(), reason });
	assert_eq!(invalid("/a:b"), reason("a:b", NameError::IllegalCharacter { character: ':' }));
	assert_eq!(invalid("/.."), reason("..", NameError::DotName));
	let long = "n".repeat(43);
	assert_eq!(invalid(&format!("/{long}")), reason(&long, NameError::TooLong { length: 43 }));
	assert_eq!(invalid("/"), UpdateError::RootDirectory);

	let error = volume.update(&source, Request::create_directory("/CONTENT", now()).unwrap()).unwrap_err();
	assert!(matches!(&error, FatxError::Update(UpdateError::AlreadyExists { path }) if path == "/Content"));
	let error =
		volume.update(&source, Request::write_file("/content", b"x", WriteMode::Replace, now()).unwrap()).unwrap_err();
	assert!(matches!(error, FatxError::Update(UpdateError::IsADirectory { .. })));
	let error = volume.update(&source, Request::create_directory("/missing/a", now()).unwrap()).unwrap_err();
	assert!(matches!(&error, FatxError::Update(UpdateError::NotFound { path }) if path == "/missing"));

	// Five 42-byte names and a separator each make 215 bytes; a 35-byte name passes 250.
	let mut path = String::new();
	for level in 0..5 {
		path.push_str(&format!("/{level}{}", "d".repeat(41)));
		mkdir(&mut volume, &source, &path);
	}
	put(&mut volume, &source, &format!("{path}/{}", "f".repeat(34)), b"");
	let long = format!("{path}/{}", "f".repeat(35));
	let error =
		volume.update(&source, Request::write_file(&long, b"", WriteMode::Replace, now()).unwrap()).unwrap_err();
	assert!(matches!(error, FatxError::Update(UpdateError::PathTooLong { length: 251, maximum: 250, .. })));

	reopen(&source, PartitionKind::Standard).walk(&source).unwrap().deny_warnings().unwrap();
}

#[test]
fn file_data_writes_extend_and_truncate() {
	let (source, mut volume) = standard();
	let first = pattern(0x10, 0x5000);
	put(&mut volume, &source, "/f", &first);
	let free = free_clusters(&volume);

	let tail = pattern(0x20, 0x4000);
	let request = Request::write_file("/f", &tail, WriteMode::Append, later()).unwrap();
	let change = volume.update(&source, request).unwrap();
	assert_eq!(change.record().size(), 0x9000);
	assert_eq!(change.record().modified(), later());
	assert_eq!(change.record().created(), now());
	assert_eq!(free_clusters(&volume), free - 1);

	let patch = pattern(0x30, 0x10);
	volume.update(&source, Request::write_file("/f", &patch, WriteMode::At(0x9010), now()).unwrap()).unwrap();
	let mut expected = [first.clone(), tail.clone()].concat();
	expected.extend([0; 0x10]);
	expected.extend(&patch);
	let listing = volume.walk(&source).unwrap().deny_warnings().unwrap();
	assert_eq!(read(&volume, &source, &listing, "/f"), expected);

	volume.update(&source, Request::set_length("/f", 0x10).unwrap()).unwrap();
	assert_eq!(free_clusters(&volume), free + 1);
	volume.update(&source, Request::set_length("/f", 0x4010).unwrap()).unwrap();
	let listing = volume.walk(&source).unwrap().deny_warnings().unwrap();
	let mut grown = first[..0x10].to_vec();
	grown.resize(0x4010, 0);
	assert_eq!(read(&volume, &source, &listing, "/f"), grown);

	volume.update(&source, Request::set_length("/f", 0).unwrap()).unwrap();
	let listing = volume.walk(&source).unwrap().deny_warnings().unwrap();
	assert_eq!(listing.find("/f").unwrap().record().first_cluster(), None);
	assert_eq!(free_clusters(&volume), free + 2);

	let error = volume
		.update(&source, Request::write_file("/f", b"x", WriteMode::At(0xffff_c001), now()).unwrap())
		.unwrap_err();
	assert!(matches!(error, FatxError::Update(UpdateError::FileTooLarge { maximum: 0xffff_c000 })));
}

#[test]
fn replacing_a_file_reuses_its_clusters() {
	let (source, mut volume) = standard();
	put(&mut volume, &source, "/f", &pattern(1, 0x8000));
	put(&mut volume, &source, "/g", &pattern(2, 0x10));

	let change = volume
		.update(&source, Request::write_file("/F", &pattern(3, 0x9000), WriteMode::Replace, later()).unwrap())
		.unwrap();

	// The two freed clusters come first, then the first free one after /g.
	let runs = volume.chain(change.record().first_cluster().unwrap()).unwrap();
	let clusters: Vec<u32> = runs.iter().flat_map(|run| run.clusters()).map(ClusterNumber::get).collect();
	assert_eq!(clusters, [2, 3, 5]);
	assert_eq!(change.record().created(), later());
	let listing = volume.walk(&source).unwrap().deny_warnings().unwrap();
	assert_eq!(read(&volume, &source, &listing, "/f"), pattern(3, 0x9000));
}

#[test]
fn rename_in_place_and_move() {
	let (source, mut volume) = standard();
	let (save, _) = populate(&mut volume, &source);

	let change = volume.update(&source, Request::rename("/name.wav", "/Renamed.wav").unwrap()).unwrap();
	assert!(matches!(&change, Change::Renamed { from, to, .. } if from == "/name.wav" && to == "/Renamed.wav"));
	volume.update(&source, Request::rename("/Content/0000000000000000/save.bin", "/save.bin").unwrap()).unwrap();
	volume.update(&source, Request::rename("/Content/0000000000000000", "/moved").unwrap()).unwrap();

	let error = volume.update(&source, Request::rename("/Content", "/Content/inner").unwrap()).unwrap_err();
	assert!(matches!(error, FatxError::Update(UpdateError::MoveIntoItself { .. })));
	let error = volume.update(&source, Request::rename("/save.bin", "/moved").unwrap()).unwrap_err();
	assert!(matches!(error, FatxError::Update(UpdateError::AlreadyExists { .. })));
	volume.update(&source, Request::rename("/moved", "/MOVED").unwrap()).unwrap();

	let listing = reopen(&source, PartitionKind::Standard).walk(&source).unwrap().deny_warnings().unwrap();
	assert_eq!(read(&volume, &source, &listing, "/save.bin"), save);
	insta::assert_snapshot!(describe(&listing));
}

#[test]
fn attributes_and_timestamps() {
	let (source, mut volume) = standard();
	populate(&mut volume, &source);

	let info = EntryInfo { attributes: Some(FileAttributes::HIDDEN), modified: Some(later()), ..EntryInfo::default() };
	let change = volume.update(&source, Request::set_info("/Content", info).unwrap()).unwrap();
	assert_eq!(change.record().attributes().bits(), 0x12);
	assert_eq!(change.record().modified(), later());
	assert_eq!(change.record().created(), now());

	let info = EntryInfo { attributes: Some(FileAttributes::READ_ONLY), ..EntryInfo::default() };
	let error = volume.update(&source, Request::set_info("/empty", info).unwrap()).unwrap_err();
	assert!(matches!(error, FatxError::Update(UpdateError::UnsupportedAttributes { attributes: 1 })));
	let info = EntryInfo { attributes: Some(FileAttributes::DIRECTORY), ..EntryInfo::default() };
	let error = volume.update(&source, Request::set_info("/empty", info).unwrap()).unwrap_err();
	assert!(matches!(error, FatxError::Update(UpdateError::DirectoryAttributeChange { .. })));
	let invalid = EntryInfo { created: Some(Timestamp::from_raw(0)), ..EntryInfo::default() };
	assert_eq!(Request::set_info("/empty", invalid).unwrap_err(), UpdateError::InvalidTimestamp { raw: 0 });

	let listing = volume.walk(&source).unwrap().deny_warnings().unwrap();
	assert_eq!(listing.find("/Content").unwrap().record().attributes().bits(), 0x12);
}

#[test]
fn async_update_matches_sync_update() {
	let (sync_source, mut sync_volume) = standard();
	let (async_source, mut async_volume) = standard();
	let data = pattern(9, 0x4100);

	put(&mut sync_volume, &sync_source, "/f", &data);
	block_on(
		async_volume.update_async(&async_source, Request::write_file("/f", &data, WriteMode::Replace, now()).unwrap()),
	)
	.unwrap();

	assert_eq!(sync_volume, async_volume);
	assert_eq!(sync_source.to_vec(), async_source.to_vec());
}

/// A source whose writes fail from the `limit`th on, and that logs the offset and length of
/// each write.
#[derive(Debug)]
struct Failing {
	inner: MemorySource,
	remaining: AtomicUsize,
	/// The bytes of the failing write that land; half of them by default.
	prefix: Option<usize>,
	writes: Mutex<Vec<(u64, usize)>>,
}

impl Failing {
	fn new(inner: MemorySource, limit: usize, prefix: Option<usize>) -> Self {
		Self { inner, remaining: AtomicUsize::new(limit), prefix, writes: Mutex::new(Vec::new()) }
	}
}

impl ReadAt for Failing {
	fn read_at(&self, context: &(), range: Range<u64>) -> io::Result<impl AsRef<[u8]>> {
		self.inner.read_at(context, range)
	}
}

impl WriteAt for Failing {
	/// The failing write lands a prefix, as a write cut off part way can.
	fn write_at(&self, offset: u64, data: &[u8]) -> io::Result<()> {
		self.writes.lock().unwrap().push((offset, data.len()));
		if self.remaining.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1)).is_err() {
			let landed = self.prefix.unwrap_or(data.len() / 2).min(data.len());
			self.inner.write_at(offset, &data[..landed])?;
			return Err(io::Error::other("injected failure"));
		}

		self.inner.write_at(offset, data)
	}

	fn flush(&self) -> io::Result<()> {
		Ok(())
	}
}

/// Runs a change on copies of `image` that fail at each of its writes in turn, and checks that
/// every interrupted volume still lists with no warning but lost clusters: the FAT is written
/// before the records that reach its new chains. Each write is cut at every sector boundary
/// inside it, the failures `WriteAt` allows; a directory record write is also cut at every byte,
/// a stricter model than the sectors give.
fn check_interrupted(image: &MemorySource, request: impl Fn() -> Request<'static>) {
	const SECTOR: u64 = 0x200;

	let complete = Failing::new(image.clone(), usize::MAX, None);
	let mut volume = reopen(&complete, PartitionKind::Standard);
	volume.update(&complete, request()).unwrap();
	let writes = complete.writes.into_inner().unwrap();
	assert!(writes.len() > 1, "the change takes several writes");

	for (limit, (offset, length)) in writes.iter().enumerate() {
		let mut prefixes: Vec<usize> = (offset.next_multiple_of(SECTOR)..*offset + *length as u64)
			.step_by(SECTOR as usize)
			.map(|boundary| (boundary - offset) as usize)
			.collect();
		prefixes.insert(0, 0);
		if *length <= 0x40 {
			prefixes = (0..*length).collect();
		}
		prefixes.dedup();

		for prefix in prefixes {
			let failing = Failing::new(image.clone(), limit, Some(prefix));
			let mut volume = reopen(&failing, PartitionKind::Standard);
			let error = volume.update(&failing, request()).unwrap_err();
			assert!(matches!(error, FatxError::Write(_)), "{error}");

			let reopened = reopen(&failing.inner, PartitionKind::Standard);
			let listing = reopened.walk(&failing.inner).unwrap();
			let unexpected: Vec<_> =
				listing.warnings().iter().filter(|warning| !matches!(warning, Warning::LostClusters { .. })).collect();
			assert_eq!(
				unexpected,
				[] as [&Warning; 0],
				"write {limit} of {} cut after {prefix:#x} bytes",
				writes.len()
			);
		}
	}
}

static DATA: [u8; 0x9000] = [0x42; 0x9000];

#[test]
fn interrupted_changes_leave_consistent_volumes() {
	let (source, mut volume) = standard();
	populate(&mut volume, &source);
	// Fill the root directory's cluster, so a new root record extends the directory.
	for index in 0..252 {
		put(&mut volume, &source, &format!("/{index}"), b"");
	}

	check_interrupted(&source, || Request::write_file("/new", &DATA, WriteMode::Replace, now()).unwrap());
	check_interrupted(&source, || Request::create_directory("/dir", now()).unwrap());
	check_interrupted(&source, || Request::write_file("/name.wav", &DATA, WriteMode::Replace, now()).unwrap());
	check_interrupted(&source, || Request::write_file("/name.wav", &DATA, WriteMode::Append, now()).unwrap());
	check_interrupted(&source, || Request::set_length("/name.wav", 0x10).unwrap());
	check_interrupted(&source, || Request::set_length("/name.wav", 0x20000).unwrap());
	check_interrupted(&source, || Request::remove("/name.wav").unwrap());
	check_interrupted(&source, || Request::rename("/Content/0000000000000000/save.bin", "/save.bin").unwrap());
}

#[test]
fn interrupted_format_leaves_no_volume() {
	let options = FormatOptions::new(SerialNumber::new(1));

	// The four writes: zeroed metadata page, FAT, root directory cluster, header. Until the
	// header lands there is no volume.
	for limit in 0..3 {
		let failing = Failing::new(MemorySource::zeroed(LENGTH as usize), limit, None);
		assert!(matches!(Volume::format(&failing, 0..LENGTH, options), Err(FatxError::Write(_))));
		assert!(matches!(
			Volume::open(&failing.inner, 0..LENGTH),
			Err(FatxError::Geometry(GeometryError::BadSignature { .. }))
		));
	}

	// Half a header holds its 16 bytes: the volume is whole, since everything else is durable.
	let header_cut = Failing::new(MemorySource::zeroed(LENGTH as usize), 3, None);
	assert!(matches!(Volume::format(&header_cut, 0..LENGTH, options), Err(FatxError::Write(_))));
	let volume = Volume::open(&header_cut.inner, 0..LENGTH).unwrap();
	assert_eq!(volume.walk(&header_cut.inner).unwrap().deny_warnings().unwrap().entries(), []);

	let complete = Failing::new(MemorySource::zeroed(LENGTH as usize), 4, None);
	Volume::format(&complete, 0..LENGTH, options).unwrap();
}

#[test]
fn chains_link_in_the_fat() {
	let (source, mut volume) = standard();
	put(&mut volume, &source, "/a", &pattern(1, 0x4001));

	assert_eq!(volume.fat().entry(ClusterNumber::new(2).unwrap()), Some(FatEntry::Link(3)));
	assert_eq!(volume.fat().entry(ClusterNumber::new(3).unwrap()), Some(FatEntry::Last));
}

#[cfg(feature = "vfs")]
#[test]
fn vfs_refuses_reads_after_a_failed_change() {
	use fatx::fatx_vfs::FatxVfs;
	use vfs::FileSystem;

	let (source, mut volume) = standard();
	put(&mut volume, &source, "/kept.bin", b"data");
	let listing = volume.walk(&source).unwrap().deny_warnings().unwrap();
	// Every write fails after landing nothing.
	let filesystem = FatxVfs::new(Failing::new(source, 0, Some(0)), volume, listing);

	assert!(filesystem.create_dir("/new").is_err());

	for result in [
		filesystem.open_file("/kept.bin").map(drop),
		filesystem.append_file("/kept.bin").map(drop),
		filesystem.create_dir("/other"),
	] {
		let error = result.unwrap_err();
		assert!(error.to_string().contains("reopen the volume"), "{error}");
	}
	assert!(filesystem.exists("/kept.bin").unwrap());

	#[cfg(feature = "async-vfs")]
	{
		use vfs::async_vfs::AsyncFileSystem;

		let error = block_on(AsyncFileSystem::open_file(&filesystem, "/kept.bin")).map(drop).unwrap_err();
		assert!(error.to_string().contains("reopen the volume"), "{error}");
	}
}

#[cfg(feature = "vfs")]
#[test]
fn vfs_writes() {
	use std::io::Read;
	use std::io::Write;

	use fatx::fatx_vfs::FatxVfs;
	use vfs::FileSystem;

	let (source, volume) = standard();
	let listing = volume.walk(&source).unwrap().deny_warnings().unwrap();
	let filesystem = FatxVfs::new(source, volume, listing);

	filesystem.create_dir("/Content").unwrap();
	let mut file = filesystem.create_file("/Content/a.bin").unwrap();
	file.write_all(b"hello ").unwrap();
	file.flush().unwrap();
	drop(file);
	let mut file = filesystem.append_file("/Content/a.bin").unwrap();
	file.write_all(b"world").unwrap();
	drop(file);
	filesystem.create_dir("/gone").unwrap();
	filesystem.remove_dir("/gone").unwrap();
	filesystem.move_file("/Content/a.bin", "/b.bin").unwrap();

	let mut data = String::new();
	filesystem.open_file("/b.bin").unwrap().read_to_string(&mut data).unwrap();
	assert_eq!(data, "hello world");
	assert!(!filesystem.exists("/gone").unwrap());
	assert!(filesystem.remove_dir("/b.bin").is_err());
	filesystem.remove_file("/b.bin").unwrap();

	let volume = filesystem.volume();
	let listing = volume.walk(filesystem.source()).unwrap().deny_warnings().unwrap();
	assert_eq!(listing.entries().iter().map(|entry| entry.path()).collect::<Vec<_>>(), ["/Content"]);
	// /b.bin took the slot /gone left; /Content/a.bin and /b.bin are deleted records.
	assert_eq!(listing.deleted().len(), 2);
}

#[cfg(feature = "async-vfs")]
#[test]
fn async_vfs_writes() {
	use futures::AsyncReadExt;
	use futures::AsyncWriteExt;
	use vfs::async_vfs::AsyncFileSystem;

	use fatx::fatx_vfs::FatxVfs;

	let (source, volume) = standard();
	let listing = volume.walk(&source).unwrap().deny_warnings().unwrap();
	let filesystem = FatxVfs::new(source, volume, listing);

	block_on(async {
		AsyncFileSystem::create_dir(&filesystem, "/Content").await.unwrap();
		let mut file = AsyncFileSystem::create_file(&filesystem, "/Content/a.bin").await.unwrap();
		file.write_all(b"async").await.unwrap();
		file.close().await.unwrap();

		let mut data = Vec::new();
		// The tree matches paths with their stored case.
		assert!(AsyncFileSystem::open_file(&filesystem, "/content/A.BIN").await.is_err());
		AsyncFileSystem::open_file(&filesystem, "/Content/a.bin").await.unwrap().read_to_end(&mut data).await.unwrap();
		assert_eq!(data, b"async");
	});
}
