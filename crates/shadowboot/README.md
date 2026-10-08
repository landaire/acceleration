# shadowboot

Compiler for layered Xbox 360 devkit shadowboot images. The current target is the verified 60 MiB devkit image profile.

The compiler reads a base image and applies a directory layer to produce a validated output image.

## Base image

Use the exact read-only flash capture as the base. A directory is not accepted as a base because the observed image contains opaque and authenticated regions that cannot be reconstructed from root files alone.

## Layer

A layer contains only changed inputs. Missing entries are copied from the base image.

```text
layer/
  boot/
    sb.bin             optional complete encrypted SB stage
    sc.bin             optional complete encrypted SC stage
    sd.bin             optional complete encrypted SD stage
    se.bin             optional complete encrypted SE stage
    hypervisor.bin     optional 0x40000-byte plaintext hypervisor
    xboxkrnl.exe       optional 0x240000-byte plaintext kernel image
  root/
    module.xex         replacement or new root file
  delete.txt           optional root names, one per line
```

`se.bin` conflicts with `hypervisor.bin` and `xboxkrnl.exe`. When either plaintext component is present, the missing component is recovered from the base SE stage. The compiler emits the verified 32 KiB record framing, 128 KiB LZX stream, chained HMAC-SHA1 key derivation, and RC4 body encryption. A replacement SC nonce changes the SD body key and therefore requires a re-encrypted `sd.bin`. When the selected SC and SD nonces change the SE body key, the compiler rebuilds SE unless `se.bin` is supplied. A supplied SE is decrypted and its complete framing and `0x280000`-byte payload are validated. The resulting boot chain must fit the base image's declared boot region. A separately prepared complete `se.bin` can be used when another compressor is required.

Root replacements retain their directory slot, timestamp, and as much of their existing allocation chain as possible. New files use the lowest free slot and clusters and carry timestamp zero. `delete.txt` releases and clears file clusters. FAT metadata flags outside the 13-bit allocation link are preserved.

## Build

```text
cargo run -p shadowboot -- \
  --base base.bin \
  --layer layer \
  --output shadowboot.bin
```

