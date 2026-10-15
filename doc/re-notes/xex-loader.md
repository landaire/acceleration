# XEX loader requirements

Evidence: build 16547 development kernel and hypervisor, exercised on the deanxbox development console by loading a system-debugger DLL through stock XBDM `dbgextld` from `DEVKIT:\DMEXT`. Addresses are kernel virtual addresses or hypervisor offsets.

## Image-load hypercalls

The kernel calls the hypervisor through `li r0, N; sc` stubs. The hypervisor maps `N` through a stub table at offset `0x15ec0`; each stub loads a byte offset into the handler table at `0xaae0`.

| Kernel number | Hypervisor handler | Purpose |
| --- | --- | --- |
| `0x11` | `0x2c520` | Validate and record SecurityInfo for an image slot |
| `0x13` | `0x2a050` | Create the image mapping |
| `0x14` | `0x2a388` | Load and verify one page range |
| `0x15` | `0x2a678` | Finish data load |
| `0x16` | `0x2a7c8` | Start import resolution |
| `0x17` | `0x2aa38` | Resolve one import library |

The slot index is module flags bit 0: system images use slot 0 and titles use slot 1.

## SecurityInfo validation

Hypercall `0x11` receives the signature and ImageInfo copy and the SecurityInfo `image_size` twice. It rejects the image when:

- `info_size` is outside `0x104..=0x174`: `C000007B`.
- The load address is not 64 KB aligned: `C0000141`.
- The load address is in `0x90000000..0x9fffffff` and ImageInfo flag `0x10000000` is clear: `C000000D`.
- The load address is outside that region and flag `0x10000000` is set: `C000000D`.
- The image size is not a multiple of the page size, which is 4 KB with flag `0x10000000` and 64 KB without it: `C000000D`.
- The image extends past the 256 MB region containing the load address: `C0000141`.

Flag `0x10000000` is the small-page flag. Stock `xbdm.xex` carries it.

## Basic image data

The Basic loader is at `0x800a5610`. The FileFormatInfo size must be at least `0x10`, and the last 32-bit word of the table, the zero-fill size of the final run, must be zero.

Each data run is read through the file callback directly into the mapped image. Runs that are 16-byte multiples but not sector multiples fail with `C000000D` before any module code runs. Runs split on 4 KB page boundaries load. The stock `xsim.xex` extension stores its whole image as one data run.

## Normal (LZX) image data

The Normal loader is at `0x800a5938`. It allocates `window_size + 0x23200` bytes: two 64 KB read buffers, the LZX window, and decoder state. Blocks are read asynchronously into a 64 KB buffer, so no block may exceed 64 KB.

Stock `xbdm.xex` uses a `0x8000` window and blocks of `0xd800`, `0xe800`, `0xe800`, `0x10000`, and `0x2800` bytes. Every size is a multiple of the 2 KB DVD sector. A single `0x2cf0`-byte block failed with `C000000D`; the same image split into 2 KB-multiple blocks loaded.

Each block begins with the next block size and the SHA-1 of the next block. The first block size and hash are in FileFormatInfo.

## Protected pages

Hypercall `0x14` loads each page range with the low nibble of its page descriptor info word. A value with bit `4` set is rejected. A code page (bit `2` clear, bit `1` set) is relocated into protected memory; a data (`2`) or read-only data (`3`) page is protected only when bit `8` is also set. For small-page images the per-page state nibble at hypervisor offset `0x1f58000` records `(type & 3) | 4`, plus `8` for a protected page. The kernel can change the low three bits through `HvxSetImagePageTableEntry` but not bit `8`.

The hypervisor helper at `0x29948` translates an image address to its page table entry. For an address in `0x90000000..0x9fffffff` whose state nibble lacks bit `8`, it clears PTE bit `0x40000000`. The protected read at `0x29a78` requires PTE bits `1` and `0x40000000`, so it reads only protected pages: in practice, code pages.

## Export table

