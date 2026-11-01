use thiserror::Error;

use crate::raw::RawWarning;

/// What to do with a value whose integrity checks failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrityPolicy {
	/// Use the value as read.
	Ignore,
	/// Reject the value.
	Deny,
}

/// A value read from flash and the integrity checks it failed. A failed check does not stop the
/// read; the caller decides whether to report it, ignore it, or treat it as an error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "failed integrity checks must be ignored, reported, or turned into an error"]
pub struct Checked<T> {
	value: T,
	warnings: Vec<RawWarning>,
}

impl<T> Checked<T> {
	pub fn new(value: T, warnings: Vec<RawWarning>) -> Self {
		Self { value, warnings }
	}

	pub fn value(&self) -> &T {
		&self.value
	}

	pub fn warnings(&self) -> &[RawWarning] {
		&self.warnings
	}

	pub fn into_parts(self) -> (T, Vec<RawWarning>) {
		(self.value, self.warnings)
	}

	/// The value, discarding any failed checks.
	pub fn ignore_warnings(self) -> T {
		self.value
	}

	/// The value, or an error when any check failed.
	pub fn deny_warnings(self) -> Result<T, IntegrityError> {
		if self.warnings.is_empty() {
			return Ok(self.value);
		}

		Err(IntegrityError { warnings: self.warnings })
	}

	pub fn resolve(self, policy: IntegrityPolicy) -> Result<T, IntegrityError> {
		match policy {
			IntegrityPolicy::Ignore => Ok(self.ignore_warnings()),
			IntegrityPolicy::Deny => self.deny_warnings(),
		}
	}

	pub fn map<U, F>(self, map: F) -> Checked<U>
	where
		F: FnOnce(T) -> U,
	{
		Checked { value: map(self.value), warnings: self.warnings }
	}
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{} integrity checks failed, the first: {}", .warnings.len(), .warnings[0])]
pub struct IntegrityError {
	pub warnings: Vec<RawWarning>,
}
