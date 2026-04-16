use std::fs;
use std::path::PathBuf;

use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;
use tabled::Table;
use tabled::Tabled;
use tabled::builder::Builder;
use tabled::settings::Style;
use xex2::Xex2;

mod idc;
mod kernel_exports;
mod xml;

#[derive(Clone, Copy, ValueEnum, Default)]
enum OutputFormat {
	#[default]
	Text,
	Json,
}

#[derive(Parser)]
#[command(name = "xex2ool", version, about = "Xbox 360 XEX2 tool")]
struct Args {
	/// Output format
	#[arg(short, long, global = true, default_value = "text")]
	format: OutputFormat,

	#[command(subcommand)]
	command: Commands,
}

#[derive(clap::Args, Default)]
struct LimitArgs {
	/// Remove all restrictions
	#[arg(short = 'a', long = "all")]
	all: bool,

	/// Remove media type restrictions
	#[arg(short = 'm', long)]
	media: bool,

	/// Remove region restrictions
	#[arg(short = 'r', long)]
	region: bool,

	/// Remove bounding path restriction
	#[arg(short = 'b', long)]
	bounding_path: bool,

	/// Remove device ID restriction
	#[arg(short = 'd', long)]
	device_id: bool,

	/// Remove console ID restriction
	#[arg(short = 'i', long)]
	console_id: bool,

	/// Remove date restrictions
	#[arg(short = 'y', long)]
	dates: bool,

	/// Remove keyvault privilege restrictions
	#[arg(short = 'v', long)]
	kv_privileges: bool,

	/// Remove signed-keyvault-only restriction
	#[arg(short = 'k', long)]
	signed_kv_only: bool,

	/// Remove minimum library version restrictions
	#[arg(short = 'l', long)]
	lib_versions: bool,

	/// Zero the media ID
	#[arg(short = 'z', long)]
	zero_media_id: bool,
}

impl From<&LimitArgs> for xex2::writer::RemoveLimits {
	fn from(args: &LimitArgs) -> Self {
		if args.all {
			return xex2::writer::RemoveLimits::all();
		}
		xex2::writer::RemoveLimits {
			media: args.media,
			region: args.region,
			bounding_path: args.bounding_path,
			device_id: args.device_id,
			console_id: args.console_id,
			dates: args.dates,
			keyvault_privileges: args.kv_privileges,
			signed_keyvault_only: args.signed_kv_only,
			library_versions: args.lib_versions,
			zero_media_id: args.zero_media_id,
		}
	}
}

