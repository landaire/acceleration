# xflash

Read-only parser and extractor for Xbox 360 devkit flash images. The file system parser supports the observed 60 MiB devkit flash profile.

```text
xflash /path/to/nand.bin info
xflash /path/to/nand.bin list
xflash /path/to/nand.bin extract-file xbdm.xex /tmp/xbdm.xex
xflash /path/to/nand.bin extract-stage SE /tmp/se.bin
```

The CLI maps its input read-only. It has no device access or flash-programming command. Unknown image profiles are rejected.

## Image formats

The input can be either of these:

- A logical image, as XBDM `getfile FLASH:\` returns it. It holds the flash data only.
- A raw NAND image, as a NAND reader or the flash driver's page reads return it. Every 512 bytes of data are followed by a 16-byte spare area.

The format is detected by default. A raw image is converted to the logical image the console sees. Spare areas are removed, bad blocks are replaced by the spare blocks that hold them, and every sector read is checked against its ECC. A sector that fails its ECC check prints a warning.

Use `--format` to name the format when detection fails:

```text
xflash --format raw-big-block /path/to/raw.bin info
```

`raw` detects only the block geometry. `logical`, `raw-small-block`, `raw-big-on-small-block`, `raw-big-block` and `raw-big-block-256k` name the format and geometry exactly.
