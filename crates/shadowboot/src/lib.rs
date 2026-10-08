mod se;

use std::collections::BTreeSet;

use thiserror::Error;
use xflash::BOOT_CHAIN_OFFSET;
use xflash::BootStage;
use xflash::ClusterId;
use xflash::DEVKIT_CLUSTER_SIZE;
use xflash::DEVKIT_FIRST_FILE_CLUSTER;
use xflash::DEVKIT_ROOT_CLUSTER;
use xflash::DIRECTORY_ENTRY_SIZE;
use xflash::DIRECTORY_NAME_SIZE;
use xflash::DIRECTORY_SLOT_COUNT;
use xflash::FAT_END_OF_CHAIN;
use xflash::FAT_FREE;
use xflash::FAT_LINK_MASK;
use xflash::Image;
use xflash::allocation_entry_offset;
use xflash::cluster_offset;
use xflash::directory_slot_offset;

pub use se::HYPERVISOR_SIZE;
pub use se::KERNEL_SIZE;
pub use se::PAYLOAD_SIZE;

#[derive(Debug, Clone, Copy)]
pub struct RootFile<'a> {
	pub name: &'a str,
	pub bytes: &'a [u8],
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BootLayer<'a> {
	pub sb: Option<&'a [u8]>,
	pub sc: Option<&'a [u8]>,
	pub sd: Option<&'a [u8]>,
	pub se: Option<&'a [u8]>,
	pub hypervisor: Option<&'a [u8]>,
	pub kernel: Option<&'a [u8]>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Layer<'a> {
	pub boot: BootLayer<'a>,
	pub root_files: &'a [RootFile<'a>],
	pub delete_root_files: &'a [&'a str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildReport {
	pub replaced_root_files: usize,
	pub added_root_files: usize,
	pub deleted_root_files: usize,
	pub replaced_boot_stages: usize,
	pub rebuilt_se: bool,
}

