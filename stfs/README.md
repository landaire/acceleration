# stfs

Parser for Xbox 360 STFS (Secure Transacted File System) packages. Handles CON, LIVE, and PIRS package types.

## Features

- Parse XContent headers, volume descriptors, and file tables
- Extract files from STFS packages (sans-io via `ReadAt` trait)
- Hash table verification (SHA-1 chain validation)
- Optional VFS integration via [fskit](https://crates.io/crates/fskit) (`vfs` feature)
- Serde serialization with optional base64 encoding for byte fields (`base64-serde` feature)

## Usage

```rust
use stfs::{BytesStfsReader, StfsPackageReader};

let data = std::fs::read("package.stfs")?;
let wrapper = BytesStfsReader::open(data)?;
let package = wrapper.package();

println!("Title: {}", package.header.display_name);

for entry in package.file_table.walk_files() {
    println!("{} ({} bytes)", entry.path, entry.entry.file_size);
}
```

## Feature Flags

- `vfs` - VFS filesystem abstraction via fskit
- `async-vfs` - Async VFS support
- `base64-serde` - Base64 encoding for byte array serde

## License

Licensed under either of [Apache License, Version 2.0](../LICENSE-APACHE) or [MIT License](../LICENSE-MIT) at your option.
