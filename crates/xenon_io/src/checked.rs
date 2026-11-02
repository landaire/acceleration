use std::error::Error;
use std::fmt;

/// What to do with a value whose integrity checks failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrityPolicy {
	/// Use the value as read.
	Ignore,
	/// Reject the value.
	Deny,
}

/// A value read from a source and the integrity checks it failed, as warnings of type `W`. A
/// failed check does not stop the read; the caller decides whether to report it, ignore it, or
/// treat it as an error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "failed integrity checks must be ignored, reported, or turned into an error"]
pub struct Checked<T, W> {
	value: T,
	warnings: Vec<W>,
}

impl<T, W> Checked<T, W> {
	pub fn new(value: T, warnings: Vec<W>) -> Self {
		Self { value, warnings }
	}

	pub fn value(&self) -> &T {
		&self.value
	}

	pub fn warnings(&self) -> &[W] {
		&self.warnings
	}

	pub fn into_parts(self) -> (T, Vec<W>) {
		(self.value, self.warnings)
	}

	/// The value, discarding any failed checks.
	pub fn ignore_warnings(self) -> T {
		self.value
	}

	/// The value, or an error when any check failed.
	pub fn deny_warnings(self) -> Result<T, IntegrityError<W>> {
		if self.warnings.is_empty() {
			return Ok(self.value);
		}

		Err(IntegrityError { warnings: self.warnings })
	}

	pub fn resolve(self, policy: IntegrityPolicy) -> Result<T, IntegrityError<W>> {
		match policy {
			IntegrityPolicy::Ignore => Ok(self.ignore_warnings()),
			IntegrityPolicy::Deny => self.deny_warnings(),
		}
	}

	pub fn map<U, F>(self, map: F) -> Checked<U, W>
	where
		F: FnOnce(T) -> U,
	{
		Checked { value: map(self.value), warnings: self.warnings }
	}
}

/// The failed checks of a value whose warnings were denied. Holds at least one warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityError<W> {
	pub warnings: Vec<W>,
}

impl<W> fmt::Display for IntegrityError<W>
where
	W: fmt::Display,
{
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{} integrity checks failed", self.warnings.len())?;
		if let Some(first) = self.warnings.first() {
			write!(f, ", the first: {first}")?;
		}

		Ok(())
	}
}

impl<W> Error for IntegrityError<W> where W: fmt::Debug + fmt::Display {}
