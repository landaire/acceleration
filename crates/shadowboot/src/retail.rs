//! A retail base accepted with `ignore_retail_checks`. Its stages are encrypted with keys that
//! derive from the CPU key, so only changes that need no stage keys apply: whole-stage
//! replacements of its own roles. The image cannot shadowboot on a stock console: a retail CB
//! does not load SC and SD from the image in RAM.

use sfcx::BootRegion;
use sfcx::ChainKind;
use sfcx::StageName;
use sfcx::StageRole;

use crate::Binding;
use crate::BootLayer;
use crate::BuildReport;
use crate::ChainLayout;
use crate::DevkitOperation;
use crate::Error;
use crate::SE_ALIGNMENT;
use crate::Warning;
use crate::base_padding;

/// Applies the whole-stage replacements of `layer` to the retail chain of `region`, already
/// copied to `output`. Each stage starts at the end of the one before it rounded up to 16 bytes;
/// a stage the layer leaves alone keeps the base bytes up to there.
pub(crate) fn apply_layer(
	region: &BootRegion<'_>,
	output: &mut [u8],
	layer: &BootLayer<'_>,
) -> Result<(BuildReport, ChainLayout), Error> {
	let devkit_operation = if layer.hypervisor.is_some() {
		Some(DevkitOperation::ReplaceHypervisor)
	} else if layer.kernel.is_some() {
		Some(DevkitOperation::ReplaceKernel)
	} else if layer.rebuild_se {
		Some(DevkitOperation::RebuildSe)
	} else {
		None
	};
	if let Some(operation) = devkit_operation {
		return Err(Error::RequiresDevkitChain { operation });
	}

	let chain: Vec<_> = region.chain().stages().collect();
	for (role, _) in layer.stages {
		if !chain.iter().any(|(candidate, _)| candidate == role) {
			return Err(Error::LayerStageNotInChain { stage: *role, kind: ChainKind::Retail });
		}
	}

	let mut pieces: Vec<&[u8]> = Vec::with_capacity(chain.len() * 2);
	let mut changed = false;
	for (role, stage) in &chain {
		let base = region.boot_stage_data(stage);
		let bytes = layer.stage(*role).unwrap_or(base);
		validate_stage(*role, stage.name(), bytes)?;
		if matches!(role, StageRole::Cb | StageRole::CbA) {
			keep_chain_shape(base, bytes)?;
		}
		let padding = if bytes == base {
			base_padding(region, stage)?
		} else if bytes.len().is_multiple_of(SE_ALIGNMENT) {
			&[]
		} else {
			return Err(Error::UnalignedBootStage { stage: stage.name(), size: bytes.len() });
		};
		changed |= bytes != base;
		pieces.push(bytes);
		pieces.push(padding);
	}

	let chain_offset = region.boot_chain_offset().get();
	let limit = region.bytes().len();
	let end = pieces
		.iter()
		.try_fold(chain_offset, |offset, piece| offset.checked_add(piece.len()))
		.ok_or(Error::BootChainSizeOverflow)?;
	if end > limit {
		return Err(Error::BootRegionOverflow { end, limit });
	}
	if changed {
		let (_, last) = chain.last().expect("a chain has stages");
		output[chain_offset..last.aligned_end().min(limit).max(end)].fill(0);
		let mut offset = chain_offset;
		for piece in pieces.iter().copied() {
			output[offset..offset + piece.len()].copy_from_slice(piece);
			offset += piece.len();
		}
	}

	let (_, first) = chain[0];
	let report = BuildReport {
		replaced_boot_stages: layer.stages.len(),
		rebuilt_se: false,
		updated_sd_signature: false,
		binding: Binding::RetailCb,
		warnings: vec![Warning::RetailBase { offset: first.offset().get(), name: first.name() }],
	};
	let roles: Vec<StageRole> = chain.iter().map(|(role, _)| *role).collect();

	Ok((report, ChainLayout::planned(chain_offset, &roles, &pieces)))
}

/// A replacement first stage must keep what selects the chain's shape: the console key letter,
/// which makes the chain retail, and the split flag, which says whether a CB_B follows.
fn keep_chain_shape(base: &[u8], bytes: &[u8]) -> Result<(), Error> {
	const CONSOLE_KEY_MASK: u8 = 0xf0;
	const SPLIT_FLAG: u16 = 0x0800;

	let expected = StageName::new([base[0], base[1]]);
	let actual = StageName::new([bytes[0], bytes[1]]);
	if bytes[0] & CONSOLE_KEY_MASK != base[0] & CONSOLE_KEY_MASK {
		return Err(Error::FirstStageKindChanged { expected, actual });
	}
	let flags = |stage: &[u8]| u16::from_be_bytes([stage[6], stage[7]]);
	if flags(bytes) & SPLIT_FLAG != flags(base) & SPLIT_FLAG {
		return Err(Error::SplitFlagChanged { stage: actual });
	}

	Ok(())
}

/// Checks a stage input against the base stage it replaces, which is named `base`: a loader of
/// the same role, keeping the base's name, and declaring the input's size.
fn validate_stage(role: StageRole, base: StageName, bytes: &[u8]) -> Result<(), Error> {
	if bytes.len() < 0x10 {
		return Err(Error::BootStageHeaderTruncated { stage: base });
	}
	let actual = StageName::new([bytes[0], bytes[1]]);
	if !role.matches(actual) {
		return Err(Error::BootStageMagic { expected: base, actual });
	}
	let declared = u32::from_be_bytes(bytes[0x0c..0x10].try_into().expect("fixed stage size field")) as usize;
	if declared != bytes.len() {
		return Err(Error::BootStageSize { stage: base, declared, actual: bytes.len() });
	}

	Ok(())
}
