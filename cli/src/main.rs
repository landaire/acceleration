use std::fs::File;
use std::path::PathBuf;

use bytes::Buf;
use bytes::Bytes;
use memmap::MmapOptions;
use stfs::fs::StFS;
use stfs::vfs::FileSystem;
use stfs::vfs::VfsPath;
use stfs::StfsPackage;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = "acceleration-cli", about = "Xbox 360 STFS package tool")]
struct Opt {
	#[structopt(name = "FILE")]
	file_name: PathBuf,
}

fn main() -> anyhow::Result<()> {
	let config = Opt::from_args();
	let file = File::open(config.file_name)?;
	let mmap = unsafe { MmapOptions::new().map(&file)? };

	let package = StfsPackage::try_from(&mmap[..])?;
	let xcontent_package = StFS { package, data: mmap };
	let path: VfsPath = VfsPath::new(xcontent_package);
	for path in path.walk_dir()? {
		let path = path?;
		println!("name={:?}, meta={:#?}", path.as_str(), path.metadata());
	}

	// let metadata = xcontent_package.metadata("default.xex")?;
	// println!("{:#X?}", metadata);
	// for file in xcontent_package.read_dir("/maps")? {
	// 	println!("{}", file);
	// }
	// panic!("{:#X?}", xcontent_package);
	Ok(())
}
