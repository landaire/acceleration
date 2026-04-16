# xcontent

Parser for Xbox 360 XContent packages. Handles the outer container format that wraps STFS and SVOD filesystems.

## Features

- Parse XContent headers (CON, LIVE, PIRS signature types)
- RSA signature verification via [xecrypt](https://crates.io/crates/xecrypt)
- STFS filesystem access via [stfs](https://crates.io/crates/stfs)
- Content metadata: title ID, media ID, display names, thumbnails, ratings

## Usage

```rust
use xcontent::XContentPackage;

let data = std::fs::read("package.stfs")?;
let package = XContentPackage::parse(&data)?;

println!("Title ID: {:08X}", package.header.metadata.title_id);
println!("Signature: {}", package.header.signature_type);
```

## License

Licensed under either of [Apache License, Version 2.0](../LICENSE-APACHE) or [MIT License](../LICENSE-MIT) at your option.
