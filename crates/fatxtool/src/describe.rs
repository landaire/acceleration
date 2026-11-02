//! Command-line text for FATX values.

use fatx::ClusterNumber;
use fatx::DeletedOrigin;
use fatx::FatEntry;
use fatx::FatWidth;
use fatx::forensic::Confidence;
use fatx::forensic::content::Agreement;
use fatx::forensic::content::ContentCheck;
use fatx::forensic::content::ContentDetail;
use fatx::forensic::content::ContentKind;
use fatx::forensic::content::ContentMatch;
use fatx::forensic::content::DeclaredSize;
use fatx::forensic::content::StfsKind;
use fatx::forensic::matching::MatchReason;
use fatx::forensic::recover::RecoveryPlan;
use fatx::forensic::recover::RecoveryStrategy;

pub fn cluster(cluster: Option<ClusterNumber>) -> String {
	match cluster {
		Some(cluster) => cluster.to_string(),
		None => "-".to_owned(),
	}
}

pub const fn fat_width(width: FatWidth) -> &'static str {
	match width {
		FatWidth::Fat16 => "FAT16",
		FatWidth::Fat32 => "FAT32",
	}
}

pub fn fat_entry(entry: FatEntry) -> String {
	match entry {
		FatEntry::Free => "free".to_owned(),
		FatEntry::Last => "end of chain".to_owned(),
		FatEntry::Link(next) => format!("link to {next:#x}"),
	}
}

pub const fn origin(origin: DeletedOrigin) -> &'static str {
	match origin {
		DeletedOrigin::LiveDirectory => "in a live directory",
		DeletedOrigin::DeletedDirectory => "in a deleted directory",
	}
}

pub const fn confidence(confidence: Confidence) -> &'static str {
	match confidence {
		Confidence::Low => "low",
		Confidence::Medium => "medium",
		Confidence::High => "high",
	}
}

pub const fn agreement(agreement: Agreement) -> &'static str {
	match agreement {
		Agreement::Agrees => "agrees",
		Agreement::Consistent => "consistent",
		Agreement::Disagrees => "disagrees",
		Agreement::Unknown => "unknown",
	}
}

pub const fn strategy(strategy: RecoveryStrategy) -> &'static str {
	match strategy {
		RecoveryStrategy::Contiguous => "contiguous",
		RecoveryStrategy::ContiguousFree => "contiguous-free",
	}
}

pub const fn kind(kind: ContentKind) -> &'static str {
	match kind {
		ContentKind::Xex2 => "XEX2",
		ContentKind::Stfs(StfsKind::Con) => "STFS CON",
		ContentKind::Stfs(StfsKind::Live) => "STFS LIVE",
		ContentKind::Stfs(StfsKind::Pirs) => "STFS PIRS",
		ContentKind::Bink => "Bink",
		ContentKind::Xtaf => "FATX",
		ContentKind::Png => "PNG",
		ContentKind::Jpeg => "JPEG",
		ContentKind::Bmp => "BMP",
		ContentKind::Riff => "RIFF",
		ContentKind::Dds => "DDS",
		ContentKind::Xpr => "XPR",
		ContentKind::WaveBank => "wave bank",
		ContentKind::SoundBank => "sound bank",
		ContentKind::Xdbf => "XDBF",
		ContentKind::Text => "text",
	}
}

pub const fn reason(reason: MatchReason) -> &'static str {
	match reason {
		MatchReason::FirstCluster => "first cluster",
		MatchReason::SizeAgrees => "size agrees",
		MatchReason::SizeConsistent => "size consistent",
		MatchReason::SizeDisagrees => "size disagrees",
		MatchReason::KindAgrees => "type agrees",
		MatchReason::KindDisagrees => "type disagrees",
	}
}

/// The detected type, the size its header declares, and any header detail.
pub fn content(content: &ContentMatch) -> String {
	let mut text = kind(content.kind).to_owned();
	match content.size {
		DeclaredSize::Exact(size) => text.push_str(&format!(", declares {size} bytes")),
		DeclaredSize::AtLeast(size) => text.push_str(&format!(", declares at least {size} bytes")),
		DeclaredSize::Unknown => {
			// The header declares no size.
		}
	}
	match &content.detail {
		ContentDetail::None => {
			// Nothing more to report.
		}
		ContentDetail::Xex2 { image_size: Some(image_size) } => {
			text.push_str(&format!(", image size {image_size:#x}"));
		}
		ContentDetail::Xex2 { image_size: None } => {
			// The header is cut off before the image size.
		}
		ContentDetail::Stfs { display_name: Some(name) } => text.push_str(&format!(", display name {name:?}")),
		ContentDetail::Stfs { display_name: None } => {
			// The header is cut off before the display name.
		}
	}

	text
}

pub fn content_check(check: &ContentCheck) -> String {
	let detected = match &check.detected {
		Some(detected) => content(detected),
		None => "no known signature".to_owned(),
	};
	let expected = match check.expected {
		Some(expected) => kind(expected),
		None => "no type",
	};

	format!("{detected}; expected {expected}; type {}, size {}", agreement(check.kind), agreement(check.size))
}

/// The clusters a plan reads and what the FAT says about them now.
pub fn plan(plan: &RecoveryPlan) -> String {
	let Some(run) = plan.clusters else {
		return "no clusters".to_owned();
	};

	let mut text = if run.count() == 1 {
		format!("cluster {}", run.first())
	} else {
		let last = u64::from(run.first().get()) + u64::from(run.count()) - 1;
		format!("clusters {}..={last:#x}", run.first())
	};
	if plan.is_intact() {
		text.push_str(", free");
	}
	if !plan.allocated.is_empty() {
		text.push_str(&format!(", {} now allocated", plan.allocated.len()));
	}
	if plan.missing > 0 {
		text.push_str(&format!(", {} past the end of the image", plan.missing));
	}

	text
}
