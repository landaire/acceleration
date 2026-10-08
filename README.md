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

## xex2ool

```
cargo install --path crates/xex2ool
```

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
cargo run -p xflash -- /path/to/nand.bin info
cargo run -p xflash -- /path/to/nand.bin list
```

Compile a copied base image with boot and root filesystem changes:

```text
cargo run -p shadowboot -- \
  --base /path/to/nand.bin \
  --layer /path/to/layer \
  --output /path/to/shadowboot.bin
```

See [`crates/shadowboot/README.md`](crates/shadowboot/README.md) for the layer contract and [`doc/re-notes`](doc/re-notes/) for the verified deanxbox format record.

## License

Licensed under either of Apache License, Version 2.0
([LICENSE-APACHE](LICENSE-APACHE)) or MIT license
([LICENSE-MIT](LICENSE-MIT)) at your option.
