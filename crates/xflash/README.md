# xflash

Command-line inspector for Xbox 360 devkit flash images, built on [`sfcx`](../sfcx/README.md).

- Prints the boot region and the root files of the flash file system, and copies them out.
- Reads the internal memory unit of a raw NAND image and runs the [`fatxtool`](../fatxtool/README.md) commands on its FATX partitions and on flash volume files such as `xlaunch.fdf`.

```text
xflash /path/to/nand.bin info
xflash /path/to/nand.bin list
xflash /path/to/nand.bin extract-file xbdm.xex /tmp/xbdm.xex
xflash /path/to/nand.bin extract-stage SE /tmp/se.bin
```

The CLI maps its input read-only. It has no device access or flash-programming command. `list` and `extract-file` support the observed 60 MiB devkit file system and reject other profiles. An output that is the input image under another name is refused.

## Memory unit

The console keeps a memory unit in the flash blocks outside the system file system. It holds two FATX partitions, `mu-data` and `mu-system`. Only raw NAND images hold it.

```text
xflash /path/to/nand.bin mu-info
xflash /path/to/nand.bin extract-mu /tmp/mu.bin
xflash /path/to/nand.bin extract-mu --partition system /tmp/mu-system.bin
```

`fatx` runs a FATX command on a memory unit partition or on a flash root file:

```text
xflash /path/to/nand.bin fatx mu-data list --deleted
xflash /path/to/nand.bin fatx mu-data extract /DataRec360.xex /tmp/DataRec360.xex
xflash /path/to/nand.bin fatx mu-system scan
xflash /path/to/nand.bin fatx xlaunch.fdf list
```

See the [`fatxtool` README](../fatxtool/README.md) for the commands.

## Image formats

The input can be either of these:

- A logical image, as XBDM `getfile FLASH:\` returns it. It holds the flash data only.
- A raw NAND image, as a NAND reader or the flash driver's page reads return it. Every 512 bytes of data are followed by a 16-byte spare area.

The format is detected by default. A raw image is read the way the console reads it. Spare areas are removed, bad blocks are replaced by the spare blocks that hold them, and every sector read is checked against its ECC.

Use `--format` to name the format when detection fails:

```text
xflash --format raw-big-block /path/to/raw.bin info
```

`raw` detects only the block geometry. `logical`, `raw-small-block`, `raw-big-on-small-block`, `raw-big-block` and `raw-big-block-256k` name the format and geometry exactly.

## Integrity checks

A sector that fails its ECC check prints a warning, and the command goes on with the bytes as stored. The same goes for memory unit reads and for FATX records the console would skip. Pass `--strict` to stop with an error instead.
