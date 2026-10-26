# xex2ool

Command-line tool for inspecting and modifying Xbox 360 XEX2 executables.
Built on top of the [`xex2`](../xex2) parser/rebuilder crate.

## Install

Each [GitHub release](https://github.com/landaire/acceleration/releases)
has executables for Linux, macOS and Windows. To build from crates.io
instead:

```
cargo install xex2ool
```

## Subcommands

| Command                     | What it does                                                                                          |
| --------------------------- | ----------------------------------------------------------------------------------------------------- |
| `info [-e] <file>`          | Header fields, execution info, resources, import libraries. `-e` adds optional headers and security info. |
| `basefile <file> [-o OUT]`  | Extract the decrypted, decompressed PE payload.                                                       |
| `resources <file> [-o DIR]` | Write every XEX resource to `DIR` (defaults to `.`).                                                  |
| `imports <file>`            | List import libraries with their versions and raw IAT / thunk address pairs.                          |
| `idc <file> [-o OUT]`       | Generate an IDA Pro IDC script to import symbols into a disassembly.                                  |
| `xml <file>`                | Dump XEX metadata as XML (matches the legacy `xextool` layout).                                       |
| `patch <file> [flags]`      | Modify the XEX: image patches, resources, compression, encryption, target machine and restrictions. See below. |

`info` and `imports` accept `-f json` (global flag) to emit JSON
instead of formatted tables, for scripting. The other subcommands ignore
the flag since their output is already structured (files, XML, IDC).

## Examples

```
# Inspect a game's metadata
xex2ool info game.xex

# Pull the PE out for disassembly
xex2ool basefile game.xex -o game.pe

# Decompress a compressed XEX (useful for diffing / patching)
xex2ool patch game.xex --decompress -o game-uncompressed.xex

# Re-compress with LZX
xex2ool patch game.xex --compress -o game-compressed.xex

# Flip between retail and devkit target
xex2ool patch game.xex --devkit -o game-devkit.xex
xex2ool patch game.xex --retail -o game-retail.xex

# Encrypt or decrypt in place (omit -o to overwrite the input)
xex2ool patch game.xex --encrypt
xex2ool patch game.xex --decrypt

# Apply verified image writes and describe bytes appended by the patch as a resource
xex2ool patch game.xex \
  --image-patch game.xepatch \
  --add-resource 'assets=0x92800000:0x20000' \
  --devkit --decrypt -o game-patched.xex

# Generate an IDA Pro script next to the file
xex2ool idc game.xex
```

Omitting `-o` replaces the input only after the rebuilt XEX is complete. Existing output permissions are retained.

## Restriction removal

`xex2ool patch` can strip the console-side execution restrictions baked
into a signed XEX. Each can be toggled individually; `--all` strips
everything at once.

| Flag                     | Removes                                |
| ------------------------ | -------------------------------------- |
| `-m`, `--media`          | Media type restriction (DVD, HDD, ...) |
| `-r`, `--region`         | Region restriction                     |
| `-b`, `--bounding-path`  | Bounding-path requirement              |
| `-d`, `--device-id`      | Device-ID restriction                  |
| `-i`, `--console-id`     | Console-ID restriction                 |
| `-y`, `--dates`          | Date-range restriction                 |
| `-v`, `--kv-privileges`  | Keyvault privilege requirement         |
| `-k`, `--signed-kv-only` | Signed-keyvault-only requirement       |
| `-l`, `--lib-versions`   | Minimum library-version requirements   |
| `-z`, `--zero-media-id`  | Zero the media ID field                |
| `-a`, `--all`            | All of the above                       |

`xex2` recomputes the affected hashes and re-signs the security info
with the devkit key. The output loads on devkits and on consoles that
accept devkit-signed XEXs; retail consoles reject the signature.

## Compression modes

`xex2ool patch` can switch a XEX between uncompressed and Normal (LZX)
compression. These flags are mutually exclusive:

| Flag               | Meaning                                                                   |
| ------------------ | ------------------------------------------------------------------------- |
| `--compress`       | Normal (LZX) compression, via the `lzxc` encoder.                         |
| `--decompress`     | No compression; emit the PE image verbatim.                                |
| `--basic-compress` | Basic compression (zero-padded raw blocks). Not implemented; fails.       |

## Image patches and resources

`--image-patch <file>` applies an `XEX2PAT` version 1 patch to the extracted image before the XEX is rebuilt. The big-endian format contains:

- `XEX2PAT\0`
- version 1, source image base and source image size as 32-bit words
- the source image SHA-1
- a 32-bit record count
- records containing a 32-bit virtual address, 32-bit byte count and replacement bytes

The source base, size and SHA-1 must match. Records must not overlap. A record beyond the source image appends zero-filled bytes up to its address. The final image size must remain aligned to the source XEX page size. Every record must remain in the source image's 256 MiB virtual-address region.

`--add-resource NAME=ADDRESS:SIZE` adds XEX resource metadata. Repeat the flag to add several resources. It does not supply the resource bytes: `ADDRESS..ADDRESS+SIZE` must already be part of the loaded image, commonly because an image-patch record added them. Names must fit the eight-byte XEX resource name field.

## Output formats

- Default: human-readable tables (`tabled`) for `info`, `imports`.
- `--format json` (or `-f json`): structured JSON, suitable for piping
  into `jq` or other tools. `imports` emits the `serde` form of
  `xex2::imports::ImportTable`. `info` emits an object of selected fields
  whose nested values (execution info, TLS, resources, security info) use
  the `serde` form of the `xex2` types.
- `xml`: emits the legacy `xextool` XML layout, for drop-in compatibility
  with existing RE workflows.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at
your option.
