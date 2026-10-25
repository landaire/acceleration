# acceleration

Libraries and tools for Xbox 360 file formats.

## tl;dr

- Parse, rebuild, convert and sign XEX2 executables with the `xex2` crate or the `xex2ool` CLI.
- Compress with LZX (`lzxc`)
- Reimplement the kernel's AES, RSA and RotSumSha primitives (`xecrypt`).
- Read STFS packages (CON/LIVE/PIRS) and browse them as a filesystem (`xcontent`, `stfs`).
- Inspect devkit flash captures and compile shadowboot images (`xflash`, `shadowboot`).
- Prebuilt executables for Linux, macOS and Windows on every release, a Nix flake, and `cargo install` for the published tools.

## Crates

| Crate | What it does |
| --- | --- |
| [`xex2`](crates/xex2/) | XEX2 executable parser, rebuilder, signer, and delta patch (XEXP) application |
| [`xex2ool`](crates/xex2ool/) | CLI for inspecting and modifying XEX files |
| [`lzxc`](crates/lzxc/) | LZX compression encoder (round-trips with `lzxd`) |
| [`xenon_lzx`](crates/xenon_lzx/) | LZX decoder that behaves like the console kernel's, including delta patch reference data |
| [`xecrypt`](crates/xecrypt/) | AES, RSA, and RotSumSha primitives from the Xbox 360 kernel |
| [`xflash`](crates/xflash/) | Read-only parser and extractor for the observed 60 MiB devkit flash profile |
| [`shadowboot`](crates/shadowboot/) | Layer compiler for deanxbox shadowboot images |
| [`xcontent`](crates/xcontent/) | "STFS" package (CON/LIVE/PIRS) parser |
| [`stfs`](crates/stfs/) | STFS filesystem/VFS handler |
| [`system_update`](crates/system_update/) | Installs a system update package over a set of system flash files |
| [`xenon_types`](crates/xenon_types/) | Shared newtypes (`TitleId`, `Sha1Hash`, `AesKey`, etc.) |

## Executables

Each [GitHub release](https://github.com/landaire/acceleration/releases) has an archive for Linux x86_64, macOS arm64 and Windows x86_64 with `acceleration`, `acceleration_ui`, `xex2ool`, `xflash` and `shadowboot`. The published tools also install from crates.io, for example `cargo install xex2ool`.

### Nix

The flake builds `acceleration`, `xex2ool`, `xflash` and `shadowboot` from source on x86_64 and aarch64 Linux and on macOS arm64. The package of each tool has its name; `default` has all four.

Run one tool:

```text
nix run github:landaire/acceleration#xex2ool -- info game.xex
```

Start a shell with every tool, or with the tools named:

```text
nix shell github:landaire/acceleration
nix shell github:landaire/acceleration#xex2ool github:landaire/acceleration#xflash
```

Add the tools to the development shell of another flake:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    acceleration.url = "github:landaire/acceleration";
  };

  outputs =
    { nixpkgs, acceleration, ... }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = [ acceleration.packages.${system}.default ];
      };
    };
}
```

## xex2ool

Basic usage:

```
xex2ool info game.xex                 # show module metadata
xex2ool basefile game.xex -o game.pe  # extract decrypted+decompressed PE
xex2ool basefile xam.xex --patch xam.xexp -o xam.pe  # PE of the module a delta patch produces
xex2ool patch game.xex --decompress --output game-uncompressed.xex
xex2ool patch game.xex --compress   --output game-compressed.xex
xex2ool patch game.xex --encrypt    --output game-encrypted.xex
xex2ool patch game.xex --devkit     --output game-devkit.xex
```

Per-field edits are exposed as flags; see `xex2ool patch --help` for the full list (region, media, bounding-path removal, date range, etc.).

## System updates

Install a system update package over a directory of system flash files:

```text
acceleration su20076000_00000000 apply-update --base flash-1888 --output flash-6717
```

The package's full files replace or join the base files. Each delta patch is applied to its base module. The result is written as `<name>.headers` and `<name>.basefile`, because the console builds patched modules in memory and never writes them as XEX files.

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

## Releases

A `vX.Y.Z` tag that names the workspace version publishes a GitHub release with the archives. Every day the `nightly` pre-release is rebuilt from the default branch when the branch has changed since the last one. Its archives are named `acceleration-nightly-<target>`.

## Building

[Buck2](https://buck2.build/) builds, tests and lints the workspace on Linux x86_64, macOS arm64 and Windows x86_64.

If you do not have a package manager that makes buck2 easily available, run `bin/buck2` (`bin\buck2.cmd` on Windows) wherever this file says `buck2`, or add `bin` to your `PATH`. The first run downloads [DotSlash](https://dotslash-cli.com) and the Buck2 release that `toolchains/buck2` pins into `.tools`. Buck2 then downloads the Rust release that `rust-toolchain.toml` names, a pinned nightly rustfmt, a Python interpreter, and every crate it needs.

You do need a few host tools:

- The C and C++ compiler drivers, which also link: `cc`, `c++` and `ar` on Linux, Xcode's `clang` and `clang++` on macOS, or Visual Studio Build Tools with the C++ workload on Windows.
- `curl`.
- Python 3.11 or later for the `build_defs` scripts.
- rustup, only to build with Cargo.
- [reindeer](https://github.com/facebookincubator/reindeer), only to regenerate the build files after you change a `Cargo.toml`.

`./bootstrap.sh` (`bootstrap.ps1` on Windows) does the first-run setup by hand and lists any host tools it can't find.

With [devenv](https://devenv.sh), `devenv shell` provides Buck2, reindeer, Python, and the same Rust release for Cargo and rust-analyzer; the C compiler stays the host's.

| Command | Purpose |
| --- | --- |
| `buck2 build //crates/...` | Build the workspace crates and the third-party crates they use on this host. |
| `buck2 test //crates/...` | Run the unit, integration and documentation tests. |
| `buck2 run //crates/xex2ool:xex2ool-bin -- ARGS` | Run a tool; each binary is `//crates/<crate>:<binary>-bin`. |
| `buck2 build -c rust.profile=release //crates/...` | Build with the optimizations of Cargo's release profile. |
| `build_defs/lint.py` | Check formatting and clippy diagnostics. |
| `build_defs/lint.py --format` | Format the workspace crates. |
| `build_defs/package_release.py --version V --target T` | Archive the release executables. |
| `build_defs/update_buck_files.py` | Regenerate the build files after a change to a `Cargo.toml` or `Cargo.lock`. |
| `toolchains/update_rust_release.py` | Regenerate `toolchains/rust_release.bzl` after a change to the channel in `rust-toolchain.toml` or to `RUSTFMT_NIGHTLY` in the script. |

Each workspace crate has a generated `crates/<name>/BUCK`, and the crates.io dependencies are in the generated `third-party/rust/BUCK`. `Cargo.toml` stays the description of every crate. A crate with optional features also has `-all-features` variants of its library and binaries, so builds and lints cover the optional code.

Buck2 builds no benchmarks; `cargo bench -p lzxc` runs them. The Python bindings build with Cargo; see [`bindings/python`](bindings/python/).

To move to another Buck2 release, replace `toolchains/buck2` with the `buck2` DotSlash file of that release, `https://github.com/facebook/buck2/releases/download/<tag>/buck2`. To move to another DotSlash release, change the version and SHA-256 values in both bootstrap scripts and run them again.

## License

Licensed under either of Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)) or MIT license ([LICENSE-MIT](LICENSE-MIT)) at your option.
