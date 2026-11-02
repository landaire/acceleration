mod common;

use common::Builder;
use common::Record;
use common::Scratch;
use common::bink;
use fatx::ClusterNumber;
use fatx::ClusterRun;
use fatx::IntegrityError;
use fatx::Warning;
use fatxtool::FatxToolError;
use fatxtool::OutputError;
use fatxtool::create_output;

const PARTITION: u64 = 0x10_0000;
const CLUSTER: usize = 0x1000;

/// Live files in the root and in /docs, a deleted Bink file whose clusters are still free, a
/// deleted directory with a file in it, and a deleted file in /docs.
fn sample_volume() -> Vec<u8> {
	let mut builder = Builder::new(PARTITION, PARTITION);
	builder.record(1, 0, Record::directory("docs", 2));
	builder.record(1, 1, Record::file("readme.txt", 3, 0x1800));
	builder.record(1, 2, Record::file("intro.bik", 6, 0x1400).deleted());
	builder.record(1, 3, Record::directory("gone", 8).deleted());
	builder.file(&[3, 4], &vec![b'r'; 0x1800]);
	builder.data(6, &bink(0x1400));

	builder.data(8, &[0xff; CLUSTER]);
	builder.record(8, 0, Record::file("lost.txt", 9, 5));
	builder.data(9, b"hello");

	builder.directory(&[2]);
	builder.record(2, 0, Record::file("note.txt", 5, 3));
	builder.record(2, 1, Record::file("draft.txt", 10, 4).deleted());
	builder.file(&[5], b"abc");
	builder.data(10, b"text");

	builder.finish()
}

