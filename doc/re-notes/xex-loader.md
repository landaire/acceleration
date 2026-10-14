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

## Import library identity

Hypercall `0x17` decrypts the exporting module's export table header with a per-boot key. The header begins with the words `0x48000000`, `0x00485645`, and `0x48000000`. The hypervisor combines the low halves of the next two words as `(word3 << 16) | word4` and compares the result with the import library `id` field at entry offset `0x18`. A mismatch fails with `C0000225`, reported as `LDRX: failed to complete load of module`.

Observed identifiers, constant across `xbdm.xex`, `xsim.xex`, and older titles:

| Library | Identifier |
| --- | --- |
| `xboxkrnl.exe` | `0x45dc17e0` |
| `xam.xex` | `0xfca15c76` |
| `xbdm.xex` | `0xeceb8109` |

Before this comparison the hypervisor verifies the import library hash chain (`C0000221`) and, in hypercall `0x16`, the import count recorded during validation.

## Bounding path

On a flash `\SystemRoot` boot, `0x800a2f90` requires a BoundingPath optional header (`0x000080ff`) when the load flags include `0x40000000`, and rejects a present header that does not match the resolved device path of the file (`C000008B` and `C00000CC`). The blob is a big-endian size word that counts itself, the NUL-terminated path, and zero padding to 4 bytes. `Xex2Builder::bounding_path` emits it.

## Allowed media

Allowed media is the word immediately after the `0x74`-byte ImageInfo. It is not covered by the RSA signature; it is covered by the header hash.
