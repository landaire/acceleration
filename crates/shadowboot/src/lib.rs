mod se;

use thiserror::Error;
use xflash::BOOT_CHAIN_OFFSET;
use xflash::BootStage;
use xflash::Image;

pub use se::HYPERVISOR_SIZE;
pub use se::KERNEL_SIZE;
pub use se::PAYLOAD_SIZE;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildReport {
	pub replaced_boot_stages: usize,
	pub rebuilt_se: bool,
	/// The base SD was written with a signed digest of the new SE.
	pub updated_sd_signature: bool,
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
	#[error("boot stage {expected} input has magic {actual:02x?}")]
	BootStageMagic { expected: &'static str, actual: [u8; 2] },
	#[error("boot stage {stage} is shorter than its 0x10-byte image header")]
	BootStageHeaderTruncated { stage: &'static str },
	#[error(
		"SD of the base image keeps SE digest {stored:02x?}, but its SE has {computed:02x?}; SD does not keep the digest where the compiler writes it"
	)]
	BaseSdDigest { stored: [u8; 20], computed: [u8; 20] },
	#[error("boot stage SD of the layer keeps SE digest {stored:02x?}, but the SE of the image has {computed:02x?}")]
	SdDigest { stored: [u8; 20], computed: [u8; 20] },
	#[error("boot stage SD signature does not authenticate digest {digest:02x?}")]
	SdSignature { digest: [u8; 20] },
	#[error("boot stage {stage} declares size {declared:#x}, but its input has size {actual:#x}")]
	BootStageSize { stage: &'static str, declared: usize, actual: usize },
	#[error("boot stage sizes overflow the host address space")]
	BootChainSizeOverflow,
	#[error("boot chain ends at {end:#x}, past the boot-region limit {limit:#x}")]
	BootRegionOverflow { end: usize, limit: usize },
}

/// Size of the shadowboot image compiled from `image`: its declared boot region.
pub fn output_size(image: &Image<'_>) -> Result<usize, Error> {
	let size = image.header().boot_image_size.get();
	if size > LOADER_SIZE_LIMIT {
		return Err(Error::LoaderSizeLimit { size, limit: LOADER_SIZE_LIMIT });
	}
	Ok(size)
}

pub fn compile(image: &Image<'_>, output: &mut [u8], layer: &BootLayer<'_>) -> Result<BuildReport, Error> {
	let size = output_size(image)?;
	if output.len() != size {
		return Err(Error::OutputSize { expected: size, actual: output.len() });
	}
	output.copy_from_slice(&image.bytes()[..size]);
	apply_boot_layer(image, output, *layer)
}

/// The decompressed SE payload of `image`: the hypervisor followed by the kernel. SD must keep
/// the digest of the SE.
pub fn payload(image: &Image<'_>) -> Result<Vec<u8>, Error> {
	let stages = image.boot_stages();
	let sc = image.boot_stage_data(&stages[1]);
	let sd = image.boot_stage_data(&stages[2]);
	let se = image.boot_stage_data(&stages[3]);
	check_base_sd_digest(image, sc, sd, se)?;
	Ok(se::decode(sc, sd, se)?)
}

/// The bytes of `image` that follow its SE up to the SE alignment.
fn base_se_padding<'a>(image: &Image<'a>) -> Result<&'a [u8], Error> {
	let stage = &image.boot_stages()[3];
	let end = stage.offset().get() + stage.size().get();
	let aligned = end.next_multiple_of(SE_ALIGNMENT);
	let limit = image.header().boot_image_size.get();
	image
		.bytes()
		.get(end..aligned)
		.filter(|_| aligned <= limit)
		.ok_or(Error::BootRegionOverflow { end: aligned, limit })
}

/// Checks that the base SD keeps the digest of the base SE, where the compiler reads and writes
/// it.
fn check_base_sd_digest(image: &Image<'_>, sc: &[u8], sd: &[u8], se: &[u8]) -> Result<(), Error> {
	let stored = se::sd_se_digest(sc, sd)?;
	let computed = se::digest(sc, sd, se, base_se_padding(image)?)?;
	if stored != computed {
		return Err(Error::BaseSdDigest { stored, computed });
	}
	if !se::sd_signature_verifies(sc, sd, sc, sd)? {
		return Err(Error::SdSignature { digest: se::sd_signature_digest(sc, sd)? });
	}
	Ok(())
}

