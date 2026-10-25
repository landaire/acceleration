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

## Resource info

The loader does not read the ResourceInfo optional header (`0x000002ff`). Its only reader in kernel 21256 is `XexGetModuleSection` (`0x800a4290`, ordinal `0x196`). The blob is a big-endian size word that counts itself, then `(size - 4) / 16` entries of an 8-byte name, a 4-byte virtual address and a 4-byte size. A name shorter than 8 bytes ends with NUL. The lookup compares names byte for byte, so it is case-sensitive, and returns the first match. It returns the stored address and size without relocating or checking them, so the data must be in the loaded image at that address. `Xex2Builder::resources` emits the header.

## Allowed media

Allowed media is the word immediately after the `0x74`-byte ImageInfo. It is not covered by the RSA signature; it is covered by the header hash.

## Image data keys

Hypercall `0x11` (hypervisor 17559 handler `0x2c990`) unwraps the ImageInfo file key in place with AES-ECB. The signature check at `0x29a68` selects the master key: image flag `0x2` or `0x4` selects the key at hypervisor offset `0x10cf8`, otherwise the key at offset `0x54`, which the hypervisor image leaves zero for boot code to fill. Flash system modules such as `xam.xex` set both flags. `xex2::crypto::master_key` holds the retail keys; the development-console key for flagged images is not known.

The kernel loaders decrypt with AES-CBC piece by piece, continuing the chain (`XeCryptAesCbc`, `0x80150608`, decrypts whole 16-byte blocks and leaves a partial tail). The executable key that verifies the header signature (retail or development) identifies the console type, and `xex2` selects the master key from it. `Xex2::extract_basefile_unchecked` takes the console type from the caller instead.

## Header layout and load address

`XexpReadImageHeaders` (`0x800a2cf8`) reads the headers of every XEX file, the base and the patch file of a delta patch included. It requires the `XEX2` magic, a header size that is a multiple of `0x800` from `0x800` to `0x10000`, and a zero reserved word at offset `0xC`, then verifies the headers (`0x800a08d8`) and the allowed media. Patched headers built from a delta patch have no size limit; the kernel fails only when it cannot allocate them.

`XexpLoadfile` (`0x800a3260`) requires a 64 KiB aligned load address in `0x80000000..0x9fffffff` whose bit `0x10000000` equals image flag `0x10000000` (small pages). Hypercall `0x11` (hypervisor 17559 `0x2ce74`) repeats these checks and requires the image to end within the 256 MiB region of its load address.

## Normal and Basic image data

`XexpLoadCompressedImage` (`0x800a5938`) creates the LDI decompressor with the FileFormatInfo window and a `0x8000`-byte frame limit. For each block it decrypts, compares the SHA-1 recorded in the previous block (or FileFormatInfo), then decodes frames of `min(0x8000, remaining image)` bytes from big-endian 16-bit-size-prefixed chunks at block offset `0x18` until a zero size. A chunk never spans blocks. Loading stops once the requested image range is complete.

`XexpLoadRawImage` (`0x800a5610`) reads (data, zero fill) runs until a run whose zero fill is zero, then zero-fills the rest of the image. The kernel rejects compression type 0.

## LDI decompressor

`LDICreateDecompression` (`0x80158630`) through `0x8015ab5c` is an LZX decoder. Each `LDIDecompress` call decodes one frame: the input is byte aligned and primed with two little-endian 16-bit words; blocks, codes, repeated offsets and the window persist across frames. Matches may not cross the end of a frame. An uncompressed block's header ends with the 16-bit word that holds its last bits discarded (a full word when the header ends on a word boundary); an odd-sized uncompressed block is followed by one pad byte. E8 translation runs on the output copy for the first `0x8000` frames. The window-data call (`0x80158858` to `0x80158978`) copies reference data to the end of the window and zeroes the rest. `xenon_lzx` implements this decoder.

## Delta patches