#[derive(Subcommand)]
enum Commands {
	/// Print info about a XEX file
	Info {
		/// Print extended info (security, optional headers)
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
	/// Modify a XEX: remove restrictions, convert format, apply patches
	Patch {
		/// Path to the XEX file
		file: PathBuf,

		/// Output path (defaults to in-place modification)
		#[arg(short, long)]
		output: Option<PathBuf>,

		/// Apply a delta patch (.xexp) file
		#[arg(long)]
		apply_patch: Option<PathBuf>,

		/// Merge patch into standalone XEX
		#[arg(long, requires = "apply_patch")]
		merge_patch: bool,

		/// Convert to devkit format
		#[arg(long, conflicts_with = "retail")]
		devkit: bool,

		/// Convert to retail format
		#[arg(long, conflicts_with = "devkit")]
		retail: bool,

		/// Encrypt the output
		#[arg(long, conflicts_with = "decrypt")]
		encrypt: bool,

		/// Decrypt the output
		#[arg(long, conflicts_with = "encrypt")]
		decrypt: bool,

		/// Compress the output
		#[arg(long, conflicts_with_all = ["decompress", "basic_compress"])]
		compress: bool,

		/// Decompress the output (remove all compression)
		#[arg(long, conflicts_with_all = ["compress", "basic_compress"])]
		decompress: bool,

		/// Use basic compression (zero-padded blocks)
		#[arg(long, conflicts_with_all = ["compress", "decompress"])]
		basic_compress: bool,

		/// Remove restrictions
		#[command(flatten)]
		limits: LimitArgs,
	},
}

fn main() -> anyhow::Result<()> {
	let args = Args::parse();
	let fmt = args.format;

	match args.command {
		Commands::Info { extended, file } => cmd_info(&file, extended, fmt),
		Commands::Basefile { file, output } => cmd_basefile(&file, output),
		Commands::Resources { file, output_dir } => cmd_resources(&file, &output_dir),
		Commands::Imports { file } => cmd_imports(&file, fmt),
		Commands::Idc { file, output } => cmd_idc(&file, output),
		Commands::Xml { file } => cmd_xml(&file),
		Commands::Patch {
			file,
			output,
			apply_patch,
			merge_patch,
			devkit,
			retail,
			encrypt,
			decrypt,
			compress,
			decompress,
			basic_compress,
			limits,
		} => {
			let machine = match (devkit, retail) {
				(true, _) => xex2::writer::TargetMachine::Devkit,
				(_, true) => xex2::writer::TargetMachine::Retail,
				_ => xex2::writer::TargetMachine::Unchanged,
			};
			let encryption = match (encrypt, decrypt) {
				(true, _) => xex2::writer::TargetEncryption::Encrypted,
				(_, true) => xex2::writer::TargetEncryption::Decrypted,
				_ => xex2::writer::TargetEncryption::Unchanged,
			};
			let compression = match (compress, decompress, basic_compress) {
				(true, _, _) => xex2::writer::TargetCompression::Normal,
				(_, true, _) => xex2::writer::TargetCompression::Uncompressed,
				(_, _, true) => xex2::writer::TargetCompression::Basic,
				_ => xex2::writer::TargetCompression::Unchanged,
			};
			cmd_patch(&file, output, apply_patch, merge_patch, machine, compression, encryption, &limits)
		}
	}
}

fn cmd_info(path: &PathBuf, extended: bool, fmt: OutputFormat) -> anyhow::Result<()> {
	let data = fs::read(path)?;
	let xex = Xex2::parse(&data)?;

	if matches!(fmt, OutputFormat::Json) {
		return cmd_info_json(&xex, extended);
	}

	let header = &xex.header;
	let security = &xex.security_info;
	let file_format = header.file_format_info()?;

	let mut b = Builder::default();

	use xex2::opt::AllowedMediaTypes;
	use xex2::opt::ModuleFlags;

	b.push_record([
		"Module Flags",
		&format_flags(
			header.module_flags,
			&[
				(ModuleFlags::TITLE, "Title"),
				(ModuleFlags::EXPORTS_TO_TITLE, "ExportsToTitle"),
				(ModuleFlags::SYSTEM_DEBUGGER, "SystemDebugger"),
				(ModuleFlags::DLL, "DLL"),
				(ModuleFlags::PATCH, "Patch"),
				(ModuleFlags::PATCH_DELTA, "PatchDelta"),
				(ModuleFlags::PATCH_FULL, "PatchFull"),
				(ModuleFlags::BOUND_PATH, "BoundPath"),
				(ModuleFlags::DEVICE_ID, "DeviceID"),
			],
		),
	]);

	b.push_record(["Data Offset", &format!("{:#010x}", header.data_offset)]);
	b.push_record(["Load Address", &format!("{:#010x}", security.image_info.load_address)]);
	b.push_record(["Image Size", &format!("{:#010x}", security.image_size)]);

	if let Some(entry) = header.entry_point() {
		b.push_record(["Entry Point", &format!("{:#010x}", entry)]);
	}
	if let Some(base) = header.original_base_address() {
		b.push_record(["Original Base", &format!("{:#010x}", base)]);
	}
	if let Some(stack) = header.default_stack_size() {
		b.push_record(["Default Stack", &format!("{:#010x}", stack)]);
	}
	if let Some(heap) = header.default_heap_size() {
		b.push_record(["Default Heap", &format!("{:#010x}", heap)]);
	}

	if let Some(exec) = header.execution_info() {
		b.push_record(["Title ID", &format!("{}", exec.title_id)]);
		b.push_record(["Media ID", &format!("{}", exec.media_id)]);
		b.push_record(["Version", &format!("{:#010x}", exec.version)]);
		b.push_record(["Base Version", &format!("{:#010x}", exec.base_version)]);
		b.push_record(["Disc", &format!("{}/{}", exec.disc_number, exec.disc_count)]);
		b.push_record(["Savegame ID", &format!("{:08X}", exec.savegame_id)]);
	}

	b.push_record(["Encryption", &format!("{:?}", file_format.encryption_type)]);
	b.push_record(["Compression", &format!("{:?}", file_format.compression_type)]);
	if let Some(ws) = file_format.window_size {
		b.push_record(["LZX Window", &format!("{:#x}", ws)]);
	}

	if let Some(dr) = header.date_range() {
		let fmt_ft = |ft: u64| -> String {
			xenon_types::filetime_to_timestamp(ft)
				.map(|ts| ts.strftime("%Y-%m-%d %H:%M:%S UTC").to_string())
				.unwrap_or_else(|| format!("0x{:016x}", ft))
		};
		let before = dr.not_before.map(&fmt_ft).unwrap_or_else(|| "none".into());
		let after = dr.not_after.map(&fmt_ft).unwrap_or_else(|| "none".into());
		b.push_record(["Date Range", &format!("{} -- {}", before, after)]);
	}

	b.push_record(["Game Regions", &format!("{:#010x}", security.image_info.game_regions)]);
	b.push_record([
		"Allowed Media",
		&format_flags(
			security.image_info.allowed_media_types,
			&[
				(AllowedMediaTypes::HARD_DISK, "HDD"),
				(AllowedMediaTypes::DVD_X2, "DVDX2"),
				(AllowedMediaTypes::DVD_CD, "DVDCD"),
				(AllowedMediaTypes::DVD_5, "DVD5"),
				(AllowedMediaTypes::DVD_9, "DVD9"),
				(AllowedMediaTypes::SYSTEM_FLASH, "Flash"),
				(AllowedMediaTypes::MEMORY_UNIT, "MU"),
				(AllowedMediaTypes::USB_MASS_STORAGE, "USB"),
				(AllowedMediaTypes::NETWORK, "Network"),
				(AllowedMediaTypes::DIRECT_FROM_MEMORY, "Memory"),
				(AllowedMediaTypes::RAM_DRIVE, "RAMDrive"),
				(AllowedMediaTypes::SVOD, "SVOD"),
				(AllowedMediaTypes::INSECURE_PACKAGE, "InsecurePkg"),
				(AllowedMediaTypes::SAVEGAME_PACKAGE, "SavegamePkg"),
				(AllowedMediaTypes::LOCALLY_SIGNED_PACKAGE, "LocalSigned"),
				(AllowedMediaTypes::LIVE_SIGNED_PACKAGE, "LiveSigned"),
				(AllowedMediaTypes::XBOX_PLATFORM_PACKAGE, "XboxPlatform"),
			],
		),
	]);

	if let Some(ratings) = header.game_ratings() {
		b.push_record(["ESRB", &ratings.esrb.to_string()]);
		b.push_record(["PEGI", &ratings.pegi.to_string()]);
		b.push_record(["CERO", &ratings.cero.to_string()]);
	}

	if let Some(bp) = header.bounding_path() {
		b.push_record(["Bounding Path", &bp]);
	}

	if let Some(tls) = header.tls_info() {
		b.push_record(["TLS Slots", &tls.slot_count.to_string()]);
		b.push_record(["TLS Data Addr", &format!("{:#010x}", tls.raw_data_address)]);
		b.push_record(["TLS Data Size", &format!("{:#x}", tls.data_size)]);
	}

	if let Some(res) = header.resource_info() {
		b.push_record(["Resources", &res.resources.len().to_string()]);
	}

	if let Some(table) = header.import_table() {
		b.push_record(["Import Libraries", &table.libraries.len().to_string()]);
	}

	let mut table = b.build();
	table.with(Style::rounded());
	println!("Module Info");
	println!("{}", table);

	if let Some(res) = header.resource_info()
		&& !res.resources.is_empty()
	{
		println!("\nResources");
		#[derive(Tabled)]
		struct ResourceRow {
			name: String,
			address: String,
			size: String,
		}

		let rows: Vec<_> = res
			.resources
			.iter()
			.map(|r| ResourceRow {
				name: r.name.clone(),
				address: format!("{:#010x}", r.address),
				size: format!("{:#x}", r.size),
			})
			.collect();
		let mut t = Table::new(rows);
		t.with(Style::rounded());
		println!("{}", t);
	}

	if let Some(imports) = header.import_table() {
		println!("\nImport Libraries");
		#[derive(Tabled)]
		struct LibRow {
			library: String,
			version: String,
			imports: usize,
		}

		let rows: Vec<_> = imports
			.libraries
			.iter()
			.map(|lib| LibRow {
				library: lib.name.clone(),
				version: format!("{:#010x}", lib.version),
				imports: lib.records.len() / 2,
			})
			.collect();
		let mut t = Table::new(rows);
		t.with(Style::rounded());
		println!("{}", t);
	}

	if extended {
		println!("\nOptional Headers");
		#[derive(Tabled)]
		struct HeaderRow {
			key: String,
			name: String,
			value: String,
		}

		let rows: Vec<_> = header
			.optional_headers
			.iter()
			.map(|(key, value)| HeaderRow {
				key: format!("{:#010x}", key),
				name: optional_header_name(*key).to_string(),
				value: match value {
					xex2::header::OptionalHeaderValue::Inline(v) => format!("{:#010x}", v),
					xex2::header::OptionalHeaderValue::Data(data) if data.len() <= 16 => hex_str(data),
					xex2::header::OptionalHeaderValue::Data(data) => format!("[{} bytes]", data.len()),
				},
			})
			.collect();
		let mut t = Table::new(rows);
		t.with(Style::rounded());
		println!("{}", t);

		println!("\nSecurity Info");
		let mut sb = Builder::default();
		sb.push_record(["Header Size", &format!("{:#010x}", security.header_size)]);
		sb.push_record(["Image Flags", &format!("{:#010x}", security.image_info.image_flags)]);
		sb.push_record(["Page Desc Count", &security.page_descriptor_count.to_string()]);
		sb.push_record(["File Key", &hex_str(&security.image_info.file_key.0)]);
		sb.push_record(["Image Hash", &hex_str(&security.image_info.image_hash)]);
		sb.push_record(["Header Hash", &hex_str(&security.image_info.header_hash)]);
		sb.push_record(["Import Count", &security.image_info.import_table_count.to_string()]);

		if let Some(key) = header.lan_key() {
			sb.push_record(["LAN Key", &hex_str(&key)]);
		}
		if let Some(device) = header.device_id() {
			sb.push_record(["Device ID", &hex_str(&device)]);
		}

		let mut st = sb.build();
		st.with(Style::rounded());
		println!("{}", st);
	}

	Ok(())
}

fn cmd_info_json(xex: &Xex2, extended: bool) -> anyhow::Result<()> {
	let header = &xex.header;
	let security = &xex.security_info;
	let file_format = header.file_format_info()?;

	let mut info = serde_json::Map::new();

	info.insert("module_flags".into(), serde_json::to_value(header.module_flags.bits())?);
	info.insert("data_offset".into(), serde_json::to_value(header.data_offset)?);
	info.insert("image_size".into(), serde_json::to_value(security.image_size)?);
	info.insert("load_address".into(), serde_json::to_value(security.image_info.load_address.0)?);

	if let Some(entry) = header.entry_point() {
		info.insert("entry_point".into(), serde_json::to_value(entry)?);
	}
	if let Some(base) = header.original_base_address() {
		info.insert("original_base_address".into(), serde_json::to_value(base)?);
	}
	if let Some(stack) = header.default_stack_size() {
		info.insert("default_stack_size".into(), serde_json::to_value(stack)?);
	}
	if let Some(heap) = header.default_heap_size() {
		info.insert("default_heap_size".into(), serde_json::to_value(heap)?);
	}
	if let Some(exec) = header.execution_info() {
		info.insert("execution_info".into(), serde_json::to_value(&exec)?);
	}

	info.insert("encryption".into(), serde_json::to_value(file_format.encryption_type)?);
	info.insert("compression".into(), serde_json::to_value(file_format.compression_type)?);
	if let Some(ws) = file_format.window_size {
		info.insert("lzx_window_size".into(), serde_json::to_value(ws)?);
	}

	info.insert("game_regions".into(), serde_json::to_value(security.image_info.game_regions)?);
	info.insert("allowed_media_types".into(), serde_json::to_value(security.image_info.allowed_media_types)?);

	if let Some(ratings) = header.game_ratings() {
		info.insert("game_ratings".into(), serde_json::to_value(&ratings)?);
	}
	if let Some(path) = header.bounding_path() {
		info.insert("bounding_path".into(), serde_json::to_value(path)?);
	}
	if let Some(tls) = header.tls_info() {
		info.insert("tls_info".into(), serde_json::to_value(&tls)?);
	}
	if let Some(res) = header.resource_info() {
		info.insert("resources".into(), serde_json::to_value(&res)?);
	}
	if let Some(table) = header.import_table() {
		info.insert("import_libraries".into(), serde_json::to_value(&table)?);
	}

	if extended {
		info.insert("security_info".into(), serde_json::to_value(security)?);

		let headers: serde_json::Map<String, serde_json::Value> = header
			.optional_headers
			.iter()
			.map(|(k, v)| {
				let key_str = k.to_string();
				let val = match v {
					xex2::header::OptionalHeaderValue::Inline(v) => serde_json::json!({ "type": "inline", "value": v }),
					xex2::header::OptionalHeaderValue::Data(d) => {
						serde_json::json!({ "type": "data", "size": d.len() })
					}
				};
				(key_str, val)
			})
			.collect();
		info.insert("optional_headers".into(), serde_json::Value::Object(headers));
	}

	println!("{}", serde_json::to_string_pretty(&serde_json::Value::Object(info))?);
	Ok(())
}

fn cmd_basefile(path: &PathBuf, output: Option<PathBuf>) -> anyhow::Result<()> {
	let data = fs::read(path)?;
	let xex = Xex2::parse(&data)?;

	let basefile = xex.extract_basefile(&data)?;

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
	let xex = Xex2::parse(&data)?;

	let resources = match xex.header.resource_info() {
		Some(r) => r,
		None => {
			println!("No resources found");
			return Ok(());
		}
	};

	let basefile = xex.extract_basefile(&data)?;
	let base_addr = xex.security_info.image_info.load_address;

	fs::create_dir_all(output_dir)?;

	for res in &resources.resources {
		let offset = (res.address - base_addr.0) as usize;
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

fn cmd_imports(path: &PathBuf, fmt: OutputFormat) -> anyhow::Result<()> {
	let data = fs::read(path)?;
	let xex = Xex2::parse(&data)?;

	let table = match xex.header.import_table() {
		Some(t) => t,
		None => {
			println!("No import table found");
			return Ok(());
		}
	};

	if matches!(fmt, OutputFormat::Json) {
		println!("{}", serde_json::to_string_pretty(&table)?);
		return Ok(());
	}

	for lib in &table.libraries {
		println!("{} (v{:#010x}, min v{:#010x})", lib.name, lib.version, lib.version_min);

		#[derive(Tabled)]
		struct ImportRow {
			#[tabled(rename = "#")]
			index: usize,
			#[tabled(rename = "IAT")]
			iat: String,
			#[tabled(rename = "Thunk")]
			thunk: String,
		}

		let rows: Vec<_> = lib
			.records
			.chunks(2)
			.enumerate()
			.filter_map(|(i, chunk)| {
				if chunk.len() < 2 {
					return None;
				}
				Some(ImportRow { index: i, iat: format!("{:#010x}", chunk[0]), thunk: format!("{:#010x}", chunk[1]) })
			})
			.collect();

		let mut t = Table::new(rows);
		t.with(Style::rounded());
		println!("{}", t);
		println!();
	}

	Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_patch(
	path: &PathBuf,
	output: Option<PathBuf>,
	apply_patch: Option<PathBuf>,
	merge_patch: bool,
	machine: xex2::writer::TargetMachine,
	compression: xex2::writer::TargetCompression,
	encryption: xex2::writer::TargetEncryption,
	limits: &LimitArgs,
) -> anyhow::Result<()> {
	if apply_patch.is_some() {
		anyhow::bail!("delta patch application (XEXP) is not yet implemented");
	}
	if merge_patch {
		anyhow::bail!("patch merging is not yet implemented");
	}
	if !matches!(machine, xex2::writer::TargetMachine::Unchanged) {
		anyhow::bail!("machine format conversion is not yet implemented");
	}
	if !matches!(compression, xex2::writer::TargetCompression::Unchanged) {
		anyhow::bail!("compression format conversion is not yet implemented");
	}
	if !matches!(encryption, xex2::writer::TargetEncryption::Unchanged) {
		anyhow::bail!("encryption format conversion is not yet implemented");
	}

	let data = fs::read(path)?;
	let xex = Xex2::parse(&data)?;

	let limits = xex2::writer::RemoveLimits::from(limits);
	let out_path = output.unwrap_or_else(|| path.clone());
	let mut out_file = std::fs::File::create(&out_path)?;
	xex.rebuild(&data)
		.target_encryption(encryption)
		.target_compression(compression)
		.target_machine(machine)
		.remove_limits(limits)
		.write_to(&mut out_file)?;
	println!("Wrote patched XEX to {}", out_path.display());

	Ok(())
}

fn cmd_idc(path: &PathBuf, output: Option<PathBuf>) -> anyhow::Result<()> {
	let data = fs::read(path)?;
	let xex = Xex2::parse(&data)?;

	let idc = idc::generate_idc(
		&xex.header,
		xex.security_info.image_info.load_address.0,
		xex.security_info.image_size,
	);

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
	let xex = Xex2::parse(&data)?;
	print!("{}", xml::generate_xml(&xex));
	Ok(())
}

fn optional_header_name(key: u32) -> String {
	match xex2::header::OptionalHeaderKey::from_u32(key) {
		Some(k) => k.to_string(),
		None => String::new(),
	}
}

fn hex_str(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn format_flags<F: bitflags::Flags<Bits = u32> + Copy>(flags: F, names: &[(F, &str)]) -> String {
	let set: Vec<&str> = names.iter().filter(|(f, _)| flags.contains(*f)).map(|(_, n)| *n).collect();
	if set.is_empty() {
		format!("{:#010x}", flags.bits())
	} else {
		format!("{:#010x}\n{}", flags.bits(), set.join("\n"))
	}
}
