//! Diagnostics on standard error and the integrity policy of one command-line run.

use std::fmt;

use anstyle::AnsiColor;
use anstyle::Style;
use xenon_io::Checked;
use xenon_io::IntegrityError;
use xenon_io::IntegrityPolicy;

use crate::error::FatxToolError;
use crate::output::InputFile;
use crate::source::VolumeSource;

/// The most failed integrity checks printed at once.
pub const SHOWN_WARNINGS: usize = 16;

/// The program name that prefixes diagnostics, what to do with failed integrity checks, and the
/// input file that outputs must not replace.
#[derive(Debug)]
pub struct Session<'a> {
	program: &'a str,
	policy: IntegrityPolicy,
	input: &'a InputFile,
}

impl<'a> Session<'a> {
	pub fn new(program: &'a str, policy: IntegrityPolicy, input: &'a InputFile) -> Self {
		Self { program, policy, input }
	}

	pub const fn policy(&self) -> IntegrityPolicy {
		self.policy
	}

	pub const fn input(&self) -> &InputFile {
		self.input
	}

	/// Prints failed integrity checks as warnings, at most [`SHOWN_WARNINGS`] of them.
	pub fn report<W>(&self, warnings: &[W])
	where
		W: fmt::Display,
	{
		self.report_counted(warnings, warnings.len());
	}

	/// Prints the first of `total` failed integrity checks as warnings, at most
	/// [`SHOWN_WARNINGS`] of them, and the number of the rest.
	pub fn report_counted<W>(&self, first: &[W], total: usize)
	where
		W: fmt::Display,
	{
		let program = self.program;
		let style = Style::new().bold().fg_color(Some(AnsiColor::Yellow.into()));
		let shown = &first[..first.len().min(SHOWN_WARNINGS)];
		for warning in shown {
			anstream::eprintln!("{program}: {style}warning{style:#}: {warning}");
		}
		if total > shown.len() {
			anstream::eprintln!("{program}: {style}warning{style:#}: {} more warnings", total - shown.len());
		}
	}

	pub fn note(&self, message: fmt::Arguments<'_>) {
		let program = self.program;
		let style = Style::new().bold();
		anstream::eprintln!("{program}: {style}note{style:#}: {message}");
	}

	/// Reports the failed integrity checks of `checked` and applies the policy to them.
	pub fn resolve<T, W>(&self, checked: Checked<T, W>) -> Result<T, IntegrityError<W>>
	where
		W: fmt::Display,
	{
		self.report(checked.warnings());

		checked.resolve(self.policy)
	}

	/// Reports the failed integrity checks of the source's reads so far and applies the policy
	/// to them.
	pub fn check_source<S>(&self, source: &S) -> Result<(), FatxToolError>
	where
		S: VolumeSource,
	{
		self.resolve(Checked::new((), source.take_warnings()))
			.map_err(|error| FatxToolError::SourceIntegrity(Box::new(error)))
	}
}