#[test]
fn list_prints_the_tree() {
	let scratch = Scratch::new("list", sample_volume());

	insta::assert_snapshot!(scratch.run(&["list"]).unwrap(), @r"
	/docs/
	/docs/note.txt
	/readme.txt
	");
}

#[test]
fn long_list_places_deleted_entries_in_the_tree() {
	let scratch = Scratch::new("list-long", sample_volume());

	insta::assert_snapshot!(scratch.run(&["list", "--long", "--deleted"]).unwrap());
}

#[test]
fn scan_matches_carved_content_with_deleted_files() {
	let scratch = Scratch::new("scan", sample_volume());

	insta::assert_snapshot!(scratch.run(&["scan"]).unwrap());
}

#[test]
fn recover_by_path_writes_the_free_clusters() {
	let scratch = Scratch::new("recover-path", sample_volume());

	let report = scratch.run(&["recover", "/INTRO.BIK", &scratch.path("intro.bik").to_string_lossy()]).unwrap();

	insta::assert_snapshot!(report);
	assert_eq!(std::fs::read(scratch.path("intro.bik")).unwrap(), bink(0x1400));
}

#[test]
fn recover_by_cluster_names_the_live_owner() {
	let scratch = Scratch::new("recover-cluster", sample_volume());
	let output = scratch.path("clusters.bin");

	let report = scratch
		.run(&[
			"recover",
			"--cluster",
			"0x3",
			"--size",
			"0x2400",
			"--strategy",
			"contiguous-free",
			&output.to_string_lossy(),
		])
		.unwrap();

	insta::assert_snapshot!(report);
	// Clusters 3 and 4 belong to /readme.txt and are not read; cluster 5 belongs to
	// /docs/note.txt.
	assert_eq!(std::fs::read(output).unwrap(), vec![0; 0x2400]);
}

#[test]
fn volume_file_mounted_through_a_longer_device_needs_its_partition_length() {
	// A FAT for a 0x2000000-byte partition in a 0x8000-byte file.
	let mut builder = Builder::new(0x200_0000, 0x8000);
	builder.record(1, 0, Record::file("a.txt", 2, 5));
	builder.file(&[2], b"hello");
	let scratch = Scratch::new("info", builder.finish());

	// The file's own length puts the root inside the FAT: an empty tree, and a.txt's cluster lost.
	assert_eq!(scratch.run(&["list"]).unwrap(), "");
	let error = scratch.run_strict(&["list"]).unwrap_err();
	let lost =
		Warning::LostClusters { count: 1, runs: vec![ClusterRun::new(ClusterNumber::new(2).unwrap(), 1).unwrap()] };
	assert!(matches!(error, FatxToolError::Integrity(IntegrityError { warnings }) if warnings == [lost]));

	insta::assert_snapshot!(scratch.run_mounted(0x200_0000, &["info"]).unwrap());
	assert_eq!(scratch.run_mounted(0x200_0000, &["list"]).unwrap(), "/a.txt\n");
}

#[test]
fn extract_all_copies_live_files() {
	let scratch = Scratch::new("extract-all", sample_volume());
	let directory = scratch.path("tree");

	let report = scratch.run(&["extract-all", &directory.to_string_lossy()]).unwrap();

	assert_eq!(report, "extracted to SCRATCH/tree: files 2, directories 1, files skipped 0\n");
	assert_eq!(std::fs::read(directory.join("readme.txt")).unwrap(), vec![b'r'; 0x1800]);
	assert_eq!(std::fs::read(directory.join("docs").join("note.txt")).unwrap(), b"abc");
	assert!(!directory.join("intro.bik").exists());
}

#[test]
fn extract_refuses_to_replace_the_input() {
	let image = sample_volume();
	let scratch = Scratch::new("extract-alias", image.clone());

	let error = scratch.run(&["extract", "/readme.txt", &scratch.image_path().to_string_lossy()]).unwrap_err();

	assert!(matches!(error, FatxToolError::Output(OutputError::OutputAliasesInput { .. })));
	assert_eq!(std::fs::read(scratch.image_path()).unwrap(), image);
}

#[test]
fn output_hard_link_cannot_truncate_input() {
	let image = sample_volume();
	let scratch = Scratch::new("hard-link", image.clone());
	let link = scratch.path("link.bin");
	std::fs::hard_link(scratch.image_path(), &link).unwrap();
	let input =
		fatxtool::InputFile::new(&std::fs::File::open(scratch.image_path()).unwrap(), scratch.image_path()).unwrap();

	let error = create_output(&input, &link).unwrap_err();

	assert!(matches!(error, OutputError::OutputAliasesInput { .. }));
	assert_eq!(std::fs::read(scratch.image_path()).unwrap(), image);
}

#[cfg(unix)]
#[test]
fn extract_all_refuses_a_linked_directory() {
	let scratch = Scratch::new("extract-linked-directory", sample_volume());
	let tree = scratch.path("tree");
	let elsewhere = scratch.path("elsewhere");
	std::fs::create_dir_all(&tree).unwrap();
	std::fs::create_dir(&elsewhere).unwrap();
	std::os::unix::fs::symlink(&elsewhere, tree.join("docs")).unwrap();

	let error = scratch.run(&["extract-all", &tree.to_string_lossy()]).unwrap_err();

	assert!(matches!(error, FatxToolError::Output(OutputError::SymbolicLink { path }) if path == tree.join("docs")));
	assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn extract_all_refuses_a_linked_file() {
	let scratch = Scratch::new("extract-linked-file", sample_volume());
	let tree = scratch.path("tree");
	let victim = scratch.path("victim");
	std::fs::create_dir_all(&tree).unwrap();
	std::fs::write(&victim, b"keep").unwrap();
	std::os::unix::fs::symlink(&victim, tree.join("readme.txt")).unwrap();

	let error = scratch.run(&["extract-all", &tree.to_string_lossy()]).unwrap_err();

	assert!(matches!(error, FatxToolError::Output(OutputError::SymbolicLink { .. })));
	assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn recover_refuses_a_linked_output_and_leaves_no_partial_file() {
	let scratch = Scratch::new("recover-linked", sample_volume());
	let victim = scratch.path("victim");
	let link = scratch.path("intro.bik");
	std::fs::write(&victim, b"keep").unwrap();
	std::os::unix::fs::symlink(&victim, &link).unwrap();

	let error = scratch.run(&["recover", "/intro.bik", &link.to_string_lossy()]).unwrap_err();

	assert!(matches!(error, FatxToolError::Output(OutputError::SymbolicLink { .. })));
	assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
	assert_eq!(partial_files(&scratch), Vec::<String>::new());
}

#[test]
fn recover_replaces_the_output_only_when_done() {
	let scratch = Scratch::new("recover-replace", sample_volume());
	let output = scratch.path("intro.bik");
	std::fs::write(&output, b"old").unwrap();

	scratch.run(&["recover", "/intro.bik", &output.to_string_lossy()]).unwrap();

	assert_eq!(std::fs::read(&output).unwrap(), bink(0x1400));
	assert_eq!(partial_files(&scratch), Vec::<String>::new());

	let image = std::fs::read(scratch.image_path()).unwrap();
	let error = scratch.run(&["recover", "/intro.bik", &scratch.image_path().to_string_lossy()]).unwrap_err();
	assert!(matches!(error, FatxToolError::Output(OutputError::OutputAliasesInput { .. })));
	assert_eq!(std::fs::read(scratch.image_path()).unwrap(), image);
}

/// Names of temporary recovery files left in the scratch directory.
fn partial_files(scratch: &Scratch) -> Vec<String> {
	std::fs::read_dir(scratch.path(""))
		.unwrap()
		.map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
		.filter(|name| name.ends_with(".partial"))
		.collect()
}
