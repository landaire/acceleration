#!/usr/bin/env python3
"""Checks formatting and clippy diagnostics of the workspace crates.

Usage: build_defs/lint.py [--buck2 PATH] [--format]

Runs from the repository root. Both checks go through Buck2: rustfmt
through `toolchains//:rustfmt`, the pinned nightly rustfmt, and clippy
through the `[clippy.json]` subtarget of every workspace Rust target,
including the `-all-features` variants. Those subtargets succeed even when
clippy reports problems, so this script reads their diagnostics and exits
with status 1 when there are any. With --format, rustfmt rewrites the files
instead, and clippy does not run.
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# The wrapper that runs the pinned Buck2 release.
BUCK2 = ROOT / "bin" / ("buck2.cmd" if sys.platform == "win32" else "buck2")

# Rules of the workspace crates, including their tests.
WORKSPACE_TARGETS = 'kind("^rust_(library|binary|test)$", root//crates/...)'


def buck2(buck2_path, *args):
    """Runs buck2 and returns its stdout; its stderr goes to the console."""
    return subprocess.run([buck2_path, *args], cwd=ROOT, check=True, stdout=subprocess.PIPE, text=True).stdout


def rust_sources():
    files = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "crates/*.rs", "crates/**/*.rs"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.split()
    return sorted(files)


def run_rustfmt(buck2_path, check):
    arguments = ["--check"] if check else []
    result = subprocess.run(
        [buck2_path, "run", "toolchains//:rustfmt", "--", *arguments, "--edition", "2024", *rust_sources()],
        cwd=ROOT,
    )
    return result.returncode == 0


def check_clippy(buck2_path):
    targets = buck2(buck2_path, "uquery", WORKSPACE_TARGETS).split()
    outputs = buck2(
        buck2_path,
        "build",
        "--show-full-simple-output",
        *[target + "[clippy.json]" for target in targets],
    )
    problems = 0
    seen = set()
    for path in outputs.split():
        for line in Path(path).read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            diagnostic = json.loads(line)
            if diagnostic.get("level") not in ("warning", "error"):
                continue
            rendered = diagnostic.get("rendered") or diagnostic.get("message", "")
            # A crate's library and unit-test builds report the same lint.
            if rendered in seen:
                continue
            seen.add(rendered)
            problems += 1
            print(rendered, file=sys.stderr)
    return problems == 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--buck2", default=str(BUCK2), help="the buck2 executable (default: bin/buck2)")
    parser.add_argument("--format", action="store_true", help="rewrite the files with rustfmt instead of checking")
    args = parser.parse_args()

    if args.format:
        return 0 if run_rustfmt(args.buck2, check=False) else 1
    format_ok = run_rustfmt(args.buck2, check=True)
    clippy_ok = check_clippy(args.buck2)
    return 0 if format_ok and clippy_ok else 1


if __name__ == "__main__":
    sys.exit(main())
