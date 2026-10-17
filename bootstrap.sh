#!/bin/sh
# Installs DotSlash into .tools and fetches the Buck2 release that
# toolchains/buck2 pins, for Linux and macOS. bin/buck2 runs this on first
# use. Windows uses bootstrap.ps1.
#
# Buck2 downloads everything else it builds with. The host tools it does
# not download are checked at the end and reported, not installed.
set -eu

DOTSLASH_VERSION=v0.5.9

root=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
tools=$root/.tools

say() {
    printf '%s\n' "$*" >&2
}

fail() {
    say "bootstrap: $*"
    exit 1
}

have() {
    command -v "$1" >/dev/null 2>&1
}

sha256_of() {
    if have sha256sum; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}

install_dotslash() {
    for tool in curl tar; do
        have "$tool" || fail "$tool is required to download DotSlash"
    done

    mkdir -p "$tools/bin"
    scratch=$(mktemp -d "$tools/download.XXXXXX")
    trap 'rm -rf "$scratch"' EXIT

    say "Downloading DotSlash $DOTSLASH_VERSION"
    curl -fsSL --retry 3 -o "$scratch/$asset" \
        "https://github.com/facebook/dotslash/releases/download/$DOTSLASH_VERSION/$asset"

    actual=$(sha256_of "$scratch/$asset")
    [ "$actual" = "$sha256" ] || fail "$asset has SHA-256 $actual, not $sha256"

    tar -xzf "$scratch/$asset" -C "$scratch" dotslash
    mv -f "$scratch/dotslash" "$tools/bin/dotslash"
    printf '%s\n' "$sha256" >"$tools/bin/dotslash.sha256"
}

missing() {
    if [ -z "${reported:-}" ]; then
        say "Missing host tools. Buck2 does not download these:"
        reported=1
    fi

    say "  $1: $2"
}

check_host_tools() {
    # Build scripts compile C and C++, and the C++ driver links.
    case $(uname -s) in
    Linux)
        for tool in cc c++ ar; do
            have "$tool" || missing "$tool" "required to compile and link. Install GCC or Clang and binutils (for example gcc, g++ or gcc-c++)."
        done
        ;;
    Darwin)
        xcrun --find clang++ >/dev/null 2>&1 ||
            missing "Xcode command line tools" "required to compile and link. Run xcode-select --install."
        ;;
    esac

    python3 -c 'import sys; sys.exit(sys.version_info < (3, 11))' 2>/dev/null ||
        missing python3 "optional. The build_defs scripts (lint, packaging, build file generation) need Python 3.11 or later."
    have cargo || missing cargo "optional. Cargo builds and build_defs/update_buck_files.py need rustup and the release in rust-toolchain.toml."
    have reindeer || missing reindeer "optional. Only build_defs/update_buck_files.py needs it, after a change to a Cargo.toml."
}

case "$(uname -s)-$(uname -m)" in
Linux-x86_64)
    asset=dotslash-linux-musl.x86_64.$DOTSLASH_VERSION.tar.gz
    sha256=4c75c6eb7890ae35993b962073f6d9bbe78b42b81a5691303ad70f63bfbf7196
    ;;
Linux-aarch64 | Linux-arm64)
    asset=dotslash-linux-musl.arm64.$DOTSLASH_VERSION.tar.gz
    sha256=e58374446dacd4a228388cd275d974bbd8f946916b513cf0eb86bdff5d9d675e
    ;;
Darwin-x86_64 | Darwin-arm64)
    asset=dotslash-macos.$DOTSLASH_VERSION.tar.gz
    sha256=67963df97f5ce512f900a85950add6e883722c3460ee5d752a6e62665fd2f986
    ;;
*)
    fail "no DotSlash release for $(uname -s) $(uname -m)"
    ;;
esac

installed=$(cat "$tools/bin/dotslash.sha256" 2>/dev/null || true)
if [ ! -x "$tools/bin/dotslash" ] || [ "$installed" != "$sha256" ]; then
    install_dotslash
fi

say "Fetching Buck2"
"$root/bin/buck2" --version >&2

check_host_tools
say "Run bin/buck2 in place of buck2, or add $root/bin to PATH."
