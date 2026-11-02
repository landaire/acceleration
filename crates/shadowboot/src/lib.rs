mod copyright;
mod retail;
mod sb;
mod se;

use std::fmt;

use sfcx::BootChain;
use sfcx::BootRegion;
use sfcx::BootStage;
use sfcx::ChainKind;
use sfcx::StageName;
use sfcx::StageRole;
use thiserror::Error;

pub use copyright::Copyright;
pub use copyright::CopyrightError;
pub use se::Payload;
pub use se::SE_ALIGNMENT;

/// Largest image the build 16547 kernel can load: its shadowboot buffer is restricted to
/// physical 0x01000000..0x01800000.
pub const LOADER_SIZE_LIMIT: usize = 0x80_0000;

/// Why a retail chain cannot be a base: shadowboot needs an SB that loads SC and SD from the
/// image in RAM, and an SC and SD signed for it.
const RETAIL_EXPLANATION: &str = "A retail boot chain does not know how to shadowboot. A devkit boot chain is \
	required: a devkit SB, with an SC and SD signed for it. You can use whatever hypervisor/kernel you want.";

#[derive(Debug, Clone, Copy, Default)]
pub struct BootLayer<'a> {
	/// Complete encrypted stages, by the role of the base stage each replaces.
	pub stages: &'a [(StageRole, &'a [u8])],
	pub hypervisor: Option<&'a [u8]>,
	pub kernel: Option<&'a [u8]>,
	/// Recompress the base SE payload even when no component bytes change.
	pub rebuild_se: bool,
	/// Keep the console binding of the written SB, so the image boots only on the console that
	/// bound it. Otherwise the binding is cleared.
	pub bind: bool,
	/// Replace the copyright text of the flash header.
	pub copyright: Option<&'a Copyright>,
	/// Accept a retail base with a warning, for the changes that need no stage keys: whole-stage
	/// replacements and the copyright text.
	pub ignore_retail_checks: bool,
}

impl<'a> BootLayer<'a> {
	pub fn stage(&self, role: StageRole) -> Option<&'a [u8]> {
		self.stages.iter().find_map(|(candidate, bytes)| (*candidate == role).then_some(*bytes))
	}
}

/// What happened to the console binding of the first stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
	/// The SB had a binding, which was cleared.
	Cleared,
	/// The SB had no binding, or `bind` kept it.
	Unchanged,
	/// The base is a retail chain: the binding of a retail CB is not an SB binding, and it is
	/// left as it is.
	RetailCb,
}

impl fmt::Display for Binding {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(match self {
			Self::Cleared => "cleared",
			Self::Unchanged => "unchanged",
			Self::RetailCb => "not applicable to a retail CB",
		})
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildReport {
	pub replaced_boot_stages: usize,
	pub rebuilt_se: bool,
	/// The base SD was written with a signed digest of the new SE.
	pub updated_sd_signature: bool,
	pub binding: Binding,
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
	/// The base SD is too short to keep an SE digest or a signature where a devkit SD does.
	BaseSdTooShort { size: usize },
	/// The layer SD keeps an SE digest that does not match the written SE. The layer SD is
	/// written as supplied.
	SdDigest { stored: [u8; 20], computed: [u8; 20] },
	/// The written SD signature does not verify with the public key in the base SD.
	SdSignature { digest: [u8; 20] },
	/// The base is a retail chain, accepted because retail checks are ignored.
	RetailBase { offset: usize, name: StageName },
}

impl fmt::Display for Warning {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::BaseSdDigest { stored, computed } => write!(
				formatter,
				"base \"SD\" keeps \"SE\" digest {stored:02x?}, but the base \"SE\" has {computed:02x?}"
			),
			Self::BaseSdSignature { digest } => {
				write!(formatter, "base \"SD\" signature does not authenticate digest {digest:02x?}")
			}
			Self::BaseSdTooShort { size } => write!(
				formatter,
				"base \"SD\" of {size:#x} bytes is too short to keep an \"SE\" digest at 0x24c or a signature"
			),
			Self::SdDigest { stored, computed } => write!(
				formatter,
				"layer \"SD\" keeps \"SE\" digest {stored:02x?}, but the written \"SE\" has {computed:02x?}"
			),
			Self::SdSignature { digest } => write!(
				formatter,
				"written \"SD\" signature does not authenticate digest {digest:02x?} with the base \"SD\" key"
			),
			Self::RetailBase { offset, name } => {
				write!(formatter, "base image has retail boot stage {name} at {offset:#x}.\n{RETAIL_EXPLANATION}")
			}
		}
	}
}

/// A change that decrypts or re-encrypts the devkit key chain, which a retail chain does not
/// have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevkitOperation {
	ReplaceHypervisor,
	ReplaceKernel,
	RebuildSe,
	Extract,
}

impl fmt::Display for DevkitOperation {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(match self {
			Self::ReplaceHypervisor => "replacing the hypervisor",
			Self::ReplaceKernel => "replacing the kernel",
			Self::RebuildSe => "rebuilding \"SE\"",
			Self::Extract => "extracting the hypervisor and kernel",
		})
	}
}

