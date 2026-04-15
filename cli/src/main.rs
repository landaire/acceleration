use std::collections::HashMap;
use std::collections::VecDeque;
use std::fs::File;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::DateTime;
use chrono::Utc;
use clap::Parser;
use clap::Subcommand;
use humansize::DECIMAL;
use memmap2::MmapOptions;
use serde_json::json;
use vfs::VfsPath;
use xcontent::xecrypt::XContentKeyMaterial;

#[derive(Debug, Subcommand)]
enum Commands {
	Info {
		/// Show additional header information
		#[arg(short, long)]
		long: bool,
		#[arg(long)]
		json: bool,
		#[arg(long)]
		json_pretty: bool,
	},
	/// Lists files
	List {
		/// Present an ASCII tree view of the files
		#[arg(short, long)]
		tree: bool,
		/// Show extra information about the files
		#[arg(short, long)]
		long: bool,
		/// Recurse into child directories
		#[arg(short, long)]
		recursive: bool,
		/// Path to print information about
		path: Option<String>,
	},
	Extract {
		/// File path to extract from the STFS package
		file_name: String,
		/// Where to write the output file
		#[arg(default_value = ".")]
		output_path: PathBuf,
	},
}

/// Xbox 360 STFS package tool
#[derive(Parser, Debug)]
#[command(name = "acceleration", version, about, long_about = None)]
struct Args {
	#[structopt(name = "FILE")]
	file_name: PathBuf,

	#[command(subcommand)]
	command: Option<Commands>,
}

const GENERAL_PADDING: usize = 25;

