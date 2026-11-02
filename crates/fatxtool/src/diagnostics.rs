//! Warning and error lines on standard error, colored when standard error is a terminal and
//! `NO_COLOR` is unset.

use std::error::Error;
use std::fmt;
use std::fmt::Write;

use anstyle::AnsiColor;
use anstyle::Style;

/// Prints `<program>: warning: <message>`.
pub fn print_warning<M>(program: &str, message: M)
where
	M: fmt::Display,
{
	anstream::eprint!("{}", render_warning(program, message));
}

/// Prints `<program>: error: <error>` and a `caused by:` line for each source of `error`.
pub fn print_error(program: &str, error: &dyn Error) {
	anstream::eprint!("{}", render_error(program, error));
}

/// The warning line with ANSI styles; the caller's stream removes them when color is off.
fn render_warning<M>(program: &str, message: M) -> String
where
	M: fmt::Display,
{
	let label = Style::new().bold().fg_color(Some(AnsiColor::Yellow.into()));

	render_labeled(program, label, "warning:", &message.to_string())
}

/// The error and its source chain with ANSI styles.
fn render_error(program: &str, error: &dyn Error) -> String {
	let label = Style::new().bold().fg_color(Some(AnsiColor::Red.into()));
	let mut rendered = render_labeled(program, label, "error:", &error.to_string());

	let mut source = error.source();
	while let Some(cause) = source {
		// Writing to a String cannot fail.
		let _ = writeln!(rendered, "  caused by: {cause}");
		source = cause.source();
	}

	rendered
}

/// The first line of `message` after the program and label, and each further line as is; all
/// of them bold.
fn render_labeled(program: &str, label: Style, name: &str, message: &str) -> String {
	let bold = Style::new().bold();
	let mut lines = message.split('\n');
	let first = lines.next().unwrap_or_default();
	let mut rendered = format!("{program}: {label}{name}{label:#} {bold}{first}{bold:#}\n");

	for line in lines {
		// Writing to a String cannot fail.
		let _ = writeln!(rendered, "{bold}{line}{bold:#}");
	}

	rendered
}

#[cfg(test)]
mod tests {
	use super::*;

	#[derive(Debug, thiserror::Error)]
	#[error("cannot open image \"a.bin\"")]
	struct Open {
		#[source]
		source: Read,
	}

	#[derive(Debug, thiserror::Error)]
	#[error("short read at 0x200")]
	struct Read {
		#[source]
		source: std::io::Error,
	}

	fn chain() -> Open {
		Open { source: Read { source: std::io::Error::other("device not ready") } }
	}

	fn plain(rendered: &str) -> String {
		anstream::adapter::strip_str(rendered).to_string()
	}

	#[test]
	fn warning_without_color() {
		let rendered = render_warning("xflash", "page 0x3 has an ECC mismatch\nsecond line\n  indented");

		assert_eq!(plain(&rendered), "xflash: warning: page 0x3 has an ECC mismatch\nsecond line\n  indented\n");
	}

	#[test]
	fn warning_with_color() {
		let rendered = render_warning("xflash", "page 0x3 has an ECC mismatch\nsecond line");

		assert_eq!(
			rendered,
			"xflash: \u{1b}[1m\u{1b}[33mwarning:\u{1b}[0m \u{1b}[1mpage 0x3 has an ECC mismatch\u{1b}[0m\n\
			 \u{1b}[1msecond line\u{1b}[0m\n"
		);
	}

	#[test]
	fn error_without_color() {
		let rendered = render_error("fatxtool", &chain());

		assert_eq!(
			plain(&rendered),
			"fatxtool: error: cannot open image \"a.bin\"\n  caused by: short read at 0x200\n  caused by: device not ready\n"
		);
	}

	#[test]
	fn error_with_color() {
		let rendered = render_error("fatxtool", &chain());

		assert_eq!(
			rendered,
			"fatxtool: \u{1b}[1m\u{1b}[31merror:\u{1b}[0m \u{1b}[1mcannot open image \"a.bin\"\u{1b}[0m\n\
			 \x20 caused by: short read at 0x200\n\
			 \x20 caused by: device not ready\n"
		);
	}

	#[test]
	fn multi_line_error_without_color() {
		let error = std::io::Error::other("first\nsecond");

		assert_eq!(plain(&render_error("xflash", &error)), "xflash: error: first\nsecond\n");
	}
}
