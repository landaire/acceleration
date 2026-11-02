//! Correlation of deleted and orphaned records with carved content.

use crate::entry::EntryRecord;
use crate::forensic::Confidence;
use crate::forensic::content::DeclaredSize;
use crate::forensic::content::kind_from_name;
use crate::forensic::scan::CarvedCandidate;

/// A record a match refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryReference {
	/// Index into [`crate::walk::Listing::deleted`].
	Deleted(usize),
	/// Index into [`crate::forensic::scan::ForensicReport::orphans`].
	FreeOrphan(usize),
	/// Index into [`crate::walk::Listing::slack`].
	SlackOrphan(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchReason {
	/// The record's first cluster is where the content starts.
	FirstCluster,
	SizeAgrees,
	/// The header gives a minimum size that the record's size meets.
	SizeConsistent,
	SizeDisagrees,
	/// The record's extension names the detected type.
	KindAgrees,
	KindDisagrees,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryMatch {
	/// Index into [`crate::forensic::scan::ForensicReport::carved`].
	pub candidate: usize,
	pub entry: EntryReference,
	pub reasons: Vec<MatchReason>,
	pub confidence: Confidence,
}

/// Matches a record whose first cluster is the candidate's cluster.
pub(crate) fn match_candidate(
	candidate_index: usize,
	candidate: &CarvedCandidate,
	entry: EntryReference,
	record: &EntryRecord,
) -> EntryMatch {
	let mut reasons = vec![MatchReason::FirstCluster];

	let size = u64::from(record.size());
	match candidate.content.size {
		DeclaredSize::Exact(declared) if declared == size => reasons.push(MatchReason::SizeAgrees),
		DeclaredSize::Exact(_) => reasons.push(MatchReason::SizeDisagrees),
		DeclaredSize::AtLeast(minimum) if minimum <= size => reasons.push(MatchReason::SizeConsistent),
		DeclaredSize::AtLeast(_) => reasons.push(MatchReason::SizeDisagrees),
		DeclaredSize::Unknown => {
			// Nothing to compare.
		}
	}

	match kind_from_name(record.name()) {
		Some(kind) if kind == candidate.content.kind => reasons.push(MatchReason::KindAgrees),
		Some(_) => reasons.push(MatchReason::KindDisagrees),
		None => {
			// The name has no extension that implies a type.
		}
	}

	let contradicted =
		reasons.iter().any(|reason| matches!(reason, MatchReason::SizeDisagrees | MatchReason::KindDisagrees));
	let supported = reasons.iter().any(|reason| matches!(reason, MatchReason::SizeAgrees | MatchReason::KindAgrees));
	let confidence = match (contradicted, supported) {
		(false, true) => Confidence::High,
		(false, false) => Confidence::Medium,
		(true, _) => Confidence::Low,
	};

	EntryMatch { candidate: candidate_index, entry, reasons, confidence }
}
