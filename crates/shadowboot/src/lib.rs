mod se;

use thiserror::Error;
use xflash::BOOT_CHAIN_OFFSET;
use xflash::BootStage;
use xflash::Image;

pub use se::HYPERVISOR_SIZE;
pub use se::KERNEL_SIZE;
pub use se::PAYLOAD_SIZE;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildReport {
	pub replaced_boot_stages: usize,
	pub rebuilt_se: bool,
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
	let (replaced_boot_stages, rebuilt_se) = apply_boot_layer(image, output, *layer)?;
	Ok(BuildReport { replaced_boot_stages, rebuilt_se })
}

fn apply_boot_layer(image: &Image<'_>, output: &mut [u8], layer: BootLayer<'_>) -> Result<(usize, bool), Error> {
	let base_stages = image.boot_stages();
	let base_sb = image.boot_stage_data(&base_stages[0]);
	let base_sc = image.boot_stage_data(&base_stages[1]);
	let base_sd = image.boot_stage_data(&base_stages[2]);
	let base_se = image.boot_stage_data(&base_stages[3]);

	if layer.se.is_some() && (layer.hypervisor.is_some() || layer.kernel.is_some()) {
		return Err(Error::SeReplacementConflict);
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
		validate_boot_stage(3, se)?;
		drop(se::decode(sc, sd, se)?);
		se
	} else if layer.hypervisor.is_some() || layer.kernel.is_some() || se_key_changed {
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
		if payload_changed || se_key_changed {
			rebuilt_se = Some(se::encode(sc, sd, base_se, &payload)?);
			rebuilt_se.as_deref().expect("rebuilt SE assigned")
		} else {
			base_se
		}
	} else {
		base_se
	};
	validate_boot_stage(3, se)?;

	let stages = [sb, sc, sd, se];
	let new_end = stages.iter().try_fold(BOOT_CHAIN_OFFSET, |offset, stage| {
		offset.checked_add(stage.len()).ok_or(Error::BootChainSizeOverflow)
	})?;
	let boot_limit = image.header().boot_image_size.get();
	if new_end > boot_limit {
		return Err(Error::BootRegionOverflow { end: new_end, limit: boot_limit });
	}

	let replaced = [layer.sb.is_some(), layer.sc.is_some(), layer.sd.is_some(), layer.se.is_some()]
		.into_iter()
		.filter(|value| *value)
		.count()
		+ usize::from(rebuilt_se.is_some());
	if replaced != 0 {
		let old_end =
			base_stages.last().map(|stage| stage.offset().get() + stage.size().get()).expect("four base stages");
		output[BOOT_CHAIN_OFFSET..old_end.max(new_end)].fill(0);
		let mut offset = BOOT_CHAIN_OFFSET;
		for stage in stages {
			output[offset..offset + stage.len()].copy_from_slice(stage);
			offset += stage.len();
		}
	}
	Ok((replaced, rebuilt_se.is_some()))
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
}
