# Flash and shadowboot status

Status labels:

- `identified`: purpose and entry point are known; behavior is not fully reconstructed.
- `partial`: principal behavior is reconstructed; edge behavior remains.
- `verified`: behavior was exercised against the live devkit or an exact captured artifact.
- `implemented`: present in acceleration and exercised through its public surface.

## Source artifacts

| Artifact | Evidence | Status |
| --- | --- | --- |
| `xbdm.xex` | Flash file and Binary Ninja image have SHA-256 `36d0cd5e14ddbbcd070ace19c725ff9f2a2c96344c55161da7acac62bee348bd`; loaded module base `0x91f00000` | verified |
| Live XBDM | `dmversion` reports `2.0.21256.0`; module size is `0x000af000` | verified |
| Live kernel | XBDM `getmem` over `0x80040000..0x80280000`; export name `xboxkrnl.exe@16547.0+1861.0`; stable `.text` SHA-256 `b6aaa048ed3b15d22748b1f498c178cb91ab2c6c7550a795d5c2555dd32667ce` | verified |
| Retail kernel reference | Build 17559 file with SHA-256 `e1d1c87dbcbec01c71c5400cc129a462fa46738d267e29151a91c5354bdf6cb8`; user-confirmed retail provenance | identified retail reference, not the live devkit kernel |
| Retail hypervisor reference | Build 17559 file; user-confirmed retail provenance | identified retail reference, not the live devkit hypervisor |
| Read-only flash dump | `getfile FLASH:\` returned exactly `0x03c00000` bytes | verified |
| Build 16547 hypervisor | Read-only SC, SD, and SE decryption followed by LZX expansion; exact size `0x40000`; SHA-256 `9bcc8fc231a6b973829ab59664a7b03510bd2e8aa751444965fb0a89bf8e3b08` | verified |

The build 17559 kernel and hypervisor loaded in Binary Ninja must not be used as byte-identical substitutes for the live system. The live kernel and hypervisor are build 16547 artifacts acquired separately. The expanded flash payload's kernel `.text` section is byte-identical to the live XBDM capture.

## Flash image components

| Component | Status | Notes |
| --- | --- | --- |
| Top-level image header | implemented | Big-endian magic, build, flags, entry, boot-region size |
| SB/SC/SD/SE chain | implemented | Strict contiguous stage parsing from offset `0x8000` |
| Root directory | implemented | Observed fixed deanxbox profile |
| Allocation chains | implemented | Big-endian 16-bit entries with 13-bit links |
| File extraction | implemented | Streams mapped slices; fragmented files supported |
| Stage extraction | implemented | Preserves exact stage bytes |
| Live kernel acquisition | verified | Exact mapped kernel range captured with XBDM `getmem` |
| Stage decryption | verified | SC, SD, and SE use chained HMAC-SHA1-derived RC4 keys |
| SE decompression | verified | 80 big-endian-framed LZX blocks with a `0x20000`-byte window expand to `0x280000` bytes |
| Hypervisor extraction | verified | First `0x40000` bytes of expanded SE payload; matching build header and PowerPC entry |
| SE assembly | implemented | Plaintext hypervisor/kernel overlay, LZX framing, chained key derivation, RC4 encryption, and boot-region bounds |
| Shadowboot image assembly | implemented | Declared boot region of the base image with the boot layer applied; output limited to the `0x800000`-byte kernel load limit; a changed SE needs an SD carrying the matching digest |
| Development shadowboot loader path | identified | Build 16547 devkit kernel selects `xboxromw2d.bin` from host, flash, optical disc, or HDD root; inspected retail build 17559 has no matching loader strings |
| Volatile console handoff | verified | Stock `0xe0000`-byte boot region at `HDD:\xboxromw2d.bin` selected on cold reboot; a 60 MiB image exceeds the load limit and is not selected |
| Flash writing | prohibited | No writer targets a console or flash device |

## XEX generation

| Component | Status | Notes |
| --- | --- | --- |
| Header signature | verified | Montgomery-domain RSA and console RotSumSha borrow rule; generated modules load on hardware |
| SecurityInfo validation | verified | Hypercall `0x11` small-page, alignment, and region rules in `xex-loader.md` |
| Basic image data | verified | 4 KB-page runs load on hardware |
| Normal image data | verified | 64 KB-bounded, 2 KB-aligned LZX block chain loads on hardware |
| Import resolution | verified | Library identifiers checked against exporter module numbers |