`XexpLoadfile` (`0x800a2f90`) builds the patch file name from the module path through the patch descriptors xam registers with `XexRegisterPatchDescriptor` (`0x800a4948`, name built at `0x800a4850`); 6717 xam registers `\Device\Flash\*.xexp_` with the final character replaced by a slot digit (`0x81907660`). The slot digit's source is not traced. Every patch of the 6717 update applies to the 1888 module named without the final `p`. The patch file must have module flag `0x10`; the base must have neither `0x10` nor `0x20`. Optional header `0x5FF` is the delta patch descriptor:

| Offset | Contents |
| --- | --- |
| `0x00` | Descriptor size; header delta records run from `0x4C` to this size |
| `0x04` | Target version |
| `0x08` | Source version, stored as the base version of the loaded module's execution info |
| `0x0C` | SHA-1 of the base file's RSA signature |
| `0x20` | Base image session key, AES-ECB encrypted with the patched module's session key |
| `0x30` | Patched header size |
| `0x34`, `0x38`, `0x3C` | Base header source offset and size, patched header target offset |
| `0x40`, `0x44`, `0x48` | Base image source offset and size, patched image target offset |

`XexDeltaDecompressHeaders` (`0x800a6430`) checks the signature digest, copies the base header range into a zeroed buffer and applies the records with a `0x8000` window. `XexpVerifyXexHeaders` (`0x800a08d8`) then checks the RSA signature (salt `XBOXREVXEX` when image flag `0x80000000` is set) and the header hash. The base image range is loaded with the base FileFormatInfo into a zeroed image, and the patch file's data (compression type 3, the Normal block chain with records at block offset `0x18`) is applied by `XexpLoadDeltaCompressedImage` (`0x800a5ff8`) with the FileFormatInfo window as window and frame limit. The patch data key is the patch file's file key AES-ECB decrypted with the patched module's session key (`0x800a3550`).

A record (`0x800a5ed8`) is a source offset, target offset, 16-bit size and 16-bit compressed size, all big-endian, within one buffer. Size 0 ends the list. Compressed size 0 zeroes the target, 1 moves the source to the target, and otherwise the record's data is one LDI frame decoded with the source bytes as window reference data, after which the decompressor is reset.

Only the patched headers and image exist on the console; no patched XEX file is written.

## Font files in system updates

The 6717 update carries `$flash_XenonCLatin.xttp` and `$flash_XenonJKLatin.xttp`. Each is a complete `xttf` font file, not a patch: the same 0x118-byte header as the `.xtt` files (version `0x00010000`, stored file size equal to the real size), zlib table data that inflates to the declared size, the tables `cmap head hhea hmtx name xchk xttf xloc xglf`, and `xglf` running to the end of the file. `XenonCLatin.xttp` is named "Xbox TC" and covers 333 Unicode ranges, including Cyrillic U+0401, U+0410 to U+044F and U+0451; `XenonJKLatin.xttp` is named "Xbox JK" and covers the same Cyrillic letters and U+E000 to U+E009. Their RSA signatures were not checked.

6717 xam (`0x818ff2a0`) and the 6717 dashboard (`0x921CA590`) choose the same fonts: the primary font is `file://media:/XenonCLatin.xtt` (`XenonJKLatin.xtt` for language 8). If the 16-bit field at `XboxHardwareInfo + 0xE` has bit `0x1000` set, the alternate font is the primary path with `p1` appended; else, if bit `0x2000` is set, `p2`; otherwise there is no alternate. The `.xttp` name occurs in the dashboard and both xams, and in none of the 21256 kernel, `systemupdate.xex` or the 1888 dashboard.

`systemupdate.xex` copies a fixed list of full files from `SOURCE:\$flash_<name>` (table at `0x92008178`, copy at `0x92076b00`); the list includes `xenonclatin.xtt` and `xenonjklatin.xtt`, which this update does not carry, so the 1888 fonts stay. xam's update code lists the package's `$flash_*` files and strips the 7-character prefix (`0x81906da0`, `0x81906f28`), then searches flash for `\Device\Flash\*.xexp_` and `\Device\Flash\Xenon*.xttp_` with the final `_` replaced by `1` or `2` from a state field at `+0xb170` (`0x81907660`). The source of that digit, its relation to the hardware bits, and the code that writes the staged files to flash are not traced. `system_update::FlashFiles` therefore stores each font under its name without the slot digit.
