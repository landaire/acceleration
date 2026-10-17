#!/usr/bin/env python3
"""Builds the workspace executables with the release profile and archives them.

Usage: build_defs/package_release.py --version VERSION --target TRIPLE
                                     [--buck2 PATH] [--out-dir DIR]

Runs from the repository root. The executables are the `rust_binary` rules
of the workspace crates. The archive, `acceleration-VERSION-TRIPLE.zip` on
Windows and `.tar.gz` elsewhere, holds one directory with the executables,
README.md and the license files. Prints the archive path.
"""

import argparse
import json
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# The wrapper that runs the pinned Buck2 release.
BUCK2 = ROOT / "bin" / ("buck2.cmd" if sys.platform == "win32" else "buck2")

# The `[[bin]]` executables of the workspace crates, which
# build_defs/update_buck_files.py labels `bin`.
EXECUTABLES = 'attrfilter(labels, "bin", kind("^rust_binary$", root//crates/...))'

DOCUMENTS = ["README.md", "LICENSE-MIT", "LICENSE-APACHE"]


def buck2(buck2_path, *args):
    """Runs buck2 and returns its stdout; its stderr goes to the console."""
    return subprocess.run([buck2_path, *args], cwd=ROOT, check=True, stdout=subprocess.PIPE, text=True).stdout


def build_executables(buck2_path):
    """Returns the path of every executable, relative to the project root."""
    targets = buck2(buck2_path, "uquery", EXECUTABLES).split()
    if not targets:
        sys.exit("error: the query matched no executables")
    with tempfile.TemporaryDirectory() as scratch:
        report_path = Path(scratch) / "report.json"
        subprocess.run(
            [buck2_path, "build", "-c", "rust.profile=release", f"--build-report={report_path}", *targets],
            cwd=ROOT,
            check=True,
        )
        report = json.loads(report_path.read_text(encoding="utf-8"))
    paths = []
    for target in targets:
        outputs = report["results"][target]["outputs"]["DEFAULT"]
        if len(outputs) != 1:
            sys.exit(f"error: {target} has {len(outputs)} default outputs, not 1")
        paths.append(ROOT / outputs[0])
    return paths


def write_archive(archive, directory, files):
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as out:
            for path in files:
                out.write(path, f"{directory}/{path.name}")
    else:
        with tarfile.open(archive, "w:gz") as out:
            for path in files:
                out.add(path, f"{directory}/{path.name}", filter=_without_owner)


def _without_owner(info):
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    return info


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--version", required=True, help="the release version, for example 0.4.0")
    parser.add_argument("--target", required=True, help="the target triple that names the archive")
    parser.add_argument("--buck2", default=str(BUCK2), help="the buck2 executable (default: bin/buck2)")
    parser.add_argument("--out-dir", type=Path, default=ROOT, help="the directory to write the archive to")
    args = parser.parse_args()

    executables = build_executables(args.buck2)
    directory = f"acceleration-{args.version}-{args.target}"
    extension = ".zip" if "windows" in args.target else ".tar.gz"
    archive = args.out_dir.resolve() / (directory + extension)
    archive.parent.mkdir(parents=True, exist_ok=True)
    write_archive(archive, directory, executables + [ROOT / name for name in DOCUMENTS])
    print(archive)
    return 0


if __name__ == "__main__":
    sys.exit(main())
