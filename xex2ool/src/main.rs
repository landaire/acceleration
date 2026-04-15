use std::fs;
use std::path::PathBuf;

use clap::Parser;
use clap::Subcommand;
use xex2::Xex2;

#[derive(Parser)]
#[command(name = "xex2ool", version, about = "Xbox 360 XEX2 tool")]
struct Args {
	#[command(subcommand)]
	command: Commands,
}

#[derive(Subcommand)]
enum Commands {
	/// Print info about a XEX file
	Info {
		/// Print extended info
		#[arg(short, long)]
		extended: bool,

		/// Path to the XEX file
		file: PathBuf,
	},
	/// Extract the basefile (PE image) from a XEX
	Basefile {
		/// Path to the XEX file
		file: PathBuf,

		/// Output path (defaults to <input>.pe)
		#[arg(short, long)]
		output: Option<PathBuf>,
	},
}

fn main() -> anyhow::Result<()> {
	let args = Args::parse();

	match args.command {
		Commands::Info { extended, file } => cmd_info(&file, extended),
		Commands::Basefile { file, output } => cmd_basefile(&file, output),
	}
}

fn cmd_info(path: &PathBuf, extended: bool) -> anyhow::Result<()> {
	let data = fs::read(path)?;
	let xex = Xex2::parse(data)?;

	let header = &xex.header;
	let security = &xex.security_info;

	println!("Module Flags:      {:#010x}", header.module_flags.0);
	println!("Data Offset:       {:#010x}", header.data_offset);
	println!("Load Address:      {:#010x}", security.image_info.load_address);
	println!("Image Size:        {:#010x}", security.image_size);

	if let Some(entry) = header.entry_point() {
		println!("Entry Point:       {:#010x}", entry);
	}

	if let Some(base) = header.original_base_address() {
		println!("Original Base:     {:#010x}", base);
	}

	if let Some(stack) = header.default_stack_size() {
		println!("Default Stack:     {:#010x}", stack);
	}

	if let Some(exec) = header.execution_info() {
		println!("Title ID:          {:08X}", exec.title_id);
		println!("Media ID:          {:08X}", exec.media_id);
		println!("Version:           {:#010x}", exec.version);
		println!("Base Version:      {:#010x}", exec.base_version);
		println!("Disc:              {}/{}", exec.disc_number, exec.disc_count);
	}

	let file_format = header.file_format_info(xex.raw())?;
	println!("Encryption:        {:?}", file_format.encryption_type);
	println!("Compression:       {:?}", file_format.compression_type);

	if extended {
		println!("\nOptional Headers ({}):", header.optional_header_count);
		for (key, value) in &header.optional_headers {
			match value {
				xex2::header::OptionalHeaderValue::Inline(v) => {
					println!("  {:#010x} = {:#010x}", key, v);
				}
				xex2::header::OptionalHeaderValue::Data(data) => {
					println!("  {:#010x} = [{} bytes]", key, data.len());
				}
			}
		}

		println!("\nSecurity Info:");
		println!("  Header Size:     {:#010x}", security.header_size);
		println!("  Allowed Media:   {:#010x}", security.image_info.allowed_media_types);
		println!("  Game Regions:    {:#010x}", security.image_info.game_regions);
		println!("  File Key:        {}", hex_str(&security.image_info.file_key));
		println!("  Image Hash:      {}", hex_str(&security.image_info.image_hash));
	}

	Ok(())
}

fn cmd_basefile(path: &PathBuf, output: Option<PathBuf>) -> anyhow::Result<()> {
	let data = fs::read(path)?;
	let xex = Xex2::parse(data)?;

	let basefile = xex.extract_basefile()?;

	let out_path = output.unwrap_or_else(|| {
		let mut p = path.clone();
		p.set_extension("pe");
		p
	});

	fs::write(&out_path, &basefile)?;
	println!("Wrote {} bytes to {}", basefile.len(), out_path.display());

	Ok(())
}

fn hex_str(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
