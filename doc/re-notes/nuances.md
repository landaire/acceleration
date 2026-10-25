# Flash and shadowboot nuance ledger

- The reference kernel and hypervisor are retail build 17559 artifacts, not live-devkit evidence.
- The separately captured live kernel identifies itself as build 16547 through its PE export name.
- Whole live-kernel memory captures vary in mutable data sections. Immutable sections, including `.text`, are the reproducible build evidence.
- `dmversion` identifies XBDM. Its value `2.0.21256.0` is not the live kernel or hypervisor build.
- `FLASH:\` reports as a directory through file attributes but `getfile FLASH:\` streams a raw 60 MiB image.
- `flash` performs persistent image update work. It is prohibited.
- `magicboot` is a title reboot command in the analyzed build, not a proven shadowboot loader.
- The flash filesystem uses allocation chains. A file's start cluster and size do not imply contiguous storage.
- In the observed image, root metadata uses 1024-byte page pairs: allocation links occupy the first 512-byte page and directory records occupy the second. This is a profile fact, not a general NAND assumption.
- Allocation entries use a 13-bit cluster link. The upper three bits contain metadata and must be preserved when changing a link. Observed examples include `0x5ffe` for free clusters and `0x9fff` for an end-of-chain marker.
- Stage names and sizes are big-endian. The observed boot chain is SB, SC, SD, SE and starts at offset `0x8000`.
- SC starts its HMAC-SHA1 state with 16 zero bytes. SD and SE use the preceding stage's derived 16-byte key.
- SE compression records use big-endian sizes around a continuous LZX stream. Treating each record as an independent LZX stream fails.
- The expanded SE payload contains the `0x40000`-byte hypervisor followed by the `0x240000`-byte kernel.
- A valid recompressed SE stage must still fit the top-level image's declared boot region. The acceleration LZX encoder is valid but may produce a larger stream than the source image's compressor.
- XBDM can read `0x80000000..0x80040000`, but this protected alias returns ciphertext. The verified plaintext hypervisor comes from the decrypted flash boot chain.
- `ExExpansionInstall` entered a non-returning path in two console-signed experiments. Do not invoke it again.
- The live kernel exposes four expansion slots at `0x90001000`; all were empty when inspected. `ExExpansionCall` therefore has no existing reader to invoke.
- The devkit HDD-root shadowboot filename is `HDD:\xboxromw2d.bin`.
- XBDM in the shadowed system loads from the NAND file system and can remove `HDD:\xboxromw2d.bin`. A shadowed boot chain that fails before XBDM starts leaves the file in place, and every cold boot selects it again until it is removed externally.
- The kernel loads `xboxromw2d.bin` whole into physical memory restricted to `0x01000000..0x01800000`. A file larger than `0x800000` bytes is never selected; the complete `0x03c00000`-byte capture fails allocation and the console boots normally.
- The `0xe0000`-byte boot region (flash header plus SB, SC, SD, SE) at `HDD:\xboxromw2d.bin` is selected on cold reboot. Shadowboot is attempted when bit `0x0200` of `XboxHardwareInfo+0x0e` is clear; after the shadowed boot the field read `0x029e`.
- The shadowed system mounts the console NAND file system. Flash root files cannot be changed through a shadowboot image.
- SD verifies SE against the 20-byte value at SD plaintext offset `0x24c` and halts on mismatch. The value is RotSumSha over `SE[0..0x10]` and the RC4-decrypted SE body up to the size rounded to 16 bytes, which includes flash bytes past the SE end. A changed SE needs an updated SD digest; SD authentication is not established.
- `host:\` is the kernel debugger's host file namespace, served over kdnet by an attached kd host, not a disk partition or normal XBDM drive mapping. Without an attached host the open fails at once.
- The inspected retail build 17559 kernel has no development shadowboot media table. This result does not classify every historical retail build.
- XEX images loaded in `0x90000000..0x9fffffff` must carry ImageInfo flag `0x10000000`; images elsewhere must not.
- The console rejects uncompressed XEX data (format 0). Uncompressed images use the Basic format.
- A Basic run table must end with a zero zero-fill size, and runs must be sector-aligned because each run is read directly into the image.
- Normal (LZX) blocks are at most 64 KB and are sized in 2 KB multiples. Stock images follow this; a single unaligned block fails with `C000000D`.
- The import library `id` must equal the exporting module's export-table module number. Zero fails with `C0000225`.
- XEX RotSumSha uses a borrow when the difference exceeds the operand, not when the minuend is smaller than the operand.
- The console verifies XEX signatures in the Montgomery domain without converting back. A valid signature satisfies `s^e * R^-(e-1) mod n == block` with `R = 2^2048`.

## LDI decompressor code tables

The kernel ignores the result of building the main, length and aligned offset tables (kernel 21256 `0x80159150`, `0x801590ec`). Main and length lengths that are neither a complete code nor all zero leave the previous table in place (`0x8015a6e0` rejects them before writing), while each decoded symbol's code length is read from the new lengths. A rejected main code also ends the code reading, so the length lengths and table stay unchanged. The aligned table sums its counts modulo 0x10000 and fills its cleared table until a code no longer fits (`0x8015a9a0`). Tables start zeroed and survive `LDIResetDecompression`. `xenon_lzx` reproduces this, including the kernel's table and node layout. Two cases have no defined result on the console and are errors: a rejected pretree, whose tables are uninitialized stack memory (`0x8015a2d8`), and a code length above 16, which writes past the length counts.

## Patched module base version

After building a patched module the kernel writes the descriptor's source version into the in-memory execution info (`0x800a66c0`). The signed patched headers keep the target version; `xex2::delta` returns them unmodified.
