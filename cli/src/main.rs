use std::{fs::File, path::PathBuf};

use memmap::MmapOptions;
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

    let xcontent_package = StfsPackage::try_from(&mmap[..])?;
    panic!("{:#X?}", xcontent_package);
    Ok(())
}