#[derive(Debug, Error)]
pub enum Error {
	#[error(transparent)]
	Se(#[from] se::Error),
	#[error(transparent)]
	Sb(#[from] sb::Error),
	#[error("boot region size {size:#x} exceeds the kernel shadowboot load limit {limit:#x}")]
	LoaderSizeLimit { size: usize, limit: usize },
	#[error("output size {actual:#x} does not match boot region size {expected:#x}")]
	OutputSize { expected: usize, actual: usize },
	#[error(
		"base image has retail boot stage {name} at {offset:#x}.\n{explanation}\n\nPass --ignore-retail-checks if you really want to use the retail image.",
		explanation = RETAIL_EXPLANATION
	)]
	RetailBase { offset: usize, name: StageName },
	#[error("{operation} requires a devkit boot chain, and the base has a retail one")]
	RequiresDevkitChain { operation: DevkitOperation },
	#[error("boot layer supplies boot stage {stage}, which the {kind} base chain does not have")]
	LayerStageNotInChain { stage: StageRole, kind: ChainKind },
	#[error("boot layer supplies \"SE\" together with a hypervisor or kernel replacement")]
	SeReplacementConflict,
	#[error("boot layer supplies \"SE\" and also requests rebuilding \"SE\"")]
	SeRebuildConflict,
	#[error("boot stage {stage} changes the downstream key state, so boot stage {required} must also be supplied")]
	BootStageDependency { stage: StageName, required: StageName },
	#[error("boot stage {stage} is unchanged even though {upstream} changes its body key")]
	StaleBootStage { stage: StageName, upstream: StageName },
	#[error("{component} input has size {actual:#x}; expected {expected:#x}")]
	ComponentSize { component: &'static str, expected: usize, actual: usize },
	#[error("replacement hypervisor declares size {declared:#x}; expected {expected:#x}")]
	HypervisorDeclaredSize { declared: usize, expected: usize },
	#[error("boot stage {expected} input is named {actual}")]
	BootStageMagic { expected: StageName, actual: StageName },
	#[error("boot stage {stage} is shorter than its 0x10-byte image header")]
	BootStageHeaderTruncated { stage: StageName },
	#[error("boot stage {stage} declares size {declared:#x}, but its input has size {actual:#x}")]
	BootStageSize { stage: StageName, declared: usize, actual: usize },
	#[error("boot stage sizes overflow the host address space")]
	BootChainSizeOverflow,
	#[error("boot chain ends at {end:#x}, past the boot-region limit {limit:#x}")]
	BootRegionOverflow { end: usize, limit: usize },
	#[error(
		"replacement boot stage {stage} is {size:#x} bytes; the next stage starts at its end rounded up to 0x10, so it must end on that boundary"
	)]
	UnalignedBootStage { stage: StageName, size: usize },
	#[error(
		"replacement boot stage {actual} has another console key letter than the base's {expected}, so the chain would become another kind"
	)]
	FirstStageKindChanged { expected: StageName, actual: StageName },
	#[error(
		"replacement boot stage {stage} changes the split flag 0x0800 of the base stage, which decides whether a CB_B follows"
	)]
	SplitFlagChanged { stage: StageName },
	#[error("the compiled image does not parse as a boot chain")]
	OutputChainUnparsable {
		#[source]
		source: sfcx::ParseError,
	},
	#[error("the compiled image's boot chain is not the one the compiler placed: expected {expected}; got {actual}")]
	OutputChainMismatch { expected: ChainLayout, actual: ChainLayout },
	#[error(
		"the changed \"SE\" needs a new digest in \"SD\", but the base \"SD\" does not keep the digest of the base \"SE\" at 0x24c; supply sd.bin"
	)]
	SdKeepsNoSeDigest,
	#[error("the flash header entry point {entry:#x} is not inside the {size:#x}-byte image")]
	EntryPastImage { entry: u64, size: u64 },
	#[error("boot stage \"SB\" enters at {entry:#x}, not before the end of the {size:#x}-byte image")]
	SbEntryPastImage { entry: u64, size: u64 },
	#[error("boot stage \"SB\" ends at {end:#x}, not before the end of the {size:#x}-byte image")]
	SbEndPastImage { end: u64, size: u64 },
	#[error("the boot chain starts at {entry:#x}, inside the copyright text of the flash header")]
	CopyrightOverlapsChain { entry: usize },
	#[error(
		"boot stage \"SE\" (build {build}) has a zero nonce and is not an encrypted hypervisor and kernel payload; it is likely stored unencrypted, as XeLL stores its loader"
	)]
	UnencryptedSe {
		build: u16,
		#[source]
		source: se::Error,
	},
}

/// Size of the shadowboot image compiled from `region`: its boot region, which is the whole
/// image when the flash header declares no boot size.
pub fn output_size(region: &BootRegion<'_>) -> Result<usize, Error> {
	let size = region.bytes().len();
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
	let (report, planned) = match region.chain() {
		BootChain::Devkit { .. } => apply_boot_layer(region, output, *layer)?,
		BootChain::Retail { .. } if layer.ignore_retail_checks => retail::apply_layer(region, output, layer)?,
		BootChain::Retail { .. } => return Err(retail_base(region)),
	};
	if let Some(copyright) = layer.copyright {
		let entry = region.boot_chain_offset().get();
		if entry < sfcx::boot::FLASH_HEADER_SIZE {
			return Err(Error::CopyrightOverlapsChain { entry });
		}
		copyright.write(output);
	}
	check_loadable(output)?;

	verify_chain(output, planned)?;

	Ok(report)
}

/// The finished image must parse to the chain the compiler placed, so that no stage is
/// dispatched under another role than intended.
fn verify_chain(output: &[u8], planned: ChainLayout) -> Result<(), Error> {
	let compiled = BootRegion::parse(output).map_err(|source| Error::OutputChainUnparsable { source })?;
	let actual = ChainLayout::of(&compiled);
	if actual != planned {
		return Err(Error::OutputChainMismatch { expected: planned, actual });
	}

	Ok(())
}

/// Where a stage of a chain lies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageLayout {
	pub role: StageRole,
	pub offset: usize,
	pub size: usize,
}

/// The stages of a chain in flash order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainLayout(pub Vec<StageLayout>);

impl ChainLayout {
	fn of(region: &BootRegion<'_>) -> Self {
		let stages = region.chain().stages();
		Self(
			stages
				.map(|(role, stage)| StageLayout { role, offset: stage.offset().get(), size: stage.size().get() })
				.collect(),
		)
	}

	/// The layout of `pieces` written from `chain_offset`: each stage of `roles` followed by
	/// the bytes up to the next stage.
	fn planned(chain_offset: usize, roles: &[StageRole], pieces: &[&[u8]]) -> Self {
		let mut offset = chain_offset;
		let mut stages = Vec::with_capacity(roles.len());
		for (role, pair) in roles.iter().zip(pieces.chunks(2)) {
			stages.push(StageLayout { role: *role, offset, size: pair[0].len() });
			offset += pair.iter().map(|piece| piece.len()).sum::<usize>();
		}

		Self(stages)
	}
}

impl fmt::Display for ChainLayout {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		for (index, stage) in self.0.iter().enumerate() {
			let separator = if index == 0 { "" } else { ", " };
			write!(formatter, "{separator}{} at {:#x} ({:#x} bytes)", stage.role, stage.offset, stage.size)?;
		}

		Ok(())
	}
}

/// Checks what `HvpShadowBootStart` checks before it verifies SB (openxbox
/// hv/src/shadowboot/shadowboot.s, around lines 168-184): the entry point lies inside the
/// image, and SB's entry point and end lie strictly before the image end. The size it checks
/// against is the file size that the kernel passes (kernel/src/init/init.c, around line 392);
/// the header's boot size field is never read on this path.
fn check_loadable(image: &[u8]) -> Result<(), Error> {
	let word = |at: usize| u32::from_be_bytes(image[at..at + 4].try_into().expect("a u32 field")) as u64;
	let size = image.len() as u64;
	let entry = word(0x08);
	if entry >= size {
		return Err(Error::EntryPastImage { entry, size });
	}
	let sb = entry as usize;
	let sb_entry = entry + word(sb + 0x08);
	if sb_entry >= size {
		return Err(Error::SbEntryPastImage { entry: sb_entry, size });
	}
	let sb_end = entry + word(sb + 0x0c);
	if sb_end >= size {
		return Err(Error::SbEndPastImage { end: sb_end, size });
	}

	Ok(())
}

