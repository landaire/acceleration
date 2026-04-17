# xex2ool

Command-line tool for inspecting and modifying Xbox 360 XEX2 executables.
Built on top of the [`xex2`](../xex2) parser/rebuilder crate.

## Install

```
cargo install --path xex2ool
```

or from the workspace:

```
cargo install xex2ool
```

## Subcommands

| Command                     | What it does                                                                                          |
| --------------------------- | ----------------------------------------------------------------------------------------------------- |
| `info [-e] <file>`          | Header fields, security info, optional-header dump. `-e` for extended output.                         |
| `basefile <file> [-o OUT]`  | Extract the decrypted, decompressed PE payload.                                                       |
| `resources <file> [-o DIR]` | Write every XEX resource to `DIR` (defaults to `.`).                                                  |
| `imports <file>`            | List import libraries and their imported ordinals / names.                                            |
| `idc <file> [-o OUT]`       | Generate an IDA Pro IDC script to import symbols into a disassembly.                                  |
| `xml <file>`                | Dump XEX metadata as XML (matches the legacy `xextool` layout).                                       |
| `patch <file> [flags]`      | Modify the XEX: compression mode, encryption, target machine, restrictions, delta patches. See below. |

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

# Apply a delta-patch XEX (.xexp) and emit a merged standalone XEX
xex2ool patch original.xex --apply-patch patch.xexp --merge-patch -o merged.xex

# Generate an IDA Pro script next to the file
xex2ool idc game.xex
```

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

Because re-signing is handled by the embedded `xex2` crate, the output
round-trips through a real Xbox 360 signature chain and remains a valid
XEX.

## Compression modes

`xex2ool patch` supports switching between LZX's three XEX compression
modes. These are mutually exclusive:

| Flag               | Meaning                                                                   |
| ------------------ | ------------------------------------------------------------------------- |
| `--compress`       | Normal (LZX) compression, via the `lzxc` encoder.                         |
| `--decompress`     | No compression — emit the PE image verbatim.                              |
| `--basic-compress` | Basic compression (zero-padded raw blocks); not recommended for shipping. |

## Output formats

- Default: human-readable tables (`tabled`) for `info`, `imports`.
- `--format json` (or `-f json`): structured JSON, suitable for piping
  into `jq` or other tools. The JSON shape mirrors the `serde`
  representation of the `xex2` types.
- `xml`: emits the legacy `xextool` XML layout, for drop-in compatibility
  with existing RE workflows.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at
your option.
