# acceleration

Libraries / utilities for Xbox 360 file formats.

## Crates

| Crate                         | What it does                                                                                                                     |
| ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| [`xex2`](xex2/)               | XEX2 executable parser, rebuilder, and signer                                                                                    |
| [`xex2ool`](xex2ool/)         | CLI for inspecting and modifying XEX files                                                                                       |
| [`lzxc`](lzxc/)               | LZX compression encoder (round-trips with `lzxd`)                                                                                |
| [`xecrypt`](xecrypt/)         | AES, RSA, and RotSumSha primitives from the Xbox 360 kernel                                                                      |
| [`xcontent`](xcontent/)       | "STFS" package (CON/LIVE/PIRS) parser                                                                                            |
| [`stfs`](stfs/)               | STFS filesystem/VFS handler. Allows for reading the inner filesystem of an XContent package when it's STFS and validating hashes |
| [`xenon_types`](xenon_types/) | Shared newtypes (`TitleId`, `Sha1Hash`, `AesKey`, ...)                                                                           |

## xex2ool

```
cargo install --path xex2ool
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

## License

Licensed under either of Apache License, Version 2.0
([LICENSE-APACHE](LICENSE-APACHE)) or MIT license
([LICENSE-MIT](LICENSE-MIT)) at your option.
