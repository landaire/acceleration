# xex2

Parser and extractor for Xbox 360 XEX2 executables.

## Features

- Parse XEX2 headers, optional headers, and security info
- AES-128-CBC decryption (retail and devkit key auto-detection)
- Basefile (PE image) extraction
  - Uncompressed
  - Basic compression (zero-fill blocks)
  - Normal compression (LZX via [lzxd](https://crates.io/crates/lzxd))
- Optional header decoding: execution info, TLS, resources, game ratings, import libraries, LAN key, bounding path
- Import table parsing with kernel export ordinal name resolution
- IDA Pro IDC script generation
- XML metadata output
- XEX restriction patching (region, media, bounding path)

## Usage

```rust
use xex2::Xex2;

let data = std::fs::read("game.xex")?;
let xex = Xex2::parse(data)?;

if let Some(exec) = xex.header.execution_info() {
    println!("Title ID: {}", exec.title_id);
}

let pe = xex.extract_basefile()?;
assert_eq!(&pe[0..2], b"MZ");
```

## Limitations

- Delta compression (XEX patches) is not yet supported
- Full re-encryption/re-compression for modified XEX output is not yet implemented
- Import ordinal resolution requires parsing the PE import descriptor table (not yet implemented)

## License

Licensed under either of [Apache License, Version 2.0](../LICENSE-APACHE) or [MIT License](../LICENSE-MIT) at your option.
