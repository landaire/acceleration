# sfcx

Reader for Xbox 360 NAND flash images, named after the console's flash controller.

- Opens a logical image (as XBDM `getfile FLASH:\` returns it) or a raw NAND image with spare areas, and detects which one it has.
- Reads a raw image the way the console's flash driver does: spare areas removed, bad blocks replaced by their spare blocks, every sector checked against its ECC.
- Parses the flash header and the boot chain that follows it: SB, SC, SD and SE on a devkit, CB (or CB_A and CB_B), an optional CC, CD and CE on a retail console. Stages are matched by the loader IDs the hypervisor checks, not by their full names. Stages are kept as stored, so retail stages stay encrypted. It also reads the kernel patch slots, CF and CG, that the flash header points to.
- Reads the flash file system of a raw image from the root directory that the spare areas tag, as the console's driver finds it. A logical image has no spare areas, so its file system is read only for the observed 60 MiB devkit profile.
- Locates the SMC firmware in the boot region and decrypts and encrypts it (`sfcx::smc`).
- Reads the internal memory unit of a raw big-block image, the FATX storage the kernel keeps in flash blocks outside the system file system, as a byte device.
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

The memory unit of a raw image reads like any other source. `MemoryUnitSource` lets a FATX reader use it directly:

```rust
let unit = image.memory_unit(&source)?.deny_warnings()?;
let system = unit.partitions().system;
let header = unit.read(&source, system.start..system.start + 4)?.deny_warnings()?;
let device = sfcx::MemoryUnitSource::new(&unit, &source);
```

Pages the memory unit never wrote read as all ones, as they do on the console.

Any type that implements [`fskit`](https://crates.io/crates/fskit)'s `ReadAt` or `AsyncReadAt` can be a source. Each operation has an `_async` form. To drive the reads yourself, step an `OpenImage` or a `FlashImage::reader` machine and feed it the bytes it asks for.

## Feature flags

- `vfs`: `sfcx_vfs::SfcxVfs` implements `vfs::FileSystem`.
- `async-vfs`: also implements `vfs::async_vfs::AsyncFileSystem`.
- `clap`: `FormatSelection` implements `clap::ValueEnum`.

## License

Licensed under either of Apache License, Version 2.0 or MIT License at your option; see [LICENSE](../LICENSE).