/// The decompressed SE payload of `region`. A retail chain has none that can be extracted: it is
/// rejected, or with `ignore_retail_checks` reported as needing a devkit chain.
pub fn payload(region: &BootRegion<'_>, ignore_retail_checks: bool) -> Result<ExtractReport, Error> {
	if ignore_retail_checks && region.chain().kind() == ChainKind::Retail {
		return Err(Error::RequiresDevkitChain { operation: DevkitOperation::Extract });
	}
	let [_, sc_stage, sd_stage, se_stage] = devkit_stages(region)?;
	let sc = region.boot_stage_data(&sc_stage);
	let sd = region.boot_stage_data(&sd_stage);
	let se = region.boot_stage_data(&se_stage);
	let base = check_base_sd(region, &se_stage, sc, sd, se)?;
	let payload = se::decode(sc, sd, se).map_err(|source| {
		// XeLL stores SE unencrypted, with a zero nonce, as code rather than a payload.
		let nonce = &se[se::NONCE_OFFSET..se::NONCE_OFFSET + se::NONCE_SIZE];
		if nonce.iter().all(|byte| *byte == 0) {
			Error::UnencryptedSe { build: se_stage.build().get(), source }
		} else {
			Error::Se(source)
		}
	})?;

	Ok(ExtractReport { payload, warnings: base.warnings })
}

/// The SB, SC, SD and SE stages of a devkit base. A retail chain cannot be a base.
fn devkit_stages(region: &BootRegion<'_>) -> Result<[BootStage; 4], Error> {
	match *region.chain() {
		BootChain::Devkit { sb, sc, sd, se } => Ok([sb, sc, sd, se]),
		BootChain::Retail { .. } => Err(retail_base(region)),
	}
}

/// The error for a retail base, naming its first stage.
fn retail_base(region: &BootRegion<'_>) -> Error {
	let (_, first) = region.chain().stages().next().expect("a chain has stages");
	Error::RetailBase { offset: first.offset().get(), name: first.name() }
}

/// The bytes of `region` that follow `stage` up to its end rounded up to 16 bytes, where the
/// next stage starts. After SE they are the SE padding.
fn base_padding<'a>(region: &BootRegion<'a>, stage: &BootStage) -> Result<&'a [u8], Error> {
	let end = stage.offset().get() + stage.size().get();
	let aligned = stage.aligned_end();

	region.bytes().get(end..aligned).ok_or(Error::BootRegionOverflow { end: aligned, limit: region.bytes().len() })
}

/// What the base SD keeps of the base SE.
struct BaseSd {
	warnings: Vec<Warning>,
	/// The base SD keeps the digest of the base SE at the offset where the compiler writes it.
	keeps_digest: bool,
}

/// Compares the SE digest that the base SD keeps with the base SE and checks the base SD
/// signature. A custom SD that is too short to keep the digest is reported, not rejected.
fn check_base_sd(
	region: &BootRegion<'_>,
	se_stage: &BootStage,
	sc: &[u8],
	sd: &[u8],
	se: &[u8],
) -> Result<BaseSd, Error> {
	let computed = se::digest(sc, sd, se, base_padding(region, se_stage)?)?;
	let stored = match se::sd_se_digest(sc, sd) {
		Ok(stored) => stored,
		Err(se::Error::SdTooShort { size }) => {
			return Ok(BaseSd { warnings: vec![Warning::BaseSdTooShort { size }], keeps_digest: false });
		}
		Err(error) => return Err(error.into()),
	};
	let mut warnings = Vec::new();
	if stored != computed {
		warnings.push(Warning::BaseSdDigest { stored, computed });
	}
	if !se::sd_signature_verifies(sc, sd, sc, sd)? {
		warnings.push(Warning::BaseSdSignature { digest: se::sd_signature_digest(sc, sd)? });
	}

	Ok(BaseSd { warnings, keeps_digest: stored == computed })
}

