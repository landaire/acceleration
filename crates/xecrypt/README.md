# xecrypt

Xbox 360 cryptographic primitives. Implements the subset of the XeCrypt API used for content package and executable verification.

## Features

- AES-128-CBC encrypt/decrypt (`xe_crypt_aes_cbc_encrypt`, `xe_crypt_aes_cbc_decrypt`)
- AES-128-ECB encrypt/decrypt
- SHA-1 hashing
- RSA signature verification for LIVE and PIRS XContent packages (CON verification is not implemented)
- Keyvault parsing (owned and zero-copy) via `keyvault`
- XEX2 header signature signing and verification (`XeCryptBnQwBeSig`) via `xex_sig`
- RSA signing with devkit keys
- `serde` feature: `Serialize` for keyvault, certificate and signature types
- Devkit and retail key support

## Usage

```rust
use xecrypt::symmetric;

let key = [0u8; 16];
let iv = [0u8; 16];
let mut data = vec![0u8; 32];
symmetric::xe_crypt_aes_cbc_encrypt(&key, &iv, &mut data);
```

## Devkit key provenance

The devkit XEX/PIRS private key is the 0x390-byte `XECRYPT_RSAPRV_2048` at
file offset and RVA `0x26b8` in the XDK `%XEDK%\bin\win32\imagexex.exe`.
The analyzed executable has SHA-256
`1de77afc2ff4e450c7b7c00b811858d38c1869df2aa274320fef3023ea4633c3`;
the extracted key blob has SHA-256
`08c0796065e64d162da1b663e8eac6457c219d911f4c6f378bc0a49cfddcbd9d`.
Its modulus matches devkit hypervisor `HvpKeyBlock.PublicKey39`.

This key signs XEX and PIRS data. It is not the key that authenticates the
shadowboot SD stage.

The build 16547 development SD private key is the 0x390-byte
`XECRYPT_RSAPRV_2048` at file offset and RVA `0x6240` in
`%XEDK%\bin\win32\ArcadeLicense.exe`. That executable has SHA-256
`b9ce8b0c67b145e39c04dbc7d66731fad0a7373f2c758b26405bb6851d1e3598`;
the key blob has SHA-256
`57ee2083e62cec0c3e1a97e33091fc248c826f46bb465c914297b0f56066f764`.
Its modulus matches the public key embedded in SD.
The XDK has no `ArcadeLicense.pdb`. Extraction therefore uses the
`XECRYPT_RSAPRV_2048` layout: `Cqw = 32`, exponent 65537, a reserved QWORD,
then the modulus and the `P`, `Q`, `DP`, `DQ`, and `CR` components. The
implementation stores the extracted `P` and `Q` limbs and derives the complete
private key from those factors and the exponent.

`ArcadeLicense.exe` does not reference the SD key record. Its license-update
path calls the signer at virtual address `0x0040c823` with mode zero, which
selects the separate LIVE key at `0x004025a0`. The same signer selects the
PIRS key at `0x00402930` for its other signing mode. No image pointer addresses
the key-data region beginning at `0x004059f0`; the SD key is retained,
unreferenced key material from a statically linked component.

## License

Licensed under either of Apache License, Version 2.0 or MIT License at your option; see [LICENSE](../LICENSE).