When hypercall `0x15` finishes the data load, `0x29cc0` reads the first `0x2c` bytes of the export table recorded in ImageInfo through the protected read, encrypts the first 16 bytes in place with the per-boot AES key at hypervisor offset `0x16630`, and writes them back. A table outside protected pages is skipped silently and stays plaintext.

The table is eleven big-endian words followed by one word per ordinal:

| Word | Contents |
| --- | --- |
| 0-2 | `0x48000000`, `0x00485645`, `0x48000000` |
| 3, 4 | Module identifier, high and low half |
| 5 | Version, low 24 bits |
| 6 | Minimum version, low 24 bits |
| 7 | Version top byte in bits 8-15, minimum version top byte in bits 0-7 |
| 8 | Image base `>> 16` |
| 9 | Slot count |
| 10 | First ordinal |

Each slot holds the export's offset from the image base, or 0 for an unused ordinal. Stock `xbdm.xex` keeps the table at `0x91f95794`, inside `.text`.

## Import resolution

Hypercall `0x16` decrypts the import descriptor recorded during validation, checks the import library count, and moves the slot to state `3`.

Hypercall `0x17` resolves one import library. The kernel loader at `0x800a11b8` calls it once per library with the image slot, the physical address of the library entry past its size word, and the exporting module's export table address. On any failure the slot moves to state `0xf`. The checks, in order:

| Status | Condition |
| --- | --- |
| `C000000D` | Slot index is 2 or more, or the slot is not in state `3` |
| `C00000CD` | Every library counted by hypercall `0x16` has been resolved |
| `C0000141` | The entry is not 4-byte aligned, or its `0x24`-byte header or record list crosses a 4 KB page |
| `C0000221` | The SHA-1 of the header and records differs from the expected digest; the slot then expects the entry's next-library digest |
| `C0000141` | The export table address is not 4-byte aligned |
| `C0000156` | The first `0x2c` bytes of the export table cannot be read from protected pages |
| `C000007B` | The decrypted first three words are not the magic words |
| `C0000225` | The module identifier (words 3 and 4) differs from the library `id` at entry offset `0x18` |
| `C0000059` | The exporter version is below the library minimum version, or the library version is below the exporter minimum version |

Each record is then resolved. Ordinals are remapped through a table at hypervisor offset `0x16648` for titles when enabled.

| Status | Condition |
| --- | --- |
| `C0000156` | The record address is unaligned, outside the importing image, or unmapped |
| `C000007B` | Record byte 1 differs from the library name index |
| `C0000138` | The ordinal is outside the export table range; hypervisor offset `0x16840` receives the ordinal |
| `C0000156` | The export table slot is not in a protected page |
| `C0000138` | The export table slot is 0; hypervisor offset `0x16840` receives the ordinal |
| `C000007B` | Record byte 0 is neither `0` (data: replaced with the export address) nor `1` (function thunk) |
| `C0000156` | A thunk extends past the importing image, or its second word is unmapped |
| `C000007B` | The thunk's second word is not `0x02000000 \| index << 16 \| ordinal` |

A function thunk is patched to `lis r11, hi` and `addi r11, r11, lo` of the export address.

An export table outside code pages therefore fails every import with `C0000156`, reported as `LDRX: failed to complete load of module`. `Xex2Builder` rejects an export table that does not lie entirely in code pages.

Observed identifiers, constant across `xbdm.xex`, `xsim.xex`, and older titles:

| Library | Identifier |
| --- | --- |
| `xboxkrnl.exe` | `0x45dc17e0` |
| `xam.xex` | `0xfca15c76` |
| `xbdm.xex` | `0xeceb8109` |

## Bounding path

On a flash `\SystemRoot` boot, `0x800a2f90` requires a BoundingPath optional header (`0x000080ff`) when the load flags include `0x40000000`, and rejects a present header that does not match the resolved device path of the file (`C000008B` and `C00000CC`). The blob is a big-endian size word that counts itself, the NUL-terminated path, and zero padding to 4 bytes. `Xex2Builder::bounding_path` emits it.

## Allowed media

Allowed media is the word immediately after the `0x74`-byte ImageInfo. It is not covered by the RSA signature; it is covered by the header hash.
