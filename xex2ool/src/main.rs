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
	/// Extract resources from a XEX
	Resources {
		/// Path to the XEX file
		file: PathBuf,

		/// Output directory (defaults to current directory)
		#[arg(short, long, default_value = ".")]
		output_dir: PathBuf,
	},
	/// List import libraries and their imports
	Imports {
		/// Path to the XEX file
		file: PathBuf,
	},
	/// Generate IDA Pro IDC script
	Idc {
		/// Path to the XEX file
		file: PathBuf,

		/// Output path (defaults to <input>.idc)
		#[arg(short, long)]
		output: Option<PathBuf>,
	},
	/// Output metadata as XML
	Xml {
		/// Path to the XEX file
		file: PathBuf,
	},
	/// Remove XEX restrictions (modifies in-place unless -o given)
	Patch {
		/// Path to the XEX file
		file: PathBuf,

		/// Output path (modifies original if not given)
		#[arg(short, long)]
		output: Option<PathBuf>,

		/// Remove all limits
		#[arg(short = 'a', long)]
		remove_all: bool,

		/// Remove media limits
		#[arg(short = 'm', long)]
		remove_media: bool,

		/// Remove region limits
		#[arg(short = 'r', long)]
		remove_region: bool,

		/// Remove bounding path
		#[arg(short = 'b', long)]
		remove_bounding_path: bool,

		/// Zero the media ID
		#[arg(short = 'z', long)]
		zero_media_id: bool,
	},
}

fn main() -> anyhow::Result<()> {
	let args = Args::parse();

	match args.command {
		Commands::Info { extended, file } => cmd_info(&file, extended),
		Commands::Basefile { file, output } => cmd_basefile(&file, output),
		Commands::Resources { file, output_dir } => cmd_resources(&file, &output_dir),
		Commands::Imports { file } => cmd_imports(&file),
		Commands::Idc { file, output } => cmd_idc(&file, output),
		Commands::Xml { file } => cmd_xml(&file),
		Commands::Patch {
			file,
			output,
			remove_all,
			remove_media,
			remove_region,
			remove_bounding_path,
			zero_media_id,
		} => cmd_patch(&file, output, remove_all, remove_media, remove_region, remove_bounding_path, zero_media_id),
	}
}

