mod se;

use std::fmt;

use sfcx::BootRegion;
use sfcx::BootStage;
use thiserror::Error;

pub use se::Payload;
pub use se::SE_ALIGNMENT;

/// Largest image the build 16547 kernel can load: its shadowboot buffer is restricted to
/// physical 0x01000000..0x01800000.
pub const LOADER_SIZE_LIMIT: usize = 0x80_0000;

#[derive(Debug, Clone, Copy, Default)]
pub struct BootLayer<'a> {
	pub sb: Option<&'a [u8]>,
	pub sc: Option<&'a [u8]>,
	pub sd: Option<&'a [u8]>,
	pub se: Option<&'a [u8]>,
	pub hypervisor: Option<&'a [u8]>,
	pub kernel: Option<&'a [u8]>,
	/// Recompress the base SE payload even when no component bytes change.
	pub rebuild_se: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildReport {
	pub replaced_boot_stages: usize,
	pub rebuilt_se: bool,
	/// The base SD was written with a signed digest of the new SE.
	pub updated_sd_signature: bool,
	pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractReport {
	pub payload: Payload,
	pub warnings: Vec<Warning>,
}

/// A failed integrity check. The image is still written: a console that does not enforce the
/// check, or that trusts another SD key, can boot it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warning {
	/// The base SD keeps an SE digest that does not match the base SE.
	BaseSdDigest { stored: [u8; 20], computed: [u8; 20] },
	/// The base SD signature does not verify with the public key in the base SD.
	BaseSdSignature { digest: [u8; 20] },
	/// The layer SD keeps an SE digest that does not match the written SE. The layer SD is
	/// written as supplied.
	SdDigest { stored: [u8; 20], computed: [u8; 20] },
	/// The written SD signature does not verify with the public key in the base SD.
	SdSignature { digest: [u8; 20] },
}

impl fmt::Display for Warning {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::BaseSdDigest { stored, computed } => {
				write!(formatter, "base SD keeps SE digest {stored:02x?}, but the base SE has {computed:02x?}")
			}
			Self::BaseSdSignature { digest } => {
				write!(formatter, "base SD signature does not authenticate digest {digest:02x?}")
			}
			Self::SdDigest { stored, computed } => {
				write!(formatter, "layer SD keeps SE digest {stored:02x?}, but the written SE has {computed:02x?}")
			}
			Self::SdSignature { digest } => {
				write!(
					formatter,
					"written SD signature does not authenticate digest {digest:02x?} with the base SD key"
				)
			}
		}
	}
}

