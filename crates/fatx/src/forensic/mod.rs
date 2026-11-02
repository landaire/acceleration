//! Recovery of deleted data. Every finding is a report: nothing here writes to the volume, and
//! uncertain findings carry their evidence and a [`Confidence`] instead of failing.

pub mod content;
pub mod matching;
pub mod orphan;
pub mod recover;
pub mod scan;

/// How strongly the evidence supports a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Confidence {
	Low,
	Medium,
	High,
}
