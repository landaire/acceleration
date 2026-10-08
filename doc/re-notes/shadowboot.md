# Shadowboot research

## Safety boundary

All work against the devkit is read-only with respect to flash.

Never invoke the XBDM `flash` command. In the analyzed XBDM image:

- Command-table entry `flash` points to `0x91f4a730`.
- The handler accepts a hardware-specific size argument and requests a binary upload.
- Upload completion continues at `0x91f4a628`.
- That path calls the routine at `0x91f735d0`, which reports `XapiFlashKernelImage()` failures.
- The routine calls the kernel import `XeKeysSaveBootLoader` and reports `Done, new image flashed` on success.

This is a persistent flash update path, not a shadowboot path.

`magicboot` is also not a demonstrated shadowboot loader. Its handler at `0x91f49de0` parses title reboot fields (`warm`, `cold`, `wait`, `stop`, `title`, `directory`, and `cmdline`) and enters the title-reboot path.

The `xflash` crate has no image writer and no device access. It maps an existing host file read-only and can only inspect or extract bytes to a separate host file. The `shadowboot` compiler writes only a separate host output assembled from that mapped base.

## Read-only acquisition performed

The live devkit already exposed `FLASH` in `drivelist`, so no drive-map patch was needed. XBDM `getfile FLASH:\` returned a `0x03c00000`-byte host file. This reads the image through the monitor and does not issue the `flash` command.

The observed image has:

- Top-level magic `ff 4f`.
- Top-level build field `0x40a3`.
- Boot-region size `0x000e0000`.
- SB at `0x00008000`, size `0x000040e0`, build 14352.
- SC immediately after SB, size `0x00005bf0`, build 16547.
- SD immediately after SC, size `0x00004f30`, build 16547.
- SE immediately after SD, size `0x000b7f84`, build 16547.
- Flash filesystem data base `0x00b80000` with `0x4000`-byte clusters.
- Root metadata cluster `0x0635` in the observed profile.
- The first 512-byte page of each 1024-byte root metadata pair contains big-endian 16-bit allocation links; the second page contains directory records.
- Allocation link `0x1fff` terminates a chain.

The first root file begins at cluster `0x0010`, which maps to image offset `0x00bc0000`. File fragmentation is real: the observed `bootanim.xex` occupies clusters `0x10` through `0x27`, then `0x38`. A parser that slices only a contiguous byte range produces the wrong final `0x1000` bytes.

## Version constraint

The reference kernel and hypervisor loaded in Binary Ninja are labeled build 17559. They are not live-devkit evidence. A separate read-only XBDM capture established that the live kernel export name is `xboxkrnl.exe@16547.0+1861.0`; XBDM itself reports `dmversion` `2.0.21256.0`.

Consequences:

- XBDM's monitor version does not identify the kernel or hypervisor build.
- No address from the build 17559 reference artifacts may be used on the live devkit.
- No structure size, syscall number, cryptographic field, or image split inferred from build 17559 is accepted as live behavior without matching live evidence.
- The SC, SD, and SE transforms and the SE compressed payload layout were established from the matching flash image. They are recorded below.

## Verified boot-chain extraction

The matching build 16547 SC, SD, and SE stages use a chained 16-byte state. Each stage stores a 16-byte nonce at offset `0x10`; its body starts at offset `0x20`. The transform is:

```text
state = 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
stage_key = HMAC-SHA1(state, stage[0x10..0x20])[0..0x10]
plaintext_body = RC4(stage_key, stage[0x20..stage_size])
state = stage_key
```

The derived keys are:

| Stage | Nonce | Derived key |
| --- | --- | --- |
| SC | `5145e4b206bf24db95c9986a700531fe` | `e0fce3a79ae5465dc613750403a8321a` |
| SD | `0d997d744eebd4e0bbee1b37cb0b2244` | `d1fd2c1f84d684cdd63426fabcdaef5d` |
| SE | `44678c874924cb86dae7abe047fdf74f` | `a47ba40e6c538cc2ebf000cb2f76d6f5` |

The decrypted SE header specifies a `0x280000`-byte output. Its compressed stream starts at offset `0x30` and contains 80 records. Each record has a big-endian 16-bit compressed size, a big-endian 16-bit output size, and a Microsoft LZX block. SD configures a `0x20000`-byte LZX window. Every record expands to `0x8000` bytes.

The expanded payload has SHA-256 `60595b9df77cd19d574b8f7ec354ade75d68397463ebc457f5e57399eedaf2fc` and contains:

- A `0x40000`-byte hypervisor at offset `0`.
- A `0x240000`-byte PE kernel at offset `0x40000`.

The extracted hypervisor has magic `5e 4e`, build 16547, entry offset `0x100`, declared size `0x40000`, and SHA-256 `9bcc8fc231a6b973829ab59664a7b03510bd2e8aa751444965fb0a89bf8e3b08`. Its entry bytes decode as PowerPC instructions.

The companion decompressed kernel is the matching live kernel. Its complete `0x13ac00`-byte `.text` section has SHA-256 `b6aaa048ed3b15d22748b1f498c178cb91ab2c6c7550a795d5c2555dd32667ce`, identical to the XBDM memory capture. This verifies the stage chain, RC4 transforms, LZX parameters, output split, and build identity without using the protected live hypervisor alias.

## Verified image generation

The SE transform is reversible. The compiler emits the same `0x30`-byte header structure, one continuous 128 KiB-window LZX stream split into 32 KiB output records, chained HMAC-SHA1 key derivation, and RC4 encryption beginning at stage offset `0x20`. Decoding a generated stage reconstructs the supplied `0x280000`-byte hypervisor and kernel payload.

The source compressor produced a `0xb7f84`-byte SE stage. Acceleration's valid LZX encoder may produce a larger stage for the same payload. The compiler rejects any complete boot chain that exceeds the top-level `0xe0000` boot-region limit rather than changing an unverified limit. A caller can supply a complete encrypted `se.bin` when a smaller compressor output is available; the compiler decrypts and validates its framing and payload before accepting it.

## Live kernel and hypervisor-alias acquisition

The mapped `xboxkrnl.exe` range was read in `0x1000`-byte XBDM `getmem` requests by the read-only xeedee command:

```text
xeedee --host deanxbox --no-progress dump-module xboxkrnl.exe /tmp/xboxkrnl.exe
```

- Base `0x80040000`.
- Size `0x00240000`.
- Export name `xboxkrnl.exe@16547.0+1861.0`.
- Stable `.text` SHA-256 `b6aaa048ed3b15d22748b1f498c178cb91ab2c6c7550a795d5c2555dd32667ce`.

Two complete captures had SHA-256 values `485b22816c2231e82be33edf58c33591b2ed181c1a4e43ad65bffd2a6039e924` and `6b8ca60a343be9c2198b738f74a123df491d4622dd5c88f617ee58a59e0d869d`. They differed at 9,527 bytes, all within `.data`, `PROTDATA`, and `CLRDATAA`. `.rdata`, `.pdata`, `.edata`, `INIT_RO`, `.timing`, `.xedata`, and `.text` were byte-identical. A whole-memory-image hash is therefore capture-specific, not a kernel build identity.

`walkmem` reports one executable range from `0x80000000` through `0x80280000`. Reading the pre-kernel `0x40000` bytes is permitted by XBDM, but those bytes do not contain the plaintext hypervisor header found in the reference artifact. The captured range has SHA-256 `3db6deb826ddfb45c5f39864260b7b2d21fb05fd8423cbf1c0e2a74b9704f573`. This range is not accepted as a hypervisor dump.

The build 17559 reference hypervisor was used only to identify the expansion container boundary. The live build 16547 kernel exports `ExExpansionInstall` at ordinal 779 (`0x800898e8`) and `ExExpansionCall` at ordinal 780 (`0x80089888`). The call wrapper searches four 16-byte records at `0x90001000`; all four records were zeroed.

A console-signed expansion-install experiment entered a non-returning path twice and required console resets. It produced no hypervisor dump. Do not invoke `ExExpansionInstall` again. The read-only flash-chain procedure above supersedes this path.

## Shadowboot media filename

The matching live build 16547 devkit kernel contains the following strings at PE file offsets `0x474`, `0x493`, `0x4bc`, and `0x4e4`:

```text
host:\xboxromw2d.bin
\Device\FlashFs\xboxromw2d.bin
\Device\CdRom0\xboxromw2d.bin
\Device\Harddisk0\Partition1\xboxromw2d.bin
```

The strings are adjacent to `INIT: failed to read shadow kernel from media.` and `INIT_SHADOWBOOT`. `\Device\Harddisk0\Partition1` is exposed by XBDM as `HDD:`, so the HDD-root destination is `HDD:\xboxromw2d.bin`. The evidence kernel is the `0x240000`-byte file with SHA-256 `bd57560a12f07eee9d5929b8f3befd93eaea54b6282b5692d165d3e933abf110`.

The `xboxrom` stem and `.bin` extension are literal. The meaning of `w2d` remains unknown.

### `host:\` namespace

`host:\` is not a disk partition or a normal XBDM drive mapping. The live development console returned `SysCache0`, `SysCache1`, `SysCache2`, `E`, `DEVKIT`, `HDD`, `MUINT`, `INTUSB` from `drivelist`; it did not return `HOST`. The kernel lists `host:\xboxromw2d.bin` separately from the native `\Device\...` candidates and contains `host:` beside debug-loader `LDRX` diagnostics. This identifies it as a debugger-backed development-host namespace. The early-boot transport and host service protocol remain unknown.

### Retail boundary

The inspected retail build 17559 kernel is `0x1b0000` bytes with SHA-256 `e1d1c87dbcbec01c71c5400cc129a462fa46738d267e29151a91c5354bdf6cb8`. It contains the normal `\Device\Flash` and `\SystemRoot` startup strings but no `xboxrom`, `INIT_SHADOWBOOT`, `shadow kernel`, or corresponding HDD-root loader path. The inspected development kernel contains the complete four-path table. The development media loader is therefore absent from the inspected retail build. Other retail kernel versions have not been classified.

### XEX packaging

The XEX header signature computes RotSumSha over the `0x74`-byte ImageInfo, selects the `XBOX360XEX` or `XBOXREVXEX` salt, applies the `XeCryptBnQwBeSig` SHA-1 and RC4 padding, performs the qword byte-order transforms, and applies the development private exponent. The development hypervisor contains the matching public modulus. `xex2::builder::Xex2Builder` implements this signature form instead of PKCS#1 and can wrap its AES session key with either the development or retail master key. Shadowboot XBDM uses normal compression and development-key AES encryption.

## Implemented image contract

The host compiler uses the exact read-only 60 MiB capture as its base. This preserves opaque and authenticated regions that cannot be reconstructed from an extracted directory. A layer can:

- Replace complete SB, SC, SD, or SE stage images.
- Replace the plaintext hypervisor or kernel and rebuild SE.
- Replace, add, or delete root filesystem files while preserving FAT metadata bits.

Missing layer entries come from the base image. A replacement SC nonce requires a corresponding re-encrypted SD because the nonce changes the SD body key. Changes to the SC and SD nonce chain rebuild SE unless a complete SE is supplied. A supplied SE is decrypted and its framing and payload are validated. The completed output is parsed again, every root allocation chain is checked for bounds, cycles, collisions, and termination, and the boot chain is checked against the declared boot region.

## Hardware deployment

The first hardware attempt used this non-flash procedure:

1. Write the generated image to `HDD:\xboxromw2d.bin`.
2. Download it through XBDM and compare it byte-for-byte with the host image.
3. Request a cold reboot.
4. Query XBDM identity and the HDD file after restart.

The console accepted the `0x03c00000`-byte image. The uploaded and downloaded copies both had SHA-256 `6e5fc0d127bbf2e88feb856e3da4559561f64e820f78b51eeff360f3e15e682c`. After the cold reboot, stock XBDM returned as `deanxbox`, and `HDD:\xboxromw2d.bin` remained present. The file was then removed through stock XBDM and absence was confirmed. An ordinary XBDM cold reboot therefore does not select the HDD shadowboot image. The loader-selection precondition remains unknown.

This path did not call `XapiFlashKernelImage`, `XeKeysSaveBootLoader`, NAND erase, or NAND program. The replacement XBDM worker attempts to remove `HDD:\xboxromw2d.bin` before opening TCP port 730 and restricts its `delete` command to the same path. If the replacement module starts, a subsequent cold reboot should return to stock. If the replacement image fails before the XBDM worker starts, the HDD file remains and recovery requires booting without that HDD path or otherwise removing the file. `flash`, `magicboot`, and expansion installation are not shadowboot substitutes.