#[derive(Debug, Error)]
pub enum Error {
	#[error(transparent)]
	Flash(#[from] xflash::ParseError),
	#[error(transparent)]
	Se(#[from] se::Error),
	#[error("output image size {actual:#x} does not match base image size {expected:#x}")]
	OutputSize { expected: usize, actual: usize },
	#[error("root file name {name:?} is invalid; names must be 1..={maximum} printable ASCII bytes without slashes")]
	InvalidRootName { name: String, maximum: usize },
	#[error("layer contains root file {name:?} more than once")]
	DuplicateRootFile { name: String },
	#[error("layer both deletes and replaces root file {name:?}")]
	DeleteReplaceConflict { name: String },
	#[error("layer requests deletion of missing root file {name:?}")]
	DeleteMissingFile { name: String },
	#[error("root file {name:?} is empty; zero-length files are not established for this flash profile")]
	EmptyRootFile { name: String },
	#[error("root file {name:?} is too large for the u32 directory size field")]
	RootFileTooLarge { name: String },
	#[error("root file {name:?} starts in reserved cluster {cluster:#x}")]
	ReservedFileCluster { name: String, cluster: u16 },
	#[error("root file {name:?} references cluster {cluster:#x} outside the data region")]
	FileClusterOutOfRange { name: String, cluster: u16 },
	#[error("root file {name:?} contains a cluster-chain cycle at {cluster:#x}")]
	ClusterChainCycle { name: String, cluster: u16 },
	#[error("root file {name:?} shares cluster {cluster:#x} with another file")]
	ClusterCollision { name: String, cluster: u16 },
	#[error("root file {name:?} reaches FAT marker {marker:#x} before its declared size")]
	ClusterChainTruncated { name: String, marker: u16 },
	#[error("root file {name:?} ends at cluster {cluster:#x}, whose FAT entry is {next:#x} instead of end-of-chain")]
	ClusterChainNotTerminated { name: String, cluster: u16, next: u16 },
	#[error("root filesystem has no free directory slot for {name:?}")]
	NoDirectorySlot { name: String },
	#[error("root filesystem needs {needed} additional clusters but only {available} are free")]
	NoFreeClusters { needed: usize, available: usize },
	#[error("directory entry offset {offset:#x} is not a root directory slot")]
	UnknownDirectoryOffset { offset: usize },
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

#[derive(Debug)]
struct FileState {
	name: String,
	slot: usize,
	timestamp: u32,
	chain: Vec<u16>,
	active: bool,
}

struct FilesystemState {
	files: Vec<FileState>,
	owned_clusters: [bool; DEVKIT_ROOT_CLUSTER as usize],
	used_slots: [bool; DIRECTORY_SLOT_COUNT],
}

pub fn compile(base: &[u8], output: &mut [u8], layer: &Layer<'_>) -> Result<BuildReport, Error> {
	if output.len() != base.len() {
		return Err(Error::OutputSize { expected: base.len(), actual: output.len() });
	}
	let base_image = Image::parse(base)?;
	output.copy_from_slice(base);

	let mut report = apply_root_layer(&base_image, output, layer.root_files, layer.delete_root_files)?;
	let (replaced_boot_stages, rebuilt_se) = apply_boot_layer(&base_image, output, layer.boot)?;
	report.replaced_boot_stages = replaced_boot_stages;
	report.rebuilt_se = rebuilt_se;

	let output_image = Image::parse(output)?;
	validate_filesystem(&output_image)?;
	Ok(report)
}

fn apply_root_layer(
	image: &Image<'_>,
	output: &mut [u8],
	replacements: &[RootFile<'_>],
	deletions: &[&str],
) -> Result<BuildReport, Error> {
	let FilesystemState { mut files, mut owned_clusters, mut used_slots } = validate_filesystem(image)?;
	let mut free_clusters = BTreeSet::new();
	for cluster in DEVKIT_FIRST_FILE_CLUSTER..DEVKIT_ROOT_CLUSTER {
		if !owned_clusters[cluster as usize]
			&& image.allocation_entry(ClusterId::new(cluster))? & FAT_LINK_MASK == FAT_FREE
		{
			free_clusters.insert(cluster);
		}
	}

	let mut replacement_order: Vec<&RootFile<'_>> = replacements.iter().collect();
	replacement_order.sort_by_key(|file| file.name.to_ascii_lowercase());
	let mut replacement_names = BTreeSet::new();
	for file in &replacement_order {
		validate_root_name(file.name)?;
		if file.bytes.is_empty() {
			return Err(Error::EmptyRootFile { name: file.name.to_owned() });
		}
		if file.bytes.len() > u32::MAX as usize {
			return Err(Error::RootFileTooLarge { name: file.name.to_owned() });
		}
		let normalized = file.name.to_ascii_lowercase();
		if !replacement_names.insert(normalized) {
			return Err(Error::DuplicateRootFile { name: file.name.to_owned() });
		}
	}

	let mut deletion_order: Vec<&str> = deletions.to_vec();
	deletion_order.sort_by_key(|name| name.to_ascii_lowercase());
	let mut deletion_names = BTreeSet::new();
	let mut deleted_root_files = 0usize;
	for name in deletion_order {
		validate_root_name(name)?;
		let normalized = name.to_ascii_lowercase();
		if !deletion_names.insert(normalized.clone()) {
			return Err(Error::DuplicateRootFile { name: name.to_owned() });
		}
		if replacement_names.contains(&normalized) {
			return Err(Error::DeleteReplaceConflict { name: name.to_owned() });
		}
		let file = files
			.iter_mut()
			.find(|file| file.active && file.name.eq_ignore_ascii_case(name))
			.ok_or_else(|| Error::DeleteMissingFile { name: name.to_owned() })?;
		for cluster in file.chain.drain(..) {
			release_cluster(output, cluster)?;
			owned_clusters[cluster as usize] = false;
			free_clusters.insert(cluster);
		}
		clear_directory_slot(output, file.slot)?;
		used_slots[file.slot] = false;
		file.active = false;
		deleted_root_files += 1;
	}

	let mut replaced_root_files = 0usize;
	let mut added_root_files = 0usize;
	for replacement in replacement_order {
		let cluster_count = replacement.bytes.len().div_ceil(DEVKIT_CLUSTER_SIZE);
		if let Some(file) =
			files.iter_mut().find(|file| file.active && file.name.eq_ignore_ascii_case(replacement.name))
		{
			resize_chain(output, &mut file.chain, cluster_count, &mut free_clusters, &mut owned_clusters)?;
			write_root_file(output, &file.chain, replacement.bytes)?;
			write_directory_slot(
				output,
				file.slot,
				&file.name,
				file.chain[0],
				replacement.bytes.len(),
				file.timestamp,
			)?;
			replaced_root_files += 1;
		} else {
			let slot = used_slots
				.iter()
				.position(|used| !used)
				.ok_or_else(|| Error::NoDirectorySlot { name: replacement.name.to_owned() })?;
			let mut chain = Vec::new();
			resize_chain(output, &mut chain, cluster_count, &mut free_clusters, &mut owned_clusters)?;
			write_root_file(output, &chain, replacement.bytes)?;
			write_directory_slot(output, slot, replacement.name, chain[0], replacement.bytes.len(), 0)?;
			used_slots[slot] = true;
			files.push(FileState { name: replacement.name.to_owned(), slot, timestamp: 0, chain, active: true });
			added_root_files += 1;
		}
	}

	Ok(BuildReport {
		replaced_root_files,
		added_root_files,
		deleted_root_files,
		replaced_boot_stages: 0,
		rebuilt_se: false,
	})
}

fn validate_filesystem(image: &Image<'_>) -> Result<FilesystemState, Error> {
	let mut files = Vec::new();
	let mut owned_clusters = [false; DEVKIT_ROOT_CLUSTER as usize];
	let mut used_slots = [false; DIRECTORY_SLOT_COUNT];
	let mut names = BTreeSet::new();

	for entry in image.root_entries() {
		let entry = entry?;
		validate_root_name(entry.name())?;
		if entry.size().get() == 0 {
			return Err(Error::EmptyRootFile { name: entry.name().to_owned() });
		}
		let normalized = entry.name().to_ascii_lowercase();
		if !names.insert(normalized) {
			return Err(Error::DuplicateRootFile { name: entry.name().to_owned() });
		}
		let slot = slot_from_offset(entry.record_offset().get())?;
		used_slots[slot] = true;
		let chain = collect_chain(image, entry.name(), entry.start_cluster().get(), entry.size().get())?;
		for cluster in &chain {
			if owned_clusters[*cluster as usize] {
				return Err(Error::ClusterCollision { name: entry.name().to_owned(), cluster: *cluster });
			}
			owned_clusters[*cluster as usize] = true;
		}
		files.push(FileState {
			name: entry.name().to_owned(),
			slot,
			timestamp: entry.timestamp(),
			chain,
			active: true,
		});
	}
	Ok(FilesystemState { files, owned_clusters, used_slots })
}

fn collect_chain(image: &Image<'_>, name: &str, start: u16, size: usize) -> Result<Vec<u16>, Error> {
	let count = size.div_ceil(DEVKIT_CLUSTER_SIZE);
	let mut chain = Vec::with_capacity(count);
	let mut seen = [false; DEVKIT_ROOT_CLUSTER as usize];
	let mut cluster = start;
	for index in 0..count {
		if cluster < DEVKIT_FIRST_FILE_CLUSTER {
			return Err(Error::ReservedFileCluster { name: name.to_owned(), cluster });
		}
		if cluster >= DEVKIT_ROOT_CLUSTER {
			return Err(Error::FileClusterOutOfRange { name: name.to_owned(), cluster });
		}
		if seen[cluster as usize] {
			return Err(Error::ClusterChainCycle { name: name.to_owned(), cluster });
		}
		seen[cluster as usize] = true;
		chain.push(cluster);
		let next = image.allocation_entry(ClusterId::new(cluster))?;
		let link = next & FAT_LINK_MASK;
		if index + 1 == count {
			if link != FAT_END_OF_CHAIN {
				return Err(Error::ClusterChainNotTerminated { name: name.to_owned(), cluster, next });
			}
		} else if link == FAT_END_OF_CHAIN || link == FAT_FREE {
			return Err(Error::ClusterChainTruncated { name: name.to_owned(), marker: next });
		} else {
			cluster = link;
		}
	}
	Ok(chain)
}

fn resize_chain(
	output: &mut [u8],
	chain: &mut Vec<u16>,
	desired: usize,
	free_clusters: &mut BTreeSet<u16>,
	owned_clusters: &mut [bool],
) -> Result<(), Error> {
	if desired > chain.len() {
		let needed = desired - chain.len();
		if free_clusters.len() < needed {
			return Err(Error::NoFreeClusters { needed, available: free_clusters.len() });
		}
		for _ in 0..needed {
			let cluster = free_clusters.pop_first().expect("free cluster count validated");
			owned_clusters[cluster as usize] = true;
			chain.push(cluster);
		}
	} else {
		for cluster in chain.drain(desired..) {
			release_cluster(output, cluster)?;
			owned_clusters[cluster as usize] = false;
			free_clusters.insert(cluster);
		}
	}
	for (index, cluster) in chain.iter().copied().enumerate() {
		let next = chain.get(index + 1).copied().unwrap_or(FAT_END_OF_CHAIN);
		write_fat(output, cluster, next)?;
	}
	Ok(())
}

fn write_root_file(output: &mut [u8], chain: &[u16], bytes: &[u8]) -> Result<(), Error> {
	for (index, cluster) in chain.iter().copied().enumerate() {
		let offset = cluster_offset(ClusterId::new(cluster))?.get();
		let target = &mut output[offset..offset + DEVKIT_CLUSTER_SIZE];
		target.fill(0);
		let source_start = index * DEVKIT_CLUSTER_SIZE;
		let source_end = (source_start + DEVKIT_CLUSTER_SIZE).min(bytes.len());
		if source_start < source_end {
			target[..source_end - source_start].copy_from_slice(&bytes[source_start..source_end]);
		}
	}
	Ok(())
}

fn release_cluster(output: &mut [u8], cluster: u16) -> Result<(), Error> {
	let offset = cluster_offset(ClusterId::new(cluster))?.get();
	output[offset..offset + DEVKIT_CLUSTER_SIZE].fill(0);
	write_fat(output, cluster, FAT_FREE)
}

fn write_fat(output: &mut [u8], cluster: u16, value: u16) -> Result<(), Error> {
	let offset = allocation_entry_offset(ClusterId::new(cluster))?.get();
	let current = u16::from_be_bytes([output[offset], output[offset + 1]]);
	let next = replace_fat_link(current, value);
	output[offset..offset + 2].copy_from_slice(&next.to_be_bytes());
	Ok(())
}

fn replace_fat_link(current: u16, value: u16) -> u16 {
	(current & !FAT_LINK_MASK) | value
}

fn write_directory_slot(
	output: &mut [u8],
	slot: usize,
	name: &str,
	start_cluster: u16,
	size: usize,
	timestamp: u32,
) -> Result<(), Error> {
	validate_root_name(name)?;
	let offset = directory_slot_offset(slot).ok_or(Error::UnknownDirectoryOffset { offset: slot })?.get();
	let record = &mut output[offset..offset + DIRECTORY_ENTRY_SIZE];
	record.fill(0);
	record[..name.len()].copy_from_slice(name.as_bytes());
	record[22..24].copy_from_slice(&start_cluster.to_be_bytes());
	record[24..28].copy_from_slice(&(size as u32).to_be_bytes());
	record[28..32].copy_from_slice(&timestamp.to_be_bytes());
	Ok(())
}

fn clear_directory_slot(output: &mut [u8], slot: usize) -> Result<(), Error> {
	let offset = directory_slot_offset(slot).ok_or(Error::UnknownDirectoryOffset { offset: slot })?.get();
	output[offset..offset + DIRECTORY_ENTRY_SIZE].fill(0);
	Ok(())
}

fn slot_from_offset(offset: usize) -> Result<usize, Error> {
	(0..DIRECTORY_SLOT_COUNT)
		.find(|slot| directory_slot_offset(*slot).is_some_and(|candidate| candidate.get() == offset))
		.ok_or(Error::UnknownDirectoryOffset { offset })
}

fn validate_root_name(name: &str) -> Result<(), Error> {
	if name.is_empty()
		|| name.len() > DIRECTORY_NAME_SIZE
		|| !name.as_bytes().iter().all(|byte| byte.is_ascii_graphic() && *byte != b'/' && *byte != b'\\')
	{
		return Err(Error::InvalidRootName { name: name.to_owned(), maximum: DIRECTORY_NAME_SIZE });
	}
	Ok(())
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
	use super::*;

	#[test]
	fn fat_link_updates_preserve_metadata_bits() {
		assert_eq!(replace_fat_link(0x9fff, FAT_FREE), 0x9ffe);
		assert_eq!(replace_fat_link(0x5ffe, 0x0123), 0x4123);
	}
}