fn cmd_info(path: &PathBuf, extended: bool) -> anyhow::Result<()> {
	let data = fs::read(path)?;
	let xex = Xex2::parse(data)?;

	let header = &xex.header;
	let security = &xex.security_info;

	println!("Module Flags:      {:#010x}", header.module_flags.0);
	if header.module_flags.is_title() {
		println!("                   Title");
	}
	if header.module_flags.is_dll() {
		println!("                   DLL");
	}
	if header.module_flags.is_patch() {
		println!("                   Patch");
	}

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

	if let Some(heap) = header.default_heap_size() {
		println!("Default Heap:      {:#010x}", heap);
	}

	if let Some(exec) = header.execution_info() {
		println!("Title ID:          {:08X}", exec.title_id);
		println!("Media ID:          {:08X}", exec.media_id);
		println!("Version:           {:#010x}", exec.version);
		println!("Base Version:      {:#010x}", exec.base_version);
		println!("Disc:              {}/{}", exec.disc_number, exec.disc_count);
		println!("Savegame ID:       {:08X}", exec.savegame_id);
	}

	let file_format = header.file_format_info(xex.raw())?;
	println!("Encryption:        {:?}", file_format.encryption_type);
	println!("Compression:       {:?}", file_format.compression_type);
	if let Some(ws) = file_format.window_size {
		println!("LZX Window:        {:#x}", ws);
	}

	println!("Game Regions:      {:#010x}", security.image_info.game_regions);
	println!("Allowed Media:     {:#010x}", security.image_info.allowed_media_types);

	if let Some(ratings) = header.game_ratings() {
		println!("ESRB:              {}", ratings.esrb);
		println!("PEGI:              {}", ratings.pegi);
		println!("CERO:              {}", ratings.cero);
	}

	if let Some(path) = header.bounding_path() {
		println!("Bounding Path:     {}", path);
	}

	if let Some(tls) = header.tls_info() {
		println!("TLS Slots:         {}", tls.slot_count);
		println!("TLS Data Addr:     {:#010x}", tls.raw_data_address);
		println!("TLS Data Size:     {:#x}", tls.data_size);
	}

	if let Some(res) = header.resource_info() {
		println!("Resources:         {}", res.resources.len());
		for r in &res.resources {
			println!("  {:8} addr={:#010x} size={:#x}", r.name, r.address, r.size);
		}
	}

	if let Some(table) = header.import_table() {
		println!("Import Libraries:  {}", table.libraries.len());
		for lib in &table.libraries {
			println!("  {} v{:#010x} ({} imports)", lib.name, lib.version, lib.records.len());
		}
	}

	if extended {
		println!();
		println!("Optional Headers ({}):", header.optional_header_count);
		for (key, value) in &header.optional_headers {
			let name = optional_header_name(*key);
			match value {
				xex2::header::OptionalHeaderValue::Inline(v) => {
					println!("  {:#010x} {:24} = {:#010x}", key, name, v);
				}
				xex2::header::OptionalHeaderValue::Data(data) => {
					println!("  {:#010x} {:24} = [{} bytes]", key, name, data.len());
				}
			}
		}

		println!();
		println!("Security Info:");
		println!("  Header Size:     {:#010x}", security.header_size);
		println!("  Image Flags:     {:#010x}", security.image_info.image_flags);
		println!("  Page Desc Count: {}", security.page_descriptor_count);
		println!("  File Key:        {}", hex_str(&security.image_info.file_key));
		println!("  Image Hash:      {}", hex_str(&security.image_info.image_hash));
		println!("  Header Hash:     {}", hex_str(&security.image_info.header_hash));
		println!("  Import Count:    {}", security.image_info.import_table_count);

		if let Some(key) = header.lan_key() {
			println!("  LAN Key:         {}", hex_str(&key));
		}

		if let Some(device) = header.device_id() {
			println!("  Device ID:       {}", hex_str(&device));
		}
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

fn cmd_resources(path: &PathBuf, output_dir: &PathBuf) -> anyhow::Result<()> {
	let data = fs::read(path)?;
	let xex = Xex2::parse(data)?;

	let resources = match xex.header.resource_info() {
		Some(r) => r,
		None => {
			println!("No resources found");
			return Ok(());
		}
	};

	let basefile = xex.extract_basefile()?;
	let base_addr = xex.security_info.image_info.load_address;

	fs::create_dir_all(output_dir)?;

	for res in &resources.resources {
		let offset = (res.address - base_addr) as usize;
		if offset + res.size as usize > basefile.len() {
			eprintln!("Resource {} extends past basefile (addr={:#x} size={:#x})", res.name, res.address, res.size);
			continue;
		}

		let res_data = &basefile[offset..offset + res.size as usize];
		let out_path = output_dir.join(&res.name);
		fs::write(&out_path, res_data)?;
		println!("Extracted {} ({} bytes)", res.name, res.size);
	}

	Ok(())
}

fn cmd_imports(path: &PathBuf) -> anyhow::Result<()> {
	let data = fs::read(path)?;
	let xex = Xex2::parse(data)?;

	let table = match xex.header.import_table() {
		Some(t) => t,
		None => {
			println!("No import table found");
			return Ok(());
		}
	};

	for lib in &table.libraries {
		println!("{} (v{:#010x}, min v{:#010x})", lib.name, lib.version, lib.version_min);
		println!("  {} imports:", lib.records.len());

		for (i, &record) in lib.records.iter().enumerate() {
			if i % 2 == 0 {
				print!("    [{:3}] IAT={:#010x}", i / 2, record);
			} else {
				println!("  thunk={:#010x}", record);
			}
		}
		if lib.records.len() % 2 != 0 {
			println!();
		}
		println!();
	}

	Ok(())
}

fn cmd_patch(
	path: &PathBuf,
	output: Option<PathBuf>,
	remove_all: bool,
	remove_media: bool,
	remove_region: bool,
	remove_bounding_path: bool,
	zero_media_id: bool,
) -> anyhow::Result<()> {
	let data = fs::read(path)?;
	let xex = Xex2::parse(data)?;

	let mut limits = xex2::writer::RemoveLimits::default();
	if remove_all {
		limits = xex2::writer::RemoveLimits::all();
	} else {
		limits.media = remove_media;
		limits.region = remove_region;
		limits.bounding_path = remove_bounding_path;
		limits.zero_media_id = zero_media_id;
	}

	let patched = xex2::writer::modify_xex(
		&xex,
		xex2::writer::TargetEncryption::Unchanged,
		xex2::writer::TargetCompression::Unchanged,
		xex2::writer::TargetMachine::Unchanged,
		&limits,
	)?;

	let out_path = output.unwrap_or_else(|| path.clone());
	fs::write(&out_path, &patched)?;
	println!("Wrote patched XEX to {}", out_path.display());

	Ok(())
}

fn cmd_idc(path: &PathBuf, output: Option<PathBuf>) -> anyhow::Result<()> {
	let data = fs::read(path)?;
	let xex = Xex2::parse(data)?;

	let idc =
		xex2::idc::generate_idc(&xex.header, xex.security_info.image_info.load_address, xex.security_info.image_size);

	let out_path = output.unwrap_or_else(|| {
		let mut p = path.clone();
		p.set_extension("idc");
		p
	});

	fs::write(&out_path, &idc)?;
	println!("Wrote IDC to {}", out_path.display());

	Ok(())
}

fn cmd_xml(path: &PathBuf) -> anyhow::Result<()> {
	let data = fs::read(path)?;
	let xex = Xex2::parse(data)?;
	print!("{}", xex2::xml::generate_xml(&xex));
	Ok(())
}

fn optional_header_name(key: u32) -> &'static str {
	use xex2::header::optional_header_keys::*;
	match key {
		RESOURCE_INFO => "ResourceInfo",
		FILE_FORMAT_INFO => "FileFormatInfo",
		BASE_REFERENCE => "BaseReference",
		DELTA_PATCH_DESCRIPTOR => "DeltaPatchDescriptor",
		BOUNDING_PATH => "BoundingPath",
		DEVICE_ID => "DeviceId",
		ORIGINAL_BASE_ADDRESS => "OriginalBaseAddress",
		ENTRY_POINT => "EntryPoint",
		TLS_INFO => "TlsInfo",
		DEFAULT_STACK_SIZE => "DefaultStackSize",
		DEFAULT_FS_CACHE_SIZE => "DefaultFsCacheSize",
		DEFAULT_HEAP_SIZE => "DefaultHeapSize",
		PAGE_HEAP_SIZE_AND_FLAGS => "PageHeapSizeAndFlags",
		IMPORT_LIBRARIES => "ImportLibraries",
		EXECUTION_INFO => "ExecutionInfo",
		SERVICE_ID_LIST => "ServiceIdList",
		TITLE_WORKSPACE_SIZE => "TitleWorkspaceSize",
		GAME_RATINGS => "GameRatings",
		LAN_KEY => "LanKey",
		XBOX_360_LOGO => "Xbox360Logo",
		MULTIDISC_MEDIA_IDS => "MultidiscMediaIds",
		ALTERNATE_TITLE_IDS => "AlternateTitleIds",
		ADDITIONAL_TITLE_MEMORY => "AdditionalTitleMemory",
		EXPORTS_BY_NAME => "ExportsByName",
		_ => "",
	}
}

fn hex_str(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
