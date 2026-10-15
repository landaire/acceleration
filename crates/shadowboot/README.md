# shadowboot

Compiler for Xbox 360 devkit shadowboot images. The current target is the verified 60 MiB devkit image profile.

The compiler reads a base flash capture, applies a boot layer, and writes the image's declared boot region: the flash header followed by the SB, SC, SD, and SE stages. For the verified profile the output is `0xe0000` bytes.

## Base image

Use the exact read-only flash capture as the base. It is mapped read-only and is never modified; the output is written to a separate temporary file and renamed into place.

## Layer

A layer contains only changed boot inputs. Missing entries are copied from the base image. An empty layer directory reproduces the base boot region byte-for-byte.

```text
layer/
  boot/
    sb.bin             optional complete encrypted SB stage
    sc.bin             optional complete encrypted SC stage
    sd.bin             optional complete encrypted SD stage
    se.bin             optional complete encrypted SE stage
    hypervisor.bin     optional 0x40000-byte plaintext hypervisor
    xboxkrnl.exe       optional 0x240000-byte plaintext kernel image
```

`se.bin` conflicts with `hypervisor.bin` and `xboxkrnl.exe`. When either plaintext component is present, the missing component is recovered from the base SE stage. The compiler emits the verified 32 KiB record framing, 128 KiB LZX stream, chained HMAC-SHA1 key derivation, and RC4 body encryption. A replacement SC nonce changes the SD body key and therefore requires a re-encrypted `sd.bin`. When the selected SC and SD nonces change the SE body key, the compiler rebuilds SE unless `se.bin` is supplied. A supplied SE is decrypted and its complete framing and `0x280000`-byte payload are validated. The resulting boot chain must fit the base image's declared boot region. A separately prepared complete `se.bin` can be used when another compressor is required.

The shadowed system mounts the console's own NAND file system, so flash root files are not part of the output and cannot be changed through a shadowboot image.

SD verifies SE with a RotSumSha digest stored in the SD plaintext at offset `0x24c` and halts on mismatch. The digest covers `SE[0..0x10]` and the decrypted SE body up to the size rounded to 16 bytes. A changed SE, including one rebuilt from a replacement hypervisor or kernel, boots only with an SD that carries the matching digest. How SD itself is authenticated has not been established.

## Size limit

The build 16547 kernel loads the whole file into physically contiguous memory restricted to physical `0x01000000..0x01800000`. An image larger than `0x800000` bytes can never be loaded, and the compiler rejects such output.

## Build

```text
shadowboot \
  --base base.bin \
  --layer layer \
  --output shadowboot.bin
```

## Console filename

For the verified devkit profile, write the compiler output to the HDD root as:

```text
HDD:\xboxromw2d.bin
```

The matching build 16547 kernel tries these paths in this order during a cold boot when the console hardware flag that suppresses shadowboot is clear:

```text
host:\xboxromw2d.bin
\Device\CdRom0\xboxromw2d.bin
\Device\FlashFs\xboxromw2d.bin
\Device\Harddisk0\Partition1\xboxromw2d.bin
```

An allocation or read failure prints a diagnostic and the normal boot continues.

`\Device\Harddisk0\Partition1` is the device path exposed by XBDM as `HDD:`. The kernel places the strings next to `INIT: failed to read shadow kernel from media.` and `INIT_SHADOWBOOT`, which identifies them as shadowboot media paths rather than flash-update paths.

The filename fields have only the following verified meaning:

- `xboxrom` is the fixed image stem used by the kernel loader.
- `w2d` is literal in the kernel string; its meaning remains unknown.
- `.bin` is the binary image extension.

The evidence is the `0x240000`-byte devkit kernel with SHA-256 `bd57560a12f07eee9d5929b8f3befd93eaea54b6282b5692d165d3e933abf110`.

## Loader availability

`host:\` is the kernel debugger's host file namespace, not a console disk partition or an XBDM drive. With a kernel debugger host attached over kdnet, the kernel sends the open, read, and close requests for `host:\xboxromw2d.bin` to that host as kd host file I/O packets; the openxbdm repository records the packet format in `docs/re-notes/kd.md`. The shadowboot loader sizes its allocation from the file size in the host's open reply. Without an attached host the open fails at once and the loader tries the next candidate. A stock boot of the build 16547 devkit kernel with a host attached requests `host:\xboxromw2d.bin`; loading a shadowboot image served that way has not been tested. `openxbdm-kd host --file 'host:\xboxromw2d.bin=shadowboot.bin'` would serve the file without writing the console disk. Use `HDD:\xboxromw2d.bin` for the verified deployment.

The inspected retail build 17559 kernel does not contain `xboxrom`, `INIT_SHADOWBOOT`, `shadow kernel`, or an equivalent HDD-root loader path. Its SHA-256 is `e1d1c87dbcbec01c71c5400cc129a462fa46738d267e29151a91c5354bdf6cb8`. This establishes that the development media loader is absent from that retail build. It does not establish behavior for every historical retail kernel.

