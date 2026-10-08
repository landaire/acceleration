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

## Console filename

For the verified devkit profile, write the compiler output to the HDD root as:

```text
HDD:\xboxromw2d.bin
```

The matching build 16547 kernel contains these path strings beginning at PE file offset `0x474`:

```text
host:\xboxromw2d.bin
\Device\FlashFs\xboxromw2d.bin
\Device\CdRom0\xboxromw2d.bin
\Device\Harddisk0\Partition1\xboxromw2d.bin
```

`\Device\Harddisk0\Partition1` is the device path exposed by XBDM as `HDD:`. The kernel places the strings next to `INIT: failed to read shadow kernel from media.` and `INIT_SHADOWBOOT`, which identifies them as shadowboot media paths rather than flash-update paths.

The filename fields have only the following verified meaning:

- `xboxrom` is the fixed image stem used by the kernel loader.
- `w2d` is literal in the kernel string; its meaning remains unknown.
- `.bin` is the binary image extension.

The evidence is the `0x240000`-byte devkit kernel with SHA-256 `bd57560a12f07eee9d5929b8f3befd93eaea54b6282b5692d165d3e933abf110`.

## Loader availability

`host:\` is a development-host loader namespace, not a console disk partition. It is distinct from the native `\Device\...` paths in the same candidate list. The live development console's XBDM drive list contains `HDD`, `DEVKIT`, cache, memory-unit, and USB mappings but no `HOST` drive. The matching development kernel also labels `host:` as a debug loader path in its `LDRX` strings. The exact early-boot host transport has not been reconstructed; use `HDD:\xboxromw2d.bin` for local deployment.

The inspected retail build 17559 kernel does not contain `xboxrom`, `INIT_SHADOWBOOT`, `shadow kernel`, or an equivalent HDD-root loader path. Its SHA-256 is `e1d1c87dbcbec01c71c5400cc129a462fa46738d267e29151a91c5354bdf6cb8`. This establishes that the development media loader is absent from that retail build. It does not establish behavior for every historical retail kernel.