fn main() -> anyhow::Result<()> {
	let args = Args::parse();
	let file = File::open(args.file_name)?;
	let mmap = unsafe { MmapOptions::new().map(&file)? };

	let package = xcontent::XContentPackage::try_from(&mmap[..])?;

	if let Commands::Info { long: _, json, json_pretty } =
		args.command.as_ref().unwrap_or(&Commands::Info { long: false, json: false, json_pretty: false })
	{
		let header = &package.header;
		let header_hash = header.header_hash(&mmap[..]);
		let header_hash_hex = hex::encode(header_hash);
		let metadata = &header.metadata;
		let signature_verification = package.verify_signature(&mmap[..]);
		let storage_path = package.storage_path();

		if *json || *json_pretty {
			let value = json!({
				"calculated": {
					"header_hash": header_hash_hex,
				},
				"header": serde_json::value::to_value(header).expect("failed to convert XContentHeader to JSON Value"),
				"metadata": serde_json::value::to_value(metadata).expect("failed to convert XContentMetadata to JSON Value"),
			});

			if *json_pretty {
				serde_json::to_writer_pretty(std::io::stdout(), &value).expect("failed to write JSON output");
			} else {
				serde_json::to_writer(std::io::stdout(), &value).expect("failed to write JSON output");
			}
			return Ok(());
		}

		println!("=== Calculated ===");
		println!("{: <padding$} {}", "Expected Signed Hash:", header_hash_hex, padding = GENERAL_PADDING);
		println!(
			"{: <padding$} {}",
			"Hash Valid:",
			if let Ok(console_kind) = signature_verification {
				format!("✅ ({:?})", console_kind)
			} else {
				"❌".to_string()
			},
			padding = GENERAL_PADDING,
		);
		println!("{: <padding$} {}", "Storage Path:", storage_path, padding = GENERAL_PADDING,);

		let content_id = hex::encode(header.content_id);
		let signature = match &header.key_material {
			XContentKeyMaterial::Certificate(_cert) => {
				hex::encode([0x0])
				//todo!("certificate");
			}
			XContentKeyMaterial::Signature(sig, _) => hex::encode(xcontent::xecrypt::raw_signature_to_standard(sig)),
		};
		println!();
		println!("=== XContentHeader ===");
		println!("{: <padding$} {}", "Signature Type:", header.signature_type, padding = GENERAL_PADDING);
		println!("{: <padding$} {}", "Signature:", signature, padding = GENERAL_PADDING);
		println!("{: <padding$} {}", "Content ID (Header Hash):", content_id, padding = GENERAL_PADDING);

		println!();
		println!("=== XContentMetadata ===");
		println!(
			"{: <padding$} {:?} (0x{:08X})",
			"Content Type:",
			metadata.content_type,
			metadata.content_type as u32,
			padding = GENERAL_PADDING
		);
		println!(
			"{: <padding$} {} ({:X} bytes)",
			"Content Size:",
			humansize::format_size(metadata.content_size, DECIMAL),
			metadata.content_size,
			padding = GENERAL_PADDING
		);
		println!("{: <padding$} 0x{:08X}", "Media ID:", metadata.media_id, padding = GENERAL_PADDING);
		println!("{: <padding$} {}", "Metadata Version:", metadata.metadata_version, padding = GENERAL_PADDING);
		println!("{: <padding$} {}", "Version:", metadata.version, padding = GENERAL_PADDING);
		println!("{: <padding$} {}", "Base Version:", metadata.base_version, padding = GENERAL_PADDING);
		println!("{: <padding$} 0x{:08X}", "Title ID:", metadata.title_id, padding = GENERAL_PADDING);
		println!("{: <padding$} {}", "Platform:", metadata.platform, padding = GENERAL_PADDING);
		println!("{: <padding$} {}", "Executable Type:", metadata.executable_type, padding = GENERAL_PADDING);
		println!("{: <padding$} {}", "Disc Number:", metadata.disc_number, padding = GENERAL_PADDING);
		println!("{: <padding$} {}", "Disc in Set:", metadata.disc_in_set, padding = GENERAL_PADDING);
		println!("{: <padding$} 0x{:08X}", "Savegame ID:", metadata.savegame_id, padding = GENERAL_PADDING);
		println!("{: <padding$} {}", "Console ID:", metadata.console_id, padding = GENERAL_PADDING);
		println!("{: <padding$} {:016X}", "Creator XUID:", metadata.creator_xuid, padding = GENERAL_PADDING);

		println!();
		println!("{: <padding$} {}", "Title Name:", metadata.title_name, padding = GENERAL_PADDING);
		println!("{: <padding$} {}", "Publisher Name:", metadata.publisher_name, padding = GENERAL_PADDING);
		for (lang_id, display_name) in metadata.display_name.iter().enumerate() {
			if display_name.is_empty() {
				continue;
			}
			println!(
				"{: <padding$} {}",
				format!("Display Name ({}):", lang_id),
				display_name.deref(),
				padding = GENERAL_PADDING
			)
		}

		println!();
		for (lang_id, description) in metadata.display_description.iter().enumerate() {
			if description.is_empty() {
				continue;
			}
			println!(
				"{: <padding$} {}",
				format!("Description ({}):", lang_id),
				description.deref(),
				padding = GENERAL_PADDING
			)
		}

		// pub content_type: ContentType,
		// pub metadata_version: u32,
		// pub content_size: u64,
		// pub media_id: u32,
		// pub version: u32,
		// pub base_version: u32,
		// pub title_id: u32,
		// pub platform: u8,
		// pub executable_type: u8,
		// pub disc_number: u8,
		// pub disc_in_set: u8,
		// pub savegame_id: u32,
		// pub console_id: [u8; 5],
		// pub profile_id: u64,

		// #[brw(seek_before = std::io::SeekFrom::Start(0x3a9))]
		// pub volume_kind: FileSystemKind,

		// #[brw(seek_before = std::io::SeekFrom::Start(0x379))]
		// #[br(args(volume_kind))]
		// pub volume_descriptor: FileSystem,

		// // Start metadata v1
		// pub data_file_count: u32,
		// pub data_file_combined_size: u64,

		// // TODO: parse the inbetween data
		// #[brw(seek_before = std::io::SeekFrom::Start(0x3fd))]
		// pub device_id: [u8; 0x14],

		// // TODO: support localized names
		// pub display_name: [FixedLengthNullWideString; 12],

		// #[brw(seek_before = std::io::SeekFrom::Start(0xd11))]
		// pub display_description: [FixedLengthNullWideString; 12],

		// #[serde(serialize_with = "serialize_null_wide_string")]
		// #[brw(seek_before = std::io::SeekFrom::Start(0x1611))]
		// #[br(dbg)]
		// pub publisher_name: NullWideString,

		// #[serde(serialize_with = "serialize_null_wide_string")]
		// #[brw(seek_before = std::io::SeekFrom::Start(0x1691))]
		// #[br(dbg)]
		// pub title_name: NullWideString,

		// #[brw(seek_before = std::io::SeekFrom::Start(0x1711))]
		// pub transfer_flags: u8,
		// #[br(dbg)]
		// pub thumbnail_image_size: u32,
		// #[br(dbg)]
		// pub title_thumbnail_image_size: u32,

		// #[br(count = thumbnail_image_size)]
		// #[brw(pad_size_to(MAX_IMAGE_SIZE))]
		// pub thumbnail_image: Vec<u8>,

		// #[br(count = title_thumbnail_image_size)]
		// #[brw(pad_size_to(MAX_IMAGE_SIZE))]
		// pub title_image: Vec<u8>,

		// #[br(if(((header_size + 0xFFF) & 0xFFFFF000) - 0x971A > 0x15F4))]
		// #[br(dbg)]
		// pub installer_type: Option<InstallerType>,
		return Ok(());
	}

	let mut path: VfsPath = package.to_vfs_path(Arc::new(mmap));

	match args.command.expect("default command should have been handled") {
		Commands::Info { long: _, json: _, json_pretty: _ } => {
			unreachable!("Handled above")
		}
		Commands::List { tree: true, long: _, recursive: _, path: start_path } => {
			let mut tree = HashMap::new();
			if start_path.is_none() {
				// need to ensure root directory is represented
				tree.insert("".to_string(), vec![]);
			}

			if let Some(start_path) = &start_path {
				path = path.join(start_path)?;
			}

			for path in path.walk_dir()? {
				let path = path?;
				let children = tree.entry(path.parent().as_str().to_string()).or_default();
				children.push(path);
			}

			let mut queue = VecDeque::new();
			if start_path.is_none() {
				queue.push_back((0, "", ".".to_string(), tree.remove("")));
			} else {
				let path_as_str = path.as_str().to_owned();
				let children = tree.remove(&path_as_str);

				queue.push_back((0, "", path_as_str, children));
			}
			while let Some((depth, tree_char, name, children)) = queue.pop_front() {
				let file_name = name.split('/').next_back().unwrap_or(name.as_ref());
				println!(
					"{tree_char:>width$}{space}{file_name}",
					space = if depth == 0 { "" } else { " " },
					width = depth * 3
				);
				if let Some(mut children) = children {
					children.sort_by_key(|child| child.is_file().unwrap());
					let mut first = true;
					while let Some(child) = children.pop() {
						let tree_char = if children.is_empty() {
							"└──"
						} else if first {
							first = false;
							"├──"
						} else {
							"├──"
						};

						let children = tree.remove(child.as_str());
						queue.push_back(((depth + 1), tree_char, child.filename(), children));
					}
				}
			}
		}
		Commands::List { tree: false, long: _, recursive: _, path: _start_path } => {
			for file in path.walk_dir()? {
				let file = file?;
				let meta = file.metadata()?;
				if file.as_str().chars().filter(|c| *c == '/').count() == 1 {
					let created: DateTime<Utc> = meta.created.unwrap().into();
					let accessed: DateTime<Utc> = meta.accessed.unwrap().into();

					println!(
						"{} {}b {} {} {}",
						if file.is_file()? { "f" } else { "d" },
						meta.len,
						created.format("%Y/%m/%d"),
						accessed.format("%Y/%m/%d"),
						file.filename()
					);
				}
			}
		}
		Commands::Extract { file_name, output_path } => {
			let path = path.join(&file_name)?;
			println!("{:?}", path.as_str());

			if path.is_dir()? {
				let target_path = output_path.join(path.filename());
				std::fs::create_dir_all(&target_path)?;

				// We're extracting a dir
				for file in path.walk_dir()? {
					let file = file?;
					let target_path =
						output_path.join(&file.as_str().strip_prefix(path.parent().as_str()).unwrap()[1..]);
					if file.is_dir()? {
						std::fs::create_dir_all(&target_path)?;
					} else {
						let mut out_file = File::create(&target_path)?;
						println!("writing output file: {:?}, {:?}", target_path, file.metadata()?);
						std::io::copy(&mut file.open_file()?, &mut out_file)?;
					}
				}
			} else {
				let mut out_file = if output_path.is_dir() {
					File::create(output_path.join(path.filename()))?
				} else {
					File::create(output_path)?
				};

				std::io::copy(&mut path.open_file()?, &mut out_file)?;
			}
		}
	}

	// for path in path.walk_dir()? {
	// 	let path = path?;
	// 	println!("name={:?}, meta={:#?}", path.as_str(), path.metadata());
	// }

	// let mut out_file = File::create("default.xex")?;
	// std::io::copy(&mut path.join("default.xex")?.open_file()?, &mut out_file)?;

	// let metadata = xcontent_package.metadata("default.xex")?;
	// println!("{:#X?}", metadata);
	// for file in xcontent_package.read_dir("/maps")? {
	// 	println!("{}", file);
	// }
	// panic!("{:#X?}", xcontent_package);
	Ok(())
}