fn apply_boot_layer(image: &Image<'_>, output: &mut [u8], layer: BootLayer<'_>) -> Result<BuildReport, Error> {
	let base_stages = image.boot_stages();
	let base_sb = image.boot_stage_data(&base_stages[0]);
	let base_sc = image.boot_stage_data(&base_stages[1]);
	let base_sd = image.boot_stage_data(&base_stages[2]);
	let base_se = image.boot_stage_data(&base_stages[3]);

	if layer.se.is_some() && (layer.hypervisor.is_some() || layer.kernel.is_some()) {
		return Err(Error::SeReplacementConflict);
	}
	if layer.se.is_some() && layer.rebuild_se {
		return Err(Error::SeRebuildConflict);
	}
	if let Some(hypervisor) = layer.hypervisor {
		validate_component_size("hypervisor", HYPERVISOR_SIZE, hypervisor.len())?;
	}
	if let Some(kernel) = layer.kernel {
		validate_component_size("kernel", KERNEL_SIZE, kernel.len())?;
	}

	let sb = layer.sb.unwrap_or(base_sb);
	let sc = layer.sc.unwrap_or(base_sc);
	let sd = layer.sd.unwrap_or(base_sd);
	for (index, stage) in [sb, sc, sd].iter().enumerate() {
		validate_boot_stage(index, stage)?;
	}
	let boot_limit = image.header().boot_image_size.get();
	let selected_end = boot_chain_end(&[sb, sc, sd])?;
	if selected_end > boot_limit {
		return Err(Error::BootRegionOverflow { end: selected_end, limit: boot_limit });
	}
	if let Some(se) = layer.se {
		validate_boot_stage(3, se)?;
		let selected_end = boot_chain_end(&[sb, sc, sd, se])?;
		if selected_end > boot_limit {
			return Err(Error::BootRegionOverflow { end: selected_end, limit: boot_limit });
		}
	}
	check_base_sd_digest(image, base_sc, base_sd, base_se)?;

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
		let mut payload = se::decode(base_sc, base_sd, base_se)?;
		let mut payload_changed = false;
		if let Some(hypervisor) = layer.hypervisor {
			payload_changed |= payload[..HYPERVISOR_SIZE] != *hypervisor;
			payload[..HYPERVISOR_SIZE].copy_from_slice(hypervisor);
		}
		if let Some(kernel) = layer.kernel {
			payload_changed |= payload[HYPERVISOR_SIZE..] != *kernel;
			payload[HYPERVISOR_SIZE..].copy_from_slice(kernel);
		}
		if payload_changed || se_key_changed || layer.rebuild_se {
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
		if se == base_se && !se_key_changed { base_se_padding(image)?.to_vec() } else { se::padding(sc, sd, se)? };

	// SD authenticates the SE digest. A layer SD must already authenticate it; otherwise the
	// base SD is updated and signed.
	let digest = se::digest(sc, sd, se, &padding)?;
	let stored = se::sd_se_digest(sc, sd)?;
	let mut updated_sd = None;
	if stored != digest {
		if layer.sd.is_some() {
			return Err(Error::SdDigest { stored, computed: digest });
		}
		updated_sd = Some(se::with_authenticated_sd_se_digest(sc, sd, &digest)?);
	}
	let sd = updated_sd.as_deref().unwrap_or(sd);
	if !se::sd_signature_verifies(sc, sd, base_sc, base_sd)? {
		return Err(Error::SdSignature { digest: se::sd_signature_digest(sc, sd)? });
	}

	let stages = [sb, sc, sd, se];
	let new_end = boot_chain_end(&stages)?.checked_add(padding.len()).ok_or(Error::BootChainSizeOverflow)?;
	if new_end > boot_limit {
		return Err(Error::BootRegionOverflow { end: new_end, limit: boot_limit });
	}

	let replaced = [layer.sb.is_some(), layer.sc.is_some(), layer.sd.is_some(), layer.se.is_some()]
		.into_iter()
		.filter(|value| *value)
		.count()
		+ usize::from(rebuilt_se.is_some());
	if replaced != 0 || updated_sd.is_some() {
		let old_end = base_stages
			.last()
			.map(|stage| stage.offset().get() + stage.size().get())
			.expect("four base stages")
			.next_multiple_of(SE_ALIGNMENT)
			.min(boot_limit);
		output[BOOT_CHAIN_OFFSET..old_end.max(new_end)].fill(0);
		let mut offset = BOOT_CHAIN_OFFSET;
		for stage in stages.into_iter().chain([padding.as_slice()]) {
			output[offset..offset + stage.len()].copy_from_slice(stage);
			offset += stage.len();
		}
	}
	Ok(BuildReport {
		replaced_boot_stages: replaced,
		rebuilt_se: rebuilt_se.is_some(),
		updated_sd_signature: updated_sd.is_some(),
	})
}

fn boot_chain_end(stages: &[&[u8]]) -> Result<usize, Error> {
	stages.iter().try_fold(BOOT_CHAIN_OFFSET, |offset, stage| {
		offset.checked_add(stage.len()).ok_or(Error::BootChainSizeOverflow)
	})
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

pub fn stage_data<'a>(image: &'a Image<'a>, name: &str) -> Option<&'a [u8]> {
	image
		.boot_stages()
		.iter()
		.find(|stage: &&BootStage| stage.name_str().eq_ignore_ascii_case(name))
		.map(|stage| image.boot_stage_data(stage))
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
		let path = std::env::var_os("SHADOWBOOT_BASE_IMAGE").expect("SHADOWBOOT_BASE_IMAGE is set");
		let file = File::open(path).unwrap();
		let base = unsafe { Mmap::map(&file) }.unwrap();
		let image = Image::parse(&base).unwrap();
		let size = image.header().boot_image_size.get();
		let stages = image.boot_stages();
		let stage = |index: usize| Some(image.boot_stage_data(&stages[index]));
		let payload = se::decode(stage(1).unwrap(), stage(2).unwrap(), stage(3).unwrap()).unwrap();

		let layers = [
			BootLayer::default(),
			BootLayer { sb: stage(0), sc: stage(1), sd: stage(2), se: stage(3), ..BootLayer::default() },
			BootLayer {
				hypervisor: Some(&payload[..HYPERVISOR_SIZE]),
				kernel: Some(&payload[HYPERVISOR_SIZE..]),
				..BootLayer::default()
			},
		];
		let mut output = vec![0u8; output_size(&image).unwrap()];
		for layer in &layers {
			output.fill(0);
			compile(&image, &mut output, layer).unwrap();
			assert_eq!(output.len(), size);
			assert!(output == base[..size]);
		}
	}

	/// Runs against a private 60 MiB devkit flash capture named by `SHADOWBOOT_BASE_IMAGE`. A
	/// recompressed SE and a changed kernel each decompress to their payload, and the compiled
	/// SD authenticates the new SE digest.
	#[test]
	#[ignore = "requires SHADOWBOOT_BASE_IMAGE"]
	fn rebuilt_se_round_trips_with_authenticated_sd() {
		let path = std::env::var_os("SHADOWBOOT_BASE_IMAGE").expect("SHADOWBOOT_BASE_IMAGE is set");
		let file = File::open(path).unwrap();
		let base = unsafe { Mmap::map(&file) }.unwrap();
		let image = Image::parse(&base).unwrap();
		let stages = image.boot_stages();
		let stage = |index: usize| image.boot_stage_data(&stages[index]);
		let stock = payload(&image).unwrap();

		let recompressed = se::encode(stage(1), stage(2), stage(3), &stock).unwrap();
		assert_ne!(recompressed.as_slice(), stage(3));
		let mut kernel = stock[HYPERVISOR_SIZE..].to_vec();
		kernel[0x1000] ^= 0xff;
		let mut changed = stock.clone();
		changed[HYPERVISOR_SIZE..].copy_from_slice(&kernel);

		let cases = [
			(BootLayer { rebuild_se: true, ..BootLayer::default() }, &stock),
			(BootLayer { se: Some(&recompressed), ..BootLayer::default() }, &stock),
			(BootLayer { kernel: Some(&kernel), ..BootLayer::default() }, &changed),
		];
		let mut output = vec![0u8; output_size(&image).unwrap()];
		let conflict = BootLayer { se: Some(stage(3)), rebuild_se: true, ..BootLayer::default() };
		assert!(matches!(compile(&image, &mut output, &conflict), Err(Error::SeRebuildConflict)));
		for (layer, expected) in &cases {
			let report = compile(&image, &mut output, layer).unwrap();
			assert!(report.updated_sd_signature);
			let mut flash = base.to_vec();
			flash[..output.len()].copy_from_slice(&output);
			let compiled = Image::parse(&flash).unwrap();
			assert!(payload(&compiled).unwrap() == **expected);
			assert!(compiled.boot_stage_data(&compiled.boot_stages()[1]) == stage(1));
		}
	}
}
