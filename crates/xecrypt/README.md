# xecrypt

Xbox 360 cryptographic primitives. Implements the subset of the XeCrypt API used for content package and executable verification.

## Features

- AES-128-CBC encrypt/decrypt (`xe_crypt_aes_cbc_encrypt`, `xe_crypt_aes_cbc_decrypt`)
- AES-128-ECB encrypt/decrypt
- SHA-1 hashing
- RSA signature verification for XContent packages (LIVE, PIRS, CON)
- Devkit and retail key support

## Usage

```rust
use xecrypt::symmetric;

let key = [0u8; 16];
let iv = [0u8; 16];
let mut data = vec![0u8; 32];
symmetric::xe_crypt_aes_cbc_encrypt(&key, &iv, &mut data);
```

## License

Licensed under either of [Apache License, Version 2.0](../LICENSE-APACHE) or [MIT License](../LICENSE-MIT) at your option.
