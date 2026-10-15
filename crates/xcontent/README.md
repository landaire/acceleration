# xcontent

Parser for Xbox 360 XContent packages. Handles the outer container format and the STFS filesystem inside it. SVOD volume descriptors are parsed from the header, but `XContentPackage::parse` does not support SVOD packages.

## Features

- Parse XContent headers (CON, LIVE, PIRS signature types)
- RSA signature verification of LIVE and PIRS packages via [xecrypt](https://crates.io/crates/xecrypt) (CON verification is not implemented)
- STFS filesystem access via [stfs](https://crates.io/crates/stfs)
- Content metadata: title ID, media ID, content type, version, display names, publisher/title names, thumbnails

## Usage

```rust
use xcontent::XContentPackage;

let data = std::fs::read("package.stfs")?;
let package = XContentPackage::parse(&data)?;

println!("Title ID: {:08X}", package.header.metadata.title_id);
println!("Signature: {}", package.header.signature_type);
```

## License

Licensed under either of Apache License, Version 2.0 or MIT License at your option; see [LICENSE](../LICENSE).
