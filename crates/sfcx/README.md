# sfcx

Reader for Xbox 360 NAND flash images, named after the console's flash controller.

- Opens a logical image (as XBDM `getfile FLASH:\` returns it) or a raw NAND image with spare areas, and detects which one it has.
- Reads a raw image the way the console's flash driver does: spare areas removed, bad blocks replaced by their spare blocks, every sector checked against its ECC.
- Parses the boot region (flash header and the SB, SC, SD and SE stages) and the file system of the observed devkit profile.
- Works with any data source through sans-io machines, with synchronous and asynchronous APIs, and mounts the file system as a [`vfs`](https://crates.io/crates/vfs) file system.

## Usage

```rust
use sfcx::FlashImage;
use sfcx::FormatSelection;
use sfcx::io::SliceSource;

let data = std::fs::read("nand.bin")?;
let source = SliceSource(data.as_slice());
let image = FlashImage::open(&source, data.len() as u64, FormatSelection::Auto)?.deny_warnings()?;
let filesystem = image.filesystem(&source)?.deny_warnings()?;
let entry = filesystem.find("xbdm.xex").ok_or("not found")?;
let xbdm = image.read_file(&source, &filesystem, entry)?.deny_warnings()?;
```

Every read returns a `Checked` value. A failed integrity check, such as an ECC mismatch, never stops a read. You choose what happens with `deny_warnings`, `ignore_warnings` or `warnings`.

Any type that implements [`fskit`](https://crates.io/crates/fskit)'s `ReadAt` or `AsyncReadAt` can be a source. Each operation has an `_async` form. To drive the reads yourself, step an `OpenImage` or a `FlashImage::reader` machine and feed it the bytes it asks for.

## Feature flags

- `vfs`: `sfcx_vfs::SfcxVfs` implements `vfs::FileSystem`.
- `async-vfs`: also implements `vfs::async_vfs::AsyncFileSystem`.
- `clap`: `FormatSelection` implements `clap::ValueEnum`.

## License

Licensed under either of Apache License, Version 2.0 or MIT License at your option; see [LICENSE](../LICENSE).
