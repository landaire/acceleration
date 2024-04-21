use std::collections::HashMap;
use std::collections::VecDeque;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Buf;
use bytes::Bytes;
use clap::Parser;
use clap::Subcommand;
use memmap::MmapOptions;
use stfs::fs::StFS;
use stfs::vfs::FileSystem;
use stfs::vfs::VfsPath;
use stfs::StfsPackage;

#[derive(Debug, Subcommand)]
enum Commands {
	/// Lists files
	List {
		/// Present an ASCII tree view of the files
		#[arg(short, long)]
		tree: bool,
		/// Show extra information about the files
		#[arg(short, long)]
		long: bool,
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
	command: Commands,
}

fn main() -> anyhow::Result<()> {
	let args = Args::parse();
	let file = File::open(args.file_name)?;
	let mmap = unsafe { MmapOptions::new().map(&file)? };

	let package = StfsPackage::try_from(&mmap[..])?;
	let xcontent_package = StFS { package, data: Arc::new(mmap) };
	let path: VfsPath = VfsPath::new(xcontent_package);

	match args.command {
		Commands::List { tree: true, long: _ } => {
			let mut tree = HashMap::new();
			tree.insert("".to_string(), vec![]);
			for path in path.walk_dir()? {
				let path = path?;
				let children = tree.entry(path.parent().as_str().to_string()).or_default();
				children.push(path);
			}

			let mut queue = VecDeque::new();
			queue.push_back((0, "", ".".to_string(), tree.remove("")));
			while let Some((depth, tree_char, name, children)) = queue.pop_front() {
				let file_name = name.split('/').last().unwrap_or(name.as_ref());
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
						} else if depth == 0 {
							""
						} else {
							"├──"
						};

						let children = tree.remove(child.as_str());
						queue.push_back(((depth + 1), tree_char, child.filename(), children));
					}
				}
			}
		}
		Commands::List { tree: false, long } => {
			todo!();
		}
		Commands::Extract { file_name, output_path } => {
			let path = path.join(&file_name)?;
			println!("{:?}", path.as_str());

			if path.is_dir()? {
				let target_path = output_path.join(&path.filename());
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
