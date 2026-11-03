use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;
use expansion::BuildError;
use expansion::HvSection;
use expansion::KernelSection;
use expansion::PackageSpec;
use expansion::PublicKeyError;
use expansion::VerifyError;
use thiserror::Error;

#[derive(Debug, Parser)]
#[command(about = "Build, sign and verify Xbox 360 hypervisor expansion packages (HXPC)")]
struct Arguments {
	#[command(subcommand)]
	command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
	/// Build a package with Flags 0 and sign it with the devkit executable key
	Build {
		/// Package id; 0, 0x454d4120 and 0x50563033 are rejected
		#[arg(long, value_parser = parse_u32)]
		id: u32,
		/// Code copied into the kernel expansion area
		#[arg(long, value_name = "FILE")]
		kernel_code: Option<PathBuf>,
		/// Bytes of zero-filled kernel data, rounded up to 0x10
		#[arg(long, value_parser = parse_u32, default_value = "0", requires = "kernel_code")]
		kernel_data_size: u32,
		/// Code copied into the hypervisor expansion area and called
		#[arg(long, value_name = "FILE")]
		hv_code: Option<PathBuf>,
		/// Offset of the hypervisor entry point in the code; a multiple of 0x10
		#[arg(long, value_parser = parse_u32, default_value = "0", requires = "hv_code")]
		hv_entry: u32,
		/// First hypervisor build that accepts the package; 0 for none
		#[arg(long, value_parser = parse_u32, default_value = "0")]
		minimum_build: u32,
		/// First hypervisor build that rejects the package; 0 for none
		#[arg(long, value_parser = parse_u32, default_value = "0")]
		maximum_build: u32,
		#[arg(long, value_name = "FILE")]
		output: PathBuf,
	},
	/// Check a package as hypervisor build 16547 devkit does
	Verify {
		/// Big-endian XECRYPT_RSAPUB_2048, or a file holding one at --public-key-offset
		#[arg(long, value_name = "FILE")]
		public_key: PathBuf,
		/// Offset of the key in --public-key (0x105b8 in a build 16547 hypervisor image)
		#[arg(long, value_parser = parse_u64)]
		public_key_offset: Option<u64>,
		package: PathBuf,
	},
}

fn parse_u64(text: &str) -> Result<u64, String> {
	let result = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
		Some(hex) => u64::from_str_radix(hex, 16),
		None => text.parse(),
	};
	result.map_err(|error| error.to_string())
}

fn parse_u32(text: &str) -> Result<u32, String> {
	u32::try_from(parse_u64(text)?).map_err(|error| error.to_string())
}

#[derive(Debug, Error)]
enum CliError {
	#[error("cannot read {}", path.display())]
	Read { path: PathBuf, source: io::Error },
	#[error("cannot write {}", path.display())]
	Write { path: PathBuf, source: io::Error },
	#[error(transparent)]
	Build(#[from] BuildError),
	#[error(transparent)]
	PublicKey(#[from] PublicKeyError),
	#[error("verification failed: {0}")]
	Verify(#[from] VerifyError),
}

fn read(path: &PathBuf) -> Result<Vec<u8>, CliError> {
	fs::read(path).map_err(|source| CliError::Read { path: path.clone(), source })
}

fn run(arguments: Arguments) -> Result<(), CliError> {
	match arguments.command {
		Command::Build {
			id,
			kernel_code,
			kernel_data_size,
			hv_code,
			hv_entry,
			minimum_build,
			maximum_build,
			output,
		} => {
			let kernel = match &kernel_code {
				Some(path) => Some(KernelSection { code: read(path)?, data_size: kernel_data_size }),
				None => None,
			};
			let hv = match &hv_code {
				Some(path) => Some(HvSection { code: read(path)?, entry: hv_entry }),
				None => None,
			};
			let package = expansion::build(&PackageSpec { id, minimum_build, maximum_build, kernel, hv })?;
			fs::write(&output, package).map_err(|source| CliError::Write { path: output, source })
		}
		Command::Verify { public_key, public_key_offset, package } => {
			let key = expansion::load_public_key(&public_key, public_key_offset)?;
			let report = expansion::verify(&read(&package)?, &key)?;
			println!("{report}");
			Ok(())
		}
	}
}

fn main() -> ExitCode {
	match run(Arguments::parse()) {
		Ok(()) => ExitCode::SUCCESS,
		Err(error) => {
			let mut message = error.to_string();
			let mut source = std::error::Error::source(&error);
			while let Some(cause) = source {
				message.push_str(": ");
				message.push_str(&cause.to_string());
				source = cause.source();
			}
			eprintln!("error: {message}");
			ExitCode::FAILURE
		}
	}
}
