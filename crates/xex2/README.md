# xex2

Parser, extractor, and rebuilder for Xbox 360 XEX2 executables.

## Features

- Parse XEX2 headers, optional headers, and security info
- AES-128-CBC decryption (retail and devkit key auto-detection)
- Basefile (PE image) extraction
  - Uncompressed
  - Basic compression (zero-fill blocks)
  - Normal compression (LZX via [lzxd](https://crates.io/crates/lzxd))
- Optional header decoding: execution info, TLS, resources, game ratings, import libraries, LAN key, bounding path
- Import library table parsing (raw IAT / thunk records) and serialization
- Restriction removal (region, media, bounding path, device/console ID, dates, keyvault, library versions, media ID) with devkit re-signing
- Rebuilding: encrypt/decrypt, retail/devkit target, LZX recompression or decompression, PE replacement (`Xex2::rebuild`)
- Building a XEX from a PE image (`builder::Xex2Builder`)

## Usage

```rust
use xex2::Xex2;

let data = std::fs::read("game.xex")?;
let xex = Xex2::parse(&data)?;

if let Some(exec) = xex.header.execution_info() {
    println!("Title ID: {}", exec.title_id);
}

let pe = xex.extract_basefile(&data)?;
assert_eq!(&pe[0..2], b"MZ");
```

## Limitations

- Delta compression (XEX patches) is not yet supported
- Basic (zero-fill) compression cannot be produced, and Basic-compressed sources cannot be converted
- Import ordinal resolution requires parsing the PE import descriptor table (not yet implemented)

## License

Licensed under either of Apache License, Version 2.0 or MIT License at your option; see [LICENSE](../LICENSE).
