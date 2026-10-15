# acceleration

Libraries / utilities for Xbox 360 file formats.

## Crates

| Crate | What it does |
| --- | --- |
| [`xex2`](crates/xex2/) | XEX2 executable parser, rebuilder, and signer |
| [`xex2ool`](crates/xex2ool/) | CLI for inspecting and modifying XEX files |
| [`lzxc`](crates/lzxc/) | LZX compression encoder (round-trips with `lzxd`) |
| [`xecrypt`](crates/xecrypt/) | AES, RSA, and RotSumSha primitives from the Xbox 360 kernel |
| [`xflash`](crates/xflash/) | Read-only parser and extractor for the observed 60 MiB devkit flash profile |
| [`shadowboot`](crates/shadowboot/) | Layer compiler for deanxbox shadowboot images |
| [`xcontent`](crates/xcontent/) | "STFS" package (CON/LIVE/PIRS) parser |
| [`stfs`](crates/stfs/) | STFS filesystem/VFS handler |
| [`xenon_types`](crates/xenon_types/) | Shared newtypes (`TitleId`, `Sha1Hash`, `AesKey`, etc.) |

## Executables

Each [GitHub release](https://github.com/landaire/acceleration/releases) has
an archive for Linux x86_64, macOS arm64 and Windows x86_64 with
`acceleration`, `acceleration_ui`, `xex2ool`, `xflash` and `shadowboot`. The
published tools also install from crates.io, for example
`cargo install xex2ool`.

## xex2ool

Basic usage:

```
xex2ool info game.xex                 # show module metadata
xex2ool basefile game.xex -o game.pe  # extract decrypted+decompressed PE
xex2ool patch game.xex --decompress --output game-uncompressed.xex
xex2ool patch game.xex --compress   --output game-compressed.xex
xex2ool patch game.xex --encrypt    --output game-encrypted.xex
xex2ool patch game.xex --devkit     --output game-devkit.xex
```

Per-field edits are exposed as flags; see `xex2ool patch --help` for the
full list (region, media, bounding-path removal, date range, etc.).

## Flash and shadowboot tools

Inspect a read-only flash capture:

```text
xflash /path/to/nand.bin info
xflash /path/to/nand.bin list
```

Compile a shadowboot boot-region image from a flash capture and a boot layer:

```text
shadowboot \
  --base /path/to/nand.bin \
  --layer /path/to/layer \
  --output /path/to/shadowboot.bin
```

See [`crates/shadowboot/README.md`](crates/shadowboot/README.md) for the layer contract and [`doc/re-notes`](doc/re-notes/) for the verified deanxbox format record.

## Building

Buck2 builds, tests and lints the workspace on Linux x86_64, macOS arm64
and Windows x86_64. It downloads the Rust release that `rust-toolchain.toml`
names, a pinned nightly rustfmt, and a Python interpreter. The host
provides the C and C++ compiler drivers, which also link: `cc`, `c++` and
`ar` on Linux, Xcode's `clang` and `clang++` on macOS, and the Visual Studio
build tools on Windows.

Each workspace crate has a generated `crates/<name>/BUCK`; the crates.io
dependencies are in the generated `third-party/rust/BUCK`. Cargo.toml stays
the description of every crate, and Cargo only resolves the workspace when
the build files are regenerated. A crate with optional features also has
`-all-features` variants of its library and binaries, so that builds and
lints cover the optional code.

With [devenv](https://devenv.sh), `devenv shell` provides Buck2, reindeer,
Python, and the same Rust release for Cargo and rust-analyzer; the C
compiler stays the host's. Without devenv, install:

- Buck2, the release that `toolchains/buck2_release.json` names. The test
  rules require that release or a later one.
- Python 3.11 or later.
- rustup, which installs the release that `rust-toolchain.toml` names, and
  [reindeer](https://github.com/facebookincubator/reindeer).

| Command | Purpose |
| --- | --- |
| `buck2 build //...` | Build everything. |
| `buck2 test //...` | Run the unit, integration and documentation tests. |
| `buck2 run //crates/xex2ool:xex2ool-bin -- ARGS` | Run a tool; each binary is `//crates/<crate>:<binary>-bin`. |
| `buck2 build -c rust.profile=release //...` | Build with the optimizations of Cargo's release profile. |
| `build_defs/lint.py` | Check formatting and clippy diagnostics. |
| `build_defs/lint.py --format` | Format the workspace crates. |
| `build_defs/package_release.py --version V --target T` | Archive the release executables. |
| `build_defs/update_buck_files.py` | Regenerate the build files after a change to a `Cargo.toml` or `Cargo.lock`. |
| `toolchains/update_rust_release.py` | Regenerate `toolchains/rust_release.bzl` after a change to the channel in `rust-toolchain.toml` or to `RUSTFMT_NIGHTLY` in the script. |

Buck2 builds no benchmarks; `cargo bench -p lzxc` runs them. The Python
bindings build with Cargo; see [`bindings/python`](bindings/python/).

## Releases

A `vX.Y.Z` tag that names the workspace version publishes a GitHub release
with the archives. Every day the `nightly` pre-release is rebuilt from the
default branch when the branch has changed since the last one; its archives
are named `acceleration-nightly-<target>`.

## License

Licensed under either of Apache License, Version 2.0
([LICENSE-APACHE](LICENSE-APACHE)) or MIT license
([LICENSE-MIT](LICENSE-MIT)) at your option.
