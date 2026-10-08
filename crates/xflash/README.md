# xflash

Read-only parser and extractor for the observed 60 MiB Xbox 360 devkit flash image profile.

```text
cargo run -p xflash -- /path/to/nand.bin info
cargo run -p xflash -- /path/to/nand.bin list
cargo run -p xflash -- /path/to/nand.bin extract-file xbdm.xex /tmp/xbdm.xex
cargo run -p xflash -- /path/to/nand.bin extract-stage SE /tmp/se.bin
```

The CLI maps its input read-only. It has no device access or flash-programming command. Unknown image profiles are rejected.