#[derive(Debug, Error)]
pub enum Error {
	#[error(transparent)]
	Se(#[from] se::Error),
	#[error("boot region size {size:#x} exceeds the kernel shadowboot load limit {limit:#x}")]
	LoaderSizeLimit { size: usize, limit: usize },
	#[error("output size {actual:#x} does not match boot region size {expected:#x}")]
	OutputSize { expected: usize, actual: usize },
	#[error("boot layer supplies SE together with a hypervisor or kernel replacement")]
	SeReplacementConflict,
	#[error("boot layer supplies SE and also requests rebuilding SE")]
	SeRebuildConflict,
	#[error("boot stage {stage} changes the downstream key state, so boot stage {required} must also be supplied")]
	BootStageDependency { stage: &'static str, required: &'static str },
	#[error("boot stage {stage} is unchanged even though {upstream} changes its body key")]
	StaleBootStage { stage: &'static str, upstream: &'static str },
	#[error("{component} input has size {actual:#x}; expected {expected:#x}")]
	ComponentSize { component: &'static str, expected: usize, actual: usize },
	#[error("replacement hypervisor declares size {declared:#x}; expected {expected:#x}")]
	HypervisorDeclaredSize { declared: usize, expected: usize },
	#[error("boot stage {expected} input has magic {actual:02x?}")]
	BootStageMagic { expected: &'static str, actual: [u8; 2] },
	#[error("boot stage {stage} is shorter than its 0x10-byte image header")]
	BootStageHeaderTruncated { stage: &'static str },
	#[error("boot stage {stage} declares size {declared:#x}, but its input has size {actual:#x}")]
	BootStageSize { stage: &'static str, declared: usize, actual: usize },
	#[error("boot stage sizes overflow the host address space")]
	BootChainSizeOverflow,
	#[error("boot chain ends at {end:#x}, past the boot-region limit {limit:#x}")]
	BootRegionOverflow { end: usize, limit: usize },
}

/// Size of the shadowboot image compiled from `region`: its declared boot region.
pub fn output_size(region: &BootRegion<'_>) -> Result<usize, Error> {
	let size = region.header().boot_image_size.get();
	if size > LOADER_SIZE_LIMIT {
		return Err(Error::LoaderSizeLimit { size, limit: LOADER_SIZE_LIMIT });
	}

	Ok(size)
}

pub fn compile(region: &BootRegion<'_>, output: &mut [u8], layer: &BootLayer<'_>) -> Result<BuildReport, Error> {
	let size = output_size(region)?;
	if output.len() != size {
		return Err(Error::OutputSize { expected: size, actual: output.len() });
	}

	output.copy_from_slice(region.bytes());
	apply_boot_layer(region, output, *layer)
}

/// The decompressed SE payload of `region`. SD must keep the digest of the SE.
pub fn payload(region: &BootRegion<'_>) -> Result<ExtractReport, Error> {
	let stages = region.boot_stages();
	let sc = region.boot_stage_data(&stages[1]);
	let sd = region.boot_stage_data(&stages[2]);
	let se = region.boot_stage_data(&stages[3]);
	let warnings = check_base_sd(region, sc, sd, se)?;

	Ok(ExtractReport { payload: se::decode(sc, sd, se)?, warnings })
}

/// The bytes of `region` that follow its SE up to the SE alignment.
fn base_se_padding<'a>(region: &BootRegion<'a>) -> Result<&'a [u8], Error> {
	let stage = &region.boot_stages()[3];
	let end = stage.offset().get() + stage.size().get();
	let aligned = aligned_se_end(stage);

	region.bytes().get(end..aligned).ok_or(Error::BootRegionOverflow { end: aligned, limit: region.bytes().len() })
}

/// End of SE in flash: SE keeps bytes up to its size rounded up to the SE alignment.
fn aligned_se_end(stage: &BootStage) -> usize {
	stage.offset().get() + stage.size().get().next_multiple_of(SE_ALIGNMENT)
}

/// Compares the SE digest that the base SD keeps with the base SE and checks the base SD
/// signature.
fn check_base_sd(region: &BootRegion<'_>, sc: &[u8], sd: &[u8], se: &[u8]) -> Result<Vec<Warning>, Error> {
	let mut warnings = Vec::new();
	let stored = se::sd_se_digest(sc, sd)?;
	let computed = se::digest(sc, sd, se, base_se_padding(region)?)?;
	if stored != computed {
		warnings.push(Warning::BaseSdDigest { stored, computed });
	}
	if !se::sd_signature_verifies(sc, sd, sc, sd)? {
		warnings.push(Warning::BaseSdSignature { digest: se::sd_signature_digest(sc, sd)? });
	}

	Ok(warnings)
}

fn apply_boot_layer(region: &BootRegion<'_>, output: &mut [u8], layer: BootLayer<'_>) -> Result<BuildReport, Error> {
	let base_stages = region.boot_stages();
	let base_sb = region.boot_stage_data(&base_stages[0]);
	let base_sc = region.boot_stage_data(&base_stages[1]);
	let base_sd = region.boot_stage_data(&base_stages[2]);
	let base_se = region.boot_stage_data(&base_stages[3]);
	let chain_offset = region.boot_chain_offset().get();

	if layer.se.is_some() && (layer.hypervisor.is_some() || layer.kernel.is_some()) {
		return Err(Error::SeReplacementConflict);
	}
	if layer.se.is_some() && layer.rebuild_se {
		return Err(Error::SeRebuildConflict);
	}

	let sb = layer.sb.unwrap_or(base_sb);
	let sc = layer.sc.unwrap_or(base_sc);
	let sd = layer.sd.unwrap_or(base_sd);
	for (index, stage) in [sb, sc, sd].iter().enumerate() {
		validate_boot_stage(index, stage)?;
	}
	let boot_limit = region.bytes().len();
	let selected_end = boot_chain_end(chain_offset, &[sb, sc, sd])?;
	if selected_end > boot_limit {
		return Err(Error::BootRegionOverflow { end: selected_end, limit: boot_limit });
	}
	if let Some(se) = layer.se {
		validate_boot_stage(3, se)?;
		let selected_end = boot_chain_end(chain_offset, &[sb, sc, sd, se])?;
		if selected_end > boot_limit {
			return Err(Error::BootRegionOverflow { end: selected_end, limit: boot_limit });
		}
	}
	let mut warnings = check_base_sd(region, base_sc, base_sd, base_se)?;

	let base_sc_state = se::sc_key_state(base_sc)?;
	let sc_state = se::sc_key_state(sc)?;
	if sc_state != base_sc_state {
		if layer.sd.is_none() {
			return Err(Error::BootStageDependency { stage: "SC", required: "SD" });
		}
		if sd == base_sd {
			return Err(Error::StaleBootStage { stage: "SD", upstream: "SC" });
		}
	}
	let se_key_changed = se::sd_key_state(sc, sd)? != se::sd_key_state(base_sc, base_sd)?;

	let mut rebuilt_se = None;
	let se = if let Some(se) = layer.se {
		drop(se::decode(sc, sd, se)?);
		se
	} else if layer.hypervisor.is_some() || layer.kernel.is_some() || se_key_changed || layer.rebuild_se {
		let base_payload = se::decode(base_sc, base_sd, base_se)?;
		if let Some(hypervisor) = layer.hypervisor {
			validate_component_size("hypervisor", base_payload.hypervisor().len(), hypervisor.len())?;
		}
		if let Some(kernel) = layer.kernel {
			validate_component_size("kernel", base_payload.kernel().len(), kernel.len())?;
		}
		let payload_changed = layer.hypervisor.is_some_and(|hypervisor| hypervisor != base_payload.hypervisor())
			|| layer.kernel.is_some_and(|kernel| kernel != base_payload.kernel());

		if payload_changed || se_key_changed || layer.rebuild_se {
			let hypervisor_size = base_payload.hypervisor().len();
			let mut bytes = base_payload.into_bytes();
			if let Some(hypervisor) = layer.hypervisor {
				bytes[..hypervisor_size].copy_from_slice(hypervisor);
			}
			if let Some(kernel) = layer.kernel {
				bytes[hypervisor_size..].copy_from_slice(kernel);
			}
			let payload = Payload::parse(bytes)?;
			if payload.hypervisor().len() != hypervisor_size {
				return Err(Error::HypervisorDeclaredSize {
					declared: payload.hypervisor().len(),
					expected: hypervisor_size,
				});
			}
			rebuilt_se = Some(se::encode(sc, sd, base_se, &payload)?);
			rebuilt_se.as_deref().expect("rebuilt SE assigned")
		} else {
			base_se
		}
	} else {
		base_se
	};
	validate_boot_stage(3, se)?;

	// Flash holds SE up to its aligned end; the base keeps its own bytes there.
	let padding =
		if se == base_se && !se_key_changed { base_se_padding(region)?.to_vec() } else { se::padding(sc, sd, se)? };

	// SD authenticates the SE digest. A layer SD must already authenticate it; otherwise the
	// base SD is updated and signed.
	let digest = se::digest(sc, sd, se, &padding)?;
	let stored = se::sd_se_digest(sc, sd)?;
	let mut updated_sd = None;
	if stored != digest {
		match layer.sd {
			Some(_) => warnings.push(Warning::SdDigest { stored, computed: digest }),
			None => updated_sd = Some(se::with_authenticated_sd_se_digest(sc, sd, &digest)?),
		}
	}
	let sd = updated_sd.as_deref().unwrap_or(sd);
	// An unchanged SD was checked with the base.
	if sd != base_sd && !se::sd_signature_verifies(sc, sd, base_sc, base_sd)? {
		warnings.push(Warning::SdSignature { digest: se::sd_signature_digest(sc, sd)? });
	}

	let stages = [sb, sc, sd, se];
	let new_end =
		boot_chain_end(chain_offset, &stages)?.checked_add(padding.len()).ok_or(Error::BootChainSizeOverflow)?;
	if new_end > boot_limit {
		return Err(Error::BootRegionOverflow { end: new_end, limit: boot_limit });
	}

	let replaced = [layer.sb.is_some(), layer.sc.is_some(), layer.sd.is_some(), layer.se.is_some()]
		.into_iter()
		.filter(|value| *value)
		.count()
		+ usize::from(rebuilt_se.is_some());
	if replaced != 0 || updated_sd.is_some() {
		let old_end = aligned_se_end(&base_stages[3]).min(boot_limit);
		output[chain_offset..old_end.max(new_end)].fill(0);
		let mut offset = chain_offset;
		for stage in stages.into_iter().chain([padding.as_slice()]) {
			output[offset..offset + stage.len()].copy_from_slice(stage);
			offset += stage.len();
		}
	}

	Ok(BuildReport {
		replaced_boot_stages: replaced,
		rebuilt_se: rebuilt_se.is_some(),
		updated_sd_signature: updated_sd.is_some(),
		warnings,
	})
}

fn boot_chain_end(chain_offset: usize, stages: &[&[u8]]) -> Result<usize, Error> {
	stages
		.iter()
		.try_fold(chain_offset, |offset, stage| offset.checked_add(stage.len()).ok_or(Error::BootChainSizeOverflow))
}

fn validate_boot_stage(index: usize, bytes: &[u8]) -> Result<(), Error> {
	const NAMES: [&str; 4] = ["SB", "SC", "SD", "SE"];
	let stage = NAMES[index];
	if bytes.len() < 0x10 {
		return Err(Error::BootStageHeaderTruncated { stage });
	}
	let expected = stage.as_bytes();
	let actual = [bytes[0], bytes[1]];
	if actual != [expected[0], expected[1]] {
		return Err(Error::BootStageMagic { expected: stage, actual });
	}
	let declared = u32::from_be_bytes(bytes[0x0c..0x10].try_into().expect("fixed stage size field")) as usize;
	if declared != bytes.len() {
		return Err(Error::BootStageSize { stage, declared, actual: bytes.len() });
	}
	Ok(())
}

fn validate_component_size(component: &'static str, expected: usize, actual: usize) -> Result<(), Error> {
	if expected != actual {
		return Err(Error::ComponentSize { component, expected, actual });
	}
	Ok(())
}

pub fn stage_data<'a>(region: &'a BootRegion<'a>, name: &str) -> Option<&'a [u8]> {
	region
		.boot_stages()
		.iter()
		.find(|stage: &&BootStage| stage.name_str().eq_ignore_ascii_case(name))
		.map(|stage| region.boot_stage_data(stage))
}

#[cfg(test)]
mod tests {
	use std::fs::File;

	use memmap2::Mmap;

	use super::*;

	/// Runs against a private 60 MiB devkit flash capture named by `SHADOWBOOT_BASE_IMAGE`.
	#[test]
	#[ignore = "requires SHADOWBOOT_BASE_IMAGE"]
	fn unchanged_layers_reproduce_base_boot_region() {
		let base = base_image();
		let region = BootRegion::parse(&base).unwrap();
		let size = region.bytes().len();
		let stages = region.boot_stages();
		let stage = |index: usize| Some(region.boot_stage_data(&stages[index]));
		let payload = se::decode(stage(1).unwrap(), stage(2).unwrap(), stage(3).unwrap()).unwrap();

		let layers = [
			BootLayer::default(),
			BootLayer { sb: stage(0), sc: stage(1), sd: stage(2), se: stage(3), ..BootLayer::default() },
			BootLayer {
				hypervisor: Some(payload.hypervisor()),
				kernel: Some(payload.kernel()),
				..BootLayer::default()
			},
		];
		let mut output = vec![0u8; output_size(&region).unwrap()];
		for layer in &layers {
			output.fill(0);
			let report = compile(&region, &mut output, layer).unwrap();
			assert_eq!(report.warnings, []);
			assert_eq!(output.len(), size);
			assert!(output == base[..size]);
		}
	}

	/// Runs against a private 60 MiB devkit flash capture named by `SHADOWBOOT_BASE_IMAGE`. A
	/// recompressed SE and a changed kernel each decompress to their payload, and the compiled
	/// SD authenticates the new SE digest. The compiled boot region is a base image itself.
	#[test]
	#[ignore = "requires SHADOWBOOT_BASE_IMAGE"]
	fn rebuilt_se_round_trips_with_authenticated_sd() {
		let base = base_image();
		let region = BootRegion::parse(&base).unwrap();
		let stages = region.boot_stages();
		let stage = |index: usize| region.boot_stage_data(&stages[index]);
		let stock = payload(&region).unwrap().payload;

		let recompressed = se::encode(stage(1), stage(2), stage(3), &stock).unwrap();
		assert_ne!(recompressed.as_slice(), stage(3));
		let mut kernel = stock.kernel().to_vec();
		kernel[0x1000] ^= 0xff;
		let changed = Payload::parse([stock.hypervisor(), kernel.as_slice()].concat()).unwrap();

		let cases = [
			(BootLayer { rebuild_se: true, ..BootLayer::default() }, &stock),
			(BootLayer { se: Some(&recompressed), ..BootLayer::default() }, &stock),
			(BootLayer { kernel: Some(&kernel), ..BootLayer::default() }, &changed),
		];
		let mut output = vec![0u8; output_size(&region).unwrap()];
		let conflict = BootLayer { se: Some(stage(3)), rebuild_se: true, ..BootLayer::default() };
		assert!(matches!(compile(&region, &mut output, &conflict), Err(Error::SeRebuildConflict)));
		for (layer, expected) in &cases {
			let report = compile(&region, &mut output, layer).unwrap();
			assert!(report.updated_sd_signature);
			assert_eq!(report.warnings, []);
			let compiled = BootRegion::parse(&output).unwrap();
			let extracted = payload(&compiled).unwrap();
			assert_eq!(extracted.warnings, []);
			assert!(extracted.payload == **expected);
			assert!(compiled.boot_stage_data(&compiled.boot_stages()[1]) == stage(1));
		}
	}

	/// Runs against a private 60 MiB devkit flash capture named by `SHADOWBOOT_BASE_IMAGE`. An SD
	/// whose signature does not verify is written with a warning, reported once.
	#[test]
	#[ignore = "requires SHADOWBOOT_BASE_IMAGE"]
	fn unverified_sd_signature_warns() {
		let base = base_image();
		let region = BootRegion::parse(&base).unwrap();
		let sd_stage = region.boot_stages()[2];
		// RC4 flips the plaintext byte under a flipped ciphertext byte. 0x300 is signed and is
		// not the SE digest.
		let flipped = sd_stage.offset().get() + 0x300;
		let mut sd = region.boot_stage_data(&sd_stage).to_vec();
		sd[0x300] ^= 0x01;
		let mut output = vec![0u8; output_size(&region).unwrap()];

		let report = compile(&region, &mut output, &BootLayer { sd: Some(&sd), ..BootLayer::default() }).unwrap();
		assert!(matches!(report.warnings.as_slice(), [Warning::SdSignature { .. }]));
		assert!(output[flipped] != base[flipped]);

		let mut tampered = region.bytes().to_vec();
		tampered[flipped] ^= 0x01;
		let tampered = BootRegion::parse(&tampered).unwrap();
		assert!(matches!(payload(&tampered).unwrap().warnings.as_slice(), [Warning::BaseSdSignature { .. }]));
		let report = compile(&tampered, &mut output, &BootLayer::default()).unwrap();
		assert!(matches!(report.warnings.as_slice(), [Warning::BaseSdSignature { .. }]));
	}

	/// Runs against a private 60 MiB devkit flash capture named by `SHADOWBOOT_BASE_IMAGE`. An SD
	/// that keeps another SE digest is reported, not rejected. A base SD with a wrong digest is
	/// replaced by a signed SD with the right one.
	#[test]
	#[ignore = "requires SHADOWBOOT_BASE_IMAGE"]
	fn mismatched_sd_digest_warns() {
		const SD_SE_DIGEST: usize = 0x24c;

		let base = base_image();
		let region = BootRegion::parse(&base).unwrap();
		let sd_stage = region.boot_stages()[2];
		let flipped = sd_stage.offset().get() + SD_SE_DIGEST;
		let mut sd = region.boot_stage_data(&sd_stage).to_vec();
		sd[SD_SE_DIGEST] ^= 0x01;
		let mut output = vec![0u8; output_size(&region).unwrap()];

		let report = compile(&region, &mut output, &BootLayer { sd: Some(&sd), ..BootLayer::default() }).unwrap();
		assert!(matches!(report.warnings.as_slice(), [Warning::SdDigest { .. }, Warning::SdSignature { .. }]));

		let mut tampered = region.bytes().to_vec();
		tampered[flipped] ^= 0x01;
		let tampered = BootRegion::parse(&tampered).unwrap();
		let extracted = payload(&tampered).unwrap();
		assert!(matches!(
			extracted.warnings.as_slice(),
			[Warning::BaseSdDigest { .. }, Warning::BaseSdSignature { .. }]
		));
		let report = compile(&tampered, &mut output, &BootLayer::default()).unwrap();
		assert!(report.updated_sd_signature);
		assert!(matches!(report.warnings.as_slice(), [Warning::BaseSdDigest { .. }, Warning::BaseSdSignature { .. }]));
		assert!(output == base[..output.len()]);
	}

	fn base_image() -> Mmap {
		let path = std::env::var_os("SHADOWBOOT_BASE_IMAGE").expect("SHADOWBOOT_BASE_IMAGE is set");
		let file = File::open(path).unwrap();
		unsafe { Mmap::map(&file) }.unwrap()
	}
}