fn apply_boot_layer(
	region: &BootRegion<'_>,
	output: &mut [u8],
	layer: BootLayer<'_>,
) -> Result<(BuildReport, ChainLayout), Error> {
	let base_stages = devkit_stages(region)?;
	let base_sb = region.boot_stage_data(&base_stages[0]);
	let base_sc = region.boot_stage_data(&base_stages[1]);
	let base_sd = region.boot_stage_data(&base_stages[2]);
	let base_se = region.boot_stage_data(&base_stages[3]);
	let chain_offset = region.boot_chain_offset().get();
	for (role, _) in layer.stages {
		if !matches!(role, StageRole::Sb | StageRole::Sc | StageRole::Sd | StageRole::Se) {
			return Err(Error::LayerStageNotInChain { stage: *role, kind: ChainKind::Devkit });
		}
	}
	let layer_se = layer.stage(StageRole::Se);

	if layer_se.is_some() && (layer.hypervisor.is_some() || layer.kernel.is_some()) {
		return Err(Error::SeReplacementConflict);
	}
	if layer_se.is_some() && layer.rebuild_se {
		return Err(Error::SeRebuildConflict);
	}

	let sb = layer.stage(StageRole::Sb).unwrap_or(base_sb);
	let sc = layer.stage(StageRole::Sc).unwrap_or(base_sc);
	let layer_sd = layer.stage(StageRole::Sd);
	let sd = layer_sd.unwrap_or(base_sd);
	for (stage, base) in [sb, sc, sd].iter().zip(&base_stages) {
		validate_boot_stage(base.name(), stage)?;
	}
	let boot_limit = region.bytes().len();
	let selected_end = boot_chain_end(chain_offset, &[sb, sc, sd])?;
	if selected_end > boot_limit {
		return Err(Error::BootRegionOverflow { end: selected_end, limit: boot_limit });
	}
	if let Some(se) = layer_se {
		validate_boot_stage(base_stages[3].name(), se)?;
		let selected_end = boot_chain_end(chain_offset, &[sb, sc, sd, se])?;
		if selected_end > boot_limit {
			return Err(Error::BootRegionOverflow { end: selected_end, limit: boot_limit });
		}
	}

	// Each stage starts at the end of the one before it rounded up to 16 bytes, and the
	// hypervisor authenticates SB over that rounded length. A stage the layer leaves alone keeps
	// the base bytes up to there; a replaced stage must end on the boundary itself.
	let mut paddings: [&[u8]; 3] = [&[]; 3];
	for (index, (stage, base)) in [sb, sc, sd].into_iter().zip([base_sb, base_sc, base_sd]).enumerate() {
		paddings[index] = if stage == base {
			base_padding(region, &base_stages[index])?
		} else if stage.len().is_multiple_of(SE_ALIGNMENT) {
			&[]
		} else {
			return Err(Error::UnalignedBootStage { stage: base_stages[index].name(), size: stage.len() });
		};
	}

	// A bound SB halts on every console but the one that bound it.
	let unbound_sb = if !layer.bind && sb::is_bound(sb)? { Some(sb::unbound(sb)?) } else { None };
	let sb = unbound_sb.as_deref().unwrap_or(sb);
	let base_sd_check = check_base_sd(region, &base_stages[3], base_sc, base_sd, base_se)?;
	let mut warnings = base_sd_check.warnings;

	let base_sc_state = se::sc_key_state(base_sc)?;
	let sc_state = se::sc_key_state(sc)?;
	if sc_state != base_sc_state {
		if layer_sd.is_none() {
			return Err(Error::BootStageDependency { stage: StageName::SC, required: StageName::SD });
		}
		if sd == base_sd {
			return Err(Error::StaleBootStage { stage: StageName::SD, upstream: StageName::SC });
		}
	}
	let se_key_changed = se::sd_key_state(sc, sd)? != se::sd_key_state(base_sc, base_sd)?;

	let mut rebuilt_se = None;
	let se = if let Some(se) = layer_se {
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
	validate_boot_stage(base_stages[3].name(), se)?;

	// Flash holds SE up to its aligned end; the base keeps its own bytes there.
	let padding = if se == base_se && !se_key_changed {
		base_padding(region, &base_stages[3])?.to_vec()
	} else {
		se::padding(sc, sd, se)?
	};

	// SD authenticates the SE digest. A layer SD must already authenticate it. A changed SE
	// updates and signs the base SD, which must keep the digest of the base SE; an unchanged SE
	// leaves the base SD as it is.
	let se_changed = se != base_se || se_key_changed;
	let mut updated_sd = None;
	if layer_sd.is_some() {
		let digest = se::digest(sc, sd, se, &padding)?;
		let stored = se::sd_se_digest(sc, sd)?;
		if stored != digest {
			warnings.push(Warning::SdDigest { stored, computed: digest });
		}
	} else if se_changed {
		if !base_sd_check.keeps_digest {
			return Err(Error::SdKeepsNoSeDigest);
		}
		let digest = se::digest(sc, sd, se, &padding)?;
		updated_sd = Some(se::with_authenticated_sd_se_digest(sc, sd, &digest)?);
	}
	let sd = updated_sd.as_deref().unwrap_or(sd);
	// An unchanged SD was checked with the base.
	if sd != base_sd && !se::sd_signature_verifies(sc, sd, base_sc, base_sd)? {
		warnings.push(Warning::SdSignature { digest: se::sd_signature_digest(sc, sd)? });
	}

	let pieces = [sb, paddings[0], sc, paddings[1], sd, paddings[2], se, padding.as_slice()];
	let new_end = pieces
		.iter()
		.try_fold(chain_offset, |offset, piece| offset.checked_add(piece.len()))
		.ok_or(Error::BootChainSizeOverflow)?;
	if new_end > boot_limit {
		return Err(Error::BootRegionOverflow { end: new_end, limit: boot_limit });
	}

	let replaced = layer.stages.len() + usize::from(rebuilt_se.is_some());
	if replaced != 0 || updated_sd.is_some() {
		let old_end = base_stages[3].aligned_end().min(boot_limit);
		output[chain_offset..old_end.max(new_end)].fill(0);
		let mut offset = chain_offset;
		for piece in pieces {
			output[offset..offset + piece.len()].copy_from_slice(piece);
			offset += piece.len();
		}
	} else if unbound_sb.is_some() {
		// Only the binding changes; every other byte stays where it is.
		let binding = chain_offset + sb::BINDING.start..chain_offset + sb::BINDING.end;
		output[binding].copy_from_slice(&sb[sb::BINDING]);
	}

	let roles = [StageRole::Sb, StageRole::Sc, StageRole::Sd, StageRole::Se];
	let report = BuildReport {
		replaced_boot_stages: replaced,
		rebuilt_se: rebuilt_se.is_some(),
		updated_sd_signature: updated_sd.is_some(),
		binding: if unbound_sb.is_some() { Binding::Cleared } else { Binding::Unchanged },
		warnings,
	};

	Ok((report, ChainLayout::planned(chain_offset, &roles, &pieces)))
}

/// End of a chain of `stages` from `chain_offset`, each starting at the end of the one before it
/// rounded up to 16 bytes.
fn boot_chain_end(chain_offset: usize, stages: &[&[u8]]) -> Result<usize, Error> {
	stages.iter().try_fold(chain_offset, |offset, stage| {
		offset.checked_add(stage.len().next_multiple_of(SE_ALIGNMENT)).ok_or(Error::BootChainSizeOverflow)
	})
}

/// Checks a stage input against the base stage it replaces, named `stage`: the same name, and a
/// declared size that is the input's size. Names are never rewritten, since the next stage's
/// digest and the signatures cover them.
fn validate_boot_stage(stage: StageName, bytes: &[u8]) -> Result<(), Error> {
	if bytes.len() < 0x10 {
		return Err(Error::BootStageHeaderTruncated { stage });
	}
	let actual = StageName::new([bytes[0], bytes[1]]);
	if actual != stage {
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

#[cfg(test)]
mod tests {
	use std::fs::File;

	use memmap2::Mmap;

	use super::*;

	/// A retail base, whose chain starts with "CB", is rejected before anything is compiled, and
	/// its payload cannot be extracted.
	#[test]
	fn retail_base_is_rejected() {
		let base = retail_region();
		let region = BootRegion::parse(&base).unwrap();
		let mut output = vec![0u8; output_size(&region).unwrap()];

		let error = compile(&region, &mut output, &BootLayer::default()).unwrap_err();
		insta::assert_snapshot!(error.to_string());
		assert!(matches!(payload(&region, false), Err(Error::RetailBase { offset: 0x8000, name: StageName::CB })));
		assert!(matches!(
			payload(&region, true),
			Err(Error::RequiresDevkitChain { operation: DevkitOperation::Extract })
		));
	}

	/// With retail checks ignored, a retail base is copied with a warning, whole stages of its
	/// own roles replace those of the base in place, and the copyright text applies. The CB
	/// binding is left alone.
	#[test]
	fn ignored_retail_checks_allow_whole_stage_replacements() {
		let base = retail_region();
		let region = BootRegion::parse(&base).unwrap();
		let mut output = vec![0u8; output_size(&region).unwrap()];

		let empty = BootLayer { ignore_retail_checks: true, ..BootLayer::default() };
		let report = compile(&region, &mut output, &empty).unwrap();
		assert!(output == base);
		assert_eq!(report.binding, Binding::RetailCb);
		assert_eq!(report.warnings, [Warning::RetailBase { offset: 0x8000, name: StageName::CB }]);
		insta::assert_snapshot!(report.warnings[0].to_string());

		let cd = region.chain().stage(StageRole::Cd).unwrap();
		let mut new_cd = region.boot_stage_data(&cd).to_vec();
		new_cd[0x100..].fill(0x77);
		let copyright: Copyright = "retail".parse().unwrap();
		let stages = [(StageRole::Cd, new_cd.as_slice())];
		let layer = BootLayer { stages: &stages, copyright: Some(&copyright), ..empty };
		let report = compile(&region, &mut output, &layer).unwrap();
		assert_eq!(report.replaced_boot_stages, 1);
		let cd_range = cd.offset().get()..cd.offset().get() + cd.size().get();
		assert!(output[cd_range.clone()] == new_cd[..]);
		assert!(output[0x80..cd_range.start] == base[0x80..cd_range.start]);
		assert!(output[cd_range.end..] == base[cd_range.end..]);
		assert_eq!(&output[0x10..0x17], b"retail\0");
	}

	/// With retail checks ignored, whatever needs the devkit key chain, a stage of another
	/// role, a stage of the wrong loader, and an unaligned replacement are still rejected.
	#[test]
	fn ignored_retail_checks_reject_devkit_changes() {
		let base = retail_region();
		let region = BootRegion::parse(&base).unwrap();
		let mut output = vec![0u8; output_size(&region).unwrap()];
		let retail = BootLayer { ignore_retail_checks: true, ..BootLayer::default() };
		let mut compile_with = |layer: BootLayer<'_>| compile(&region, &mut output, &layer).unwrap_err();

		let hypervisor = [0u8; 0x10];
		let errors = [
			compile_with(BootLayer { hypervisor: Some(&hypervisor), ..retail }),
			compile_with(BootLayer { kernel: Some(&hypervisor), ..retail }),
			compile_with(BootLayer { rebuild_se: true, ..retail }),
		];
		let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
		insta::assert_snapshot!(messages.join("\n"));

		let sd = stage(*b"SD", 0, 0x400);
		let error = compile_with(BootLayer { stages: &[(StageRole::Sd, &sd)], ..retail });
		assert!(matches!(error, Error::LayerStageNotInChain { stage: StageRole::Sd, kind: ChainKind::Retail }));
		let wrong = stage(*b"CD", 0, 0x400);
		let error = compile_with(BootLayer { stages: &[(StageRole::CbB, &wrong)], ..retail });
		assert!(matches!(error, Error::BootStageMagic { expected: StageName::CB, actual: StageName::CD }));
		let ce = stage(*b"CE", 0, 0x408);
		let error = compile_with(BootLayer { stages: &[(StageRole::Ce, &ce)], ..retail });
		assert!(matches!(error, Error::UnalignedBootStage { stage: StageName::CE, size: 0x408 }));
	}

	/// A replacement CB_A must keep the retail console key letter and the base's split flag;
	/// either change would make the later stages parse under other roles.
	#[test]
	fn ignored_retail_checks_keep_the_chain_shape() {
		let base = retail_region();
		let region = BootRegion::parse(&base).unwrap();
		let mut output = vec![0u8; output_size(&region).unwrap()];
		let cb_a = region.boot_stage_data(&region.chain().stage(StageRole::CbA).unwrap());
		let retail = BootLayer { ignore_retail_checks: true, ..BootLayer::default() };

		let mut unsplit = cb_a.to_vec();
		unsplit[6..8].copy_from_slice(&0u16.to_be_bytes());
		let error = compile(&region, &mut output, &BootLayer { stages: &[(StageRole::CbA, &unsplit)], ..retail });
		assert!(matches!(error, Err(Error::SplitFlagChanged { stage: StageName::CB })));

		let mut devkit = cb_a.to_vec();
		devkit[0] = b'S';
		let error = compile(&region, &mut output, &BootLayer { stages: &[(StageRole::CbA, &devkit)], ..retail });
		assert!(matches!(error, Err(Error::FirstStageKindChanged { expected: StageName::CB, actual: StageName::SB })));

		let mut same_shape = cb_a.to_vec();
		same_shape[0x200] ^= 0xff;
		compile(&region, &mut output, &BootLayer { stages: &[(StageRole::CbA, &same_shape)], ..retail }).unwrap();
		let compiled = BootRegion::parse(&output).unwrap();
		assert_eq!(ChainLayout::of(&compiled), ChainLayout::of(&region));
	}

	/// The compiled image is reparsed: a chain other than the planned one, or no chain at all,
	/// is an error.
	#[test]
	fn compiled_chain_must_match_the_plan() {
		let base = retail_region();
		let region = BootRegion::parse(&base).unwrap();
		let planned = ChainLayout::of(&region);
		assert!(verify_chain(&base, planned.clone()).is_ok());

		let mut other = planned.clone();
		other.0[1].role = StageRole::Cd;
		let error = verify_chain(&base, other).unwrap_err();
		insta::assert_snapshot!(error.to_string());

		let mut broken = base.clone();
		broken[0x8000] = b'X';
		assert!(matches!(verify_chain(&broken, planned), Err(Error::OutputChainUnparsable { .. })));
	}

	/// A retail flash header and a chain of CB_A, CB_B, CD and CE from 0x8000. CE is not a whole
	/// number of 16-byte blocks; the bytes after it are 0xee.
	fn retail_region() -> Vec<u8> {
		let mut base = vec![0u8; 0xa000];
		base[..2].copy_from_slice(&[0xff, 0x4f]);
		base[8..12].copy_from_slice(&0x8000u32.to_be_bytes());
		base[12..16].copy_from_slice(&0xa000u32.to_be_bytes());
		let stages =
			[stage(*b"CB", 0x0800, 0x400), stage(*b"CB", 0, 0x400), stage(*b"CD", 0, 0x400), stage(*b"CE", 0, 0x404)];
		let mut offset = 0x8000;
		for stage in stages {
			base[offset..offset + stage.len()].copy_from_slice(&stage);
			let end = offset + stage.len();
			offset = end.next_multiple_of(16);
			base[end..offset].fill(0xee);
		}
		base
	}

	/// A stage named `name` with `flags`, `size` bytes long, filled with a pattern, entered at
	/// 0x20.
	fn stage(name: [u8; 2], flags: u16, size: usize) -> Vec<u8> {
		let mut stage: Vec<u8> = (0..size).map(|index| index as u8).collect();
		stage[..2].copy_from_slice(&name);
		stage[6..8].copy_from_slice(&flags.to_be_bytes());
		stage[0x08..0x0c].copy_from_slice(&0x20u32.to_be_bytes());
		stage[0x0c..0x10].copy_from_slice(&(size as u32).to_be_bytes());
		stage
	}

	/// A compact ROM: the chain right after the 0x80-byte flash header, no SMC, and no declared
	/// boot size, so the boot region is the whole image. Its SD does not keep the SE digest:
	/// an empty layer reproduces the image with a warning, and a changed SE cannot be placed.
	#[test]
	fn compact_rom_with_custom_sd_compiles_unchanged() {
		let rom = compact_rom();
		let region = BootRegion::parse(&rom).unwrap();
		assert_eq!(region.bytes().len(), rom.len());
		assert_eq!(region.smc(), Ok(None));
		let mut output = vec![0u8; output_size(&region).unwrap()];

		let report = compile(&region, &mut output, &BootLayer::default()).unwrap();
		assert!(output == rom);
		assert!(
			report.binding == Binding::Unchanged && !report.updated_sd_signature && report.replaced_boot_stages == 0
		);
		assert!(matches!(report.warnings.as_slice(), [Warning::BaseSdDigest { .. }, Warning::BaseSdSignature { .. }]));
		assert!(payload(&region, false).is_ok());

		// The encoder reproduces the base SE, so only a changed kernel changes SE.
		let rebuild = BootLayer { rebuild_se: true, ..BootLayer::default() };
		assert!(!compile(&region, &mut output, &rebuild).unwrap().updated_sd_signature);
		let kernel = vec![0x5a; 0x7000];
		let changed = BootLayer { kernel: Some(&kernel), ..BootLayer::default() };
		let error = compile(&region, &mut output, &changed).unwrap_err();
		assert!(matches!(error, Error::SdKeepsNoSeDigest));
	}

	/// The copyright text replaces 0x10..0x60 of the flash header, NUL-terminated and zero
	/// filled, and leaves the word at 0x60 and every other byte as they were.
	#[test]
	fn copyright_replaces_the_header_text() {
		let mut rom = compact_rom();
		rom[0x10..0x60].fill(b'x');
		rom[0x60..0x64].copy_from_slice(&0x4000u32.to_be_bytes());
		let region = BootRegion::parse(&rom).unwrap();
		let copyright: Copyright = "\u{a9} 2026 Shadowboot".parse().unwrap();
		let mut output = vec![0u8; output_size(&region).unwrap()];

		compile(&region, &mut output, &BootLayer { copyright: Some(&copyright), ..BootLayer::default() }).unwrap();
		assert!(output[0x80..] == rom[0x80..]);
		let lines: Vec<String> = output[..0x80]
			.chunks(16)
			.enumerate()
			.map(|(index, line)| {
				let bytes: Vec<String> = line.iter().map(|byte| format!("{byte:02x}")).collect();
				format!("{:#04x}: {}", index * 16, bytes.join(" "))
			})
			.collect();
		insta::assert_snapshot!(lines.join("\n"));
	}

	/// The conditions the hypervisor checks before it verifies SB.
	#[test]
	fn image_must_hold_the_entry_point_and_sb() {
		let image = |size: usize, entry: u32, sb_entry: u32, sb_size: u32| {
			let mut image = vec![0u8; size];
			image[0x08..0x0c].copy_from_slice(&entry.to_be_bytes());
			let sb = entry as usize;
			if sb + 0x10 <= size {
				image[sb + 0x08..sb + 0x0c].copy_from_slice(&sb_entry.to_be_bytes());
				image[sb + 0x0c..sb + 0x10].copy_from_slice(&sb_size.to_be_bytes());
			}
			image
		};

		assert!(check_loadable(&image(0x1000, 0x80, 0x3c0, 0x400)).is_ok());
		let errors = [
			check_loadable(&image(0x1000, 0x1000, 0x3c0, 0x400)).unwrap_err(),
			check_loadable(&image(0x1000, 0x80, 0xf80, 0x400)).unwrap_err(),
			check_loadable(&image(0x1000, 0x80, 0x3c0, 0xf80)).unwrap_err(),
		];
		insta::assert_snapshot!(errors.map(|error| error.to_string()).join("\n"));
	}

	/// An SE with a zero nonce that does not decode names what it likely is.
	#[test]
	fn unencrypted_se_fails_extraction() {
		let mut rom = compact_rom();
		let region = BootRegion::parse(&rom).unwrap();
		let se = devkit_stages(&region).unwrap()[3];
		let start = se.offset().get();
		rom[start + 0x10..start + se.size().get()].fill(0);
		let region = BootRegion::parse(&rom).unwrap();

		let error = payload(&region, false).unwrap_err();
		assert!(matches!(error, Error::UnencryptedSe { build: 1337, .. }));
		let mut output = vec![0u8; output_size(&region).unwrap()];
		assert!(compile(&region, &mut output, &BootLayer::default()).is_ok());
		assert!(output == rom);
	}

	/// Runs against a bound devkit image named by `SHADOWBOOT_BASE_IMAGE`, such as a 60 MiB
	/// devkit flash or a ROM cut from one. With `bind`, unchanged layers reproduce it.
	#[test]
	#[ignore = "requires SHADOWBOOT_BASE_IMAGE"]
	fn unchanged_layers_reproduce_base_boot_region() {
		let base = image("SHADOWBOOT_BASE_IMAGE");
		let region = BootRegion::parse(&base).unwrap();
		let size = region.bytes().len();
		let stages = devkit_stages(&region).unwrap();
		let stage = |index: usize| region.boot_stage_data(&stages[index]);
		let payload = se::decode(stage(1), stage(2), stage(3)).unwrap();
		let all_stages = [
			(StageRole::Sb, stage(0)),
			(StageRole::Sc, stage(1)),
			(StageRole::Sd, stage(2)),
			(StageRole::Se, stage(3)),
		];

		let layers = [
			BootLayer::default(),
			BootLayer { stages: &all_stages, ..BootLayer::default() },
			BootLayer {
				hypervisor: Some(payload.hypervisor()),
				kernel: Some(payload.kernel()),
				..BootLayer::default()
			},
		];
		let mut output = vec![0u8; output_size(&region).unwrap()];
		for layer in &layers {
			output.fill(0);
			let report = compile(&region, &mut output, &BootLayer { bind: true, ..*layer }).unwrap();
			assert_eq!(report.warnings, []);
			assert!(report.binding == Binding::Unchanged);
			assert_eq!(output.len(), size);
			assert!(output == base[..size]);
		}
	}

	/// Runs against a bound devkit image named by `SHADOWBOOT_BASE_IMAGE` and the unbound
	/// RGLoader ROM named by `SHADOWBOOT_UNBOUND_IMAGE`. By default the SB binding is cleared:
	/// the SB plaintext changes only at 0x20..0x40, which becomes zero, and then matches the
	/// RGLoader SB apart from the nonce. Every other byte, SC, SD and SE included, stays.
	#[test]
	#[ignore = "requires SHADOWBOOT_BASE_IMAGE and SHADOWBOOT_UNBOUND_IMAGE"]
	fn bound_sb_is_unbound_by_default() {
		let base = image("SHADOWBOOT_BASE_IMAGE");
		let region = BootRegion::parse(&base).unwrap();
		let sb_stage = devkit_stages(&region).unwrap()[0];
		let sb_range = sb_stage.offset().get()..sb_stage.offset().get() + sb_stage.size().get();
		let mut output = vec![0u8; output_size(&region).unwrap()];

		let report = compile(&region, &mut output, &BootLayer::default()).unwrap();
		assert!(report.binding == Binding::Cleared);
		assert_eq!(report.warnings, []);
		let before = sb::plaintext(&base[sb_range.clone()]);
		let after = sb::plaintext(&output[sb_range.clone()]);
		assert!(after[0x20..0x40].iter().all(|byte| *byte == 0));
		assert!(before[0x20..0x40].iter().any(|byte| *byte != 0));
		assert_eq!(before[..0x20], after[..0x20]);
		assert!(before[0x40..] == after[0x40..]);
		assert!(output[..sb_range.start] == base[..sb_range.start]);
		assert!(output[sb_range.end..] == base[sb_range.end..output.len()]);

		let rgloader = image("SHADOWBOOT_UNBOUND_IMAGE");
		let rgloader_region = BootRegion::parse(&rgloader).unwrap();
		let rgloader_sb = rgloader_region.boot_stage_data(&devkit_stages(&rgloader_region).unwrap()[0]);
		let rgloader_plaintext = sb::plaintext(rgloader_sb);
		assert_eq!(after[..0x10], rgloader_plaintext[..0x10]);
		assert!(after[0x20..] == rgloader_plaintext[0x20..]);
	}

	/// Runs against the shadowboot ROMs named by `SHADOWBOOT_UNBOUND_IMAGE` (RGLoader),
	/// `SHADOWBOOT_FREEBOOT_IMAGE` and `SHADOWBOOT_XELL_IMAGE`. Their SBs are unbound, so an
	/// empty layer reproduces each byte for byte. The freeboot SE decodes to a hypervisor and
	/// kernel; the XeLL SE is unencrypted code.
	#[test]
	#[ignore = "requires SHADOWBOOT_UNBOUND_IMAGE, SHADOWBOOT_FREEBOOT_IMAGE and SHADOWBOOT_XELL_IMAGE"]
	fn unbound_roms_compile_unchanged() {
		for variable in ["SHADOWBOOT_UNBOUND_IMAGE", "SHADOWBOOT_FREEBOOT_IMAGE", "SHADOWBOOT_XELL_IMAGE"] {
			let rom = image(variable);
			let region = BootRegion::parse(&rom).unwrap();
			let mut output = vec![0u8; output_size(&region).unwrap()];
			let report = compile(&region, &mut output, &BootLayer::default()).unwrap();
			assert!(report.binding == Binding::Unchanged, "{variable}");
			assert!(output == rom[..], "{variable}");

			let extracted = payload(&region, false);
			match variable {
				"SHADOWBOOT_XELL_IMAGE" => assert!(matches!(extracted, Err(Error::UnencryptedSe { build: 1337, .. }))),
				_ => assert!(extracted.is_ok(), "{variable}"),
			}
		}
	}

	/// Runs against a private 60 MiB devkit flash capture named by `SHADOWBOOT_BASE_IMAGE`. A
	/// recompressed SE and a changed kernel each decompress to their payload, and the compiled
	/// SD authenticates the new SE digest. The compiled boot region is a base image itself.
	#[test]
	#[ignore = "requires SHADOWBOOT_BASE_IMAGE"]
	fn rebuilt_se_round_trips_with_authenticated_sd() {
		let base = image("SHADOWBOOT_BASE_IMAGE");
		let region = BootRegion::parse(&base).unwrap();
		let stages = devkit_stages(&region).unwrap();
		let stage = |index: usize| region.boot_stage_data(&stages[index]);
		let stock = payload(&region, false).unwrap().payload;

		let recompressed = se::encode(stage(1), stage(2), stage(3), &stock).unwrap();
		assert_ne!(recompressed.as_slice(), stage(3));
		let mut kernel = stock.kernel().to_vec();
		kernel[0x1000] ^= 0xff;
		let changed = Payload::parse([stock.hypervisor(), kernel.as_slice()].concat()).unwrap();

		let cases = [
			(BootLayer { rebuild_se: true, ..BootLayer::default() }, &stock),
			(BootLayer { stages: &[(StageRole::Se, &recompressed)], ..BootLayer::default() }, &stock),
			(BootLayer { kernel: Some(&kernel), ..BootLayer::default() }, &changed),
		];
		let mut output = vec![0u8; output_size(&region).unwrap()];
		let conflict = BootLayer { stages: &[(StageRole::Se, stage(3))], rebuild_se: true, ..BootLayer::default() };
		assert!(matches!(compile(&region, &mut output, &conflict), Err(Error::SeRebuildConflict)));
		for (layer, expected) in &cases {
			let report = compile(&region, &mut output, layer).unwrap();
			assert!(report.updated_sd_signature);
			assert_eq!(report.warnings, []);
			let compiled = BootRegion::parse(&output).unwrap();
			let extracted = payload(&compiled, false).unwrap();
			assert_eq!(extracted.warnings, []);
			assert!(extracted.payload == **expected);
			assert!(compiled.boot_stage_data(&devkit_stages(&compiled).unwrap()[1]) == stage(1));
		}
	}

	/// Runs against a private 60 MiB devkit flash capture named by `SHADOWBOOT_BASE_IMAGE`. An SD
	/// whose signature does not verify is written with a warning, reported once.
	#[test]
	#[ignore = "requires SHADOWBOOT_BASE_IMAGE"]
	fn unverified_sd_signature_warns() {
		let base = image("SHADOWBOOT_BASE_IMAGE");
		let region = BootRegion::parse(&base).unwrap();
		let sd_stage = devkit_stages(&region).unwrap()[2];
		// RC4 flips the plaintext byte under a flipped ciphertext byte. 0x300 is signed and is
		// not the SE digest.
		let flipped = sd_stage.offset().get() + 0x300;
		let mut sd = region.boot_stage_data(&sd_stage).to_vec();
		sd[0x300] ^= 0x01;
		let mut output = vec![0u8; output_size(&region).unwrap()];

		let layer = BootLayer { stages: &[(StageRole::Sd, &sd)], ..BootLayer::default() };
		let report = compile(&region, &mut output, &layer).unwrap();
		assert!(matches!(report.warnings.as_slice(), [Warning::SdSignature { .. }]));
		assert!(output[flipped] != base[flipped]);

		let mut tampered = region.bytes().to_vec();
		tampered[flipped] ^= 0x01;
		let tampered = BootRegion::parse(&tampered).unwrap();
		assert!(matches!(payload(&tampered, false).unwrap().warnings.as_slice(), [Warning::BaseSdSignature { .. }]));
		let report = compile(&tampered, &mut output, &BootLayer::default()).unwrap();
		assert!(matches!(report.warnings.as_slice(), [Warning::BaseSdSignature { .. }]));
	}

	/// Runs against a private 60 MiB devkit flash capture named by `SHADOWBOOT_BASE_IMAGE`. An SD
	/// that keeps another SE digest is reported, not rejected. A base SD with a wrong digest is
	/// written unchanged when SE is unchanged, and a changed SE cannot be authenticated by it.
	#[test]
	#[ignore = "requires SHADOWBOOT_BASE_IMAGE"]
	fn mismatched_sd_digest_warns() {
		const SD_SE_DIGEST: usize = 0x24c;

		let base = image("SHADOWBOOT_BASE_IMAGE");
		let region = BootRegion::parse(&base).unwrap();
		let sd_stage = devkit_stages(&region).unwrap()[2];
		let flipped = sd_stage.offset().get() + SD_SE_DIGEST;
		let mut sd = region.boot_stage_data(&sd_stage).to_vec();
		sd[SD_SE_DIGEST] ^= 0x01;
		let mut output = vec![0u8; output_size(&region).unwrap()];

		let layer = BootLayer { stages: &[(StageRole::Sd, &sd)], ..BootLayer::default() };
		let report = compile(&region, &mut output, &layer).unwrap();
		assert!(matches!(report.warnings.as_slice(), [Warning::SdDigest { .. }, Warning::SdSignature { .. }]));

		let mut tampered = region.bytes().to_vec();
		tampered[flipped] ^= 0x01;
		let tampered_region = BootRegion::parse(&tampered).unwrap();
		let extracted = payload(&tampered_region, false).unwrap();
		assert!(matches!(
			extracted.warnings.as_slice(),
			[Warning::BaseSdDigest { .. }, Warning::BaseSdSignature { .. }]
		));
		let bind = BootLayer { bind: true, ..BootLayer::default() };
		let report = compile(&tampered_region, &mut output, &bind).unwrap();
		assert!(!report.updated_sd_signature);
		assert!(matches!(report.warnings.as_slice(), [Warning::BaseSdDigest { .. }, Warning::BaseSdSignature { .. }]));
		assert!(output == tampered);
		let rebuild = BootLayer { rebuild_se: true, ..BootLayer::default() };
		assert!(matches!(compile(&tampered_region, &mut output, &rebuild), Err(Error::SdKeepsNoSeDigest)));
	}

	/// Stages whose sizes are not multiples of 16 keep their places and the bytes after them:
	/// unbinding changes only the binding, and the compiled chain still parses. A replacement
	/// stage that does not end on the boundary is rejected.
	#[test]
	fn unaligned_stages_keep_their_places() {
		let rom = rom([0x104, 0x108, 0x30c], true);
		let region = BootRegion::parse(&rom).unwrap();
		let offsets: Vec<usize> = devkit_stages(&region).unwrap().iter().map(|stage| stage.offset().get()).collect();
		assert_eq!(offsets[..3], [0x80, 0x190, 0x2a0]);
		let mut output = vec![0u8; output_size(&region).unwrap()];

		let report = compile(&region, &mut output, &BootLayer::default()).unwrap();
		assert!(report.binding == Binding::Cleared);
		assert!(output[..0xa0] == rom[..0xa0]);
		assert!(output[0xc0..] == rom[0xc0..]);
		let compiled = BootRegion::parse(&output).unwrap();
		assert!(!sb::is_bound(compiled.boot_stage_data(&devkit_stages(&compiled).unwrap()[0])).unwrap());

		let sb_stage = devkit_stages(&region).unwrap()[0];
		let same = [(StageRole::Sb, region.boot_stage_data(&sb_stage))];
		let same_sb = BootLayer { stages: &same, bind: true, ..BootLayer::default() };
		compile(&region, &mut output, &same_sb).unwrap();
		assert!(output == rom);

		let sc_stage = devkit_stages(&region).unwrap()[1];
		let mut sc = region.boot_stage_data(&sc_stage).to_vec();
		sc[0x100] ^= 0x01;
		let layer = BootLayer { stages: &[(StageRole::Sc, &sc)], ..BootLayer::default() };
		let error = compile(&region, &mut output, &layer).unwrap_err();
		assert!(matches!(error, Error::UnalignedBootStage { stage: StageName::SC, size: 0x108 }));
	}

	/// A flash header of 0x80 bytes without boot size or SMC, then SB, SC, SD and SE. SD is
	/// 0x300 bytes and keeps a zero SE digest; SE holds a small payload.
	fn compact_rom() -> Vec<u8> {
		rom([0x100, 0x100, 0x300], false)
	}

	/// A compact ROM with SB, SC and SD of `sizes`. Each stage starts at the end of the one
	/// before it rounded up to 16 bytes; the bytes in between are 0xee. A bound SB keeps a
	/// binding.
	fn rom(sizes: [usize; 3], bound: bool) -> Vec<u8> {
		let stage = |name: [u8; 2], build: u16, size: usize, nonce: u8| {
			let mut stage = vec![0u8; size];
			stage[..2].copy_from_slice(&name);
			stage[2..4].copy_from_slice(&build.to_be_bytes());
			stage[0x0c..0x10].copy_from_slice(&(size as u32).to_be_bytes());
			stage[0x10..0x20].fill(nonce);
			stage
		};
		let sb = stage(*b"SB", 14352, sizes[0], 0x11);
		let sb = if bound { sb } else { sb::unbound(&sb).unwrap() };
		let sc = stage(*b"SC", 17489, sizes[1], 0x22);
		let sd = stage(*b"SD", 1337, sizes[2], 0x33);
		let mut bytes = vec![0u8; 0x8000];
		bytes[se::HYPERVISOR_SIZE_FIELD].copy_from_slice(&0x1000u32.to_be_bytes());
		let payload = Payload::parse(bytes).unwrap();
		let se = se::encode(&sc, &sd, &stage(*b"SE", 1337, 0x30, 0x44), &payload).unwrap();
		let se = [se.as_slice(), &se::padding(&sc, &sd, &se).unwrap()].concat();

		let mut rom = vec![0u8; 0x80];
		rom[..2].copy_from_slice(&[0xff, 0x4f]);
		rom[2..4].copy_from_slice(&1337u16.to_be_bytes());
		rom[8..12].copy_from_slice(&0x80u32.to_be_bytes());
		for stage in [sb, sc, sd] {
			rom.extend_from_slice(&stage);
			rom.resize(rom.len().next_multiple_of(16), 0xee);
		}
		rom.extend_from_slice(&se);
		rom.resize(rom.len().next_multiple_of(0x1000), 0);
		rom
	}

	fn image(variable: &str) -> Mmap {
		let path = std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} is set"));
		let file = File::open(path).unwrap();
		unsafe { Mmap::map(&file) }.unwrap()
	}
}
