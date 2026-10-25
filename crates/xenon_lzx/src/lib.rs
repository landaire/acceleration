//! LZX decoder with the behavior of the Xbox 360 kernel's LDI decompressor.
//!
//! The kernel decodes XEX "Normal" image data and XEX delta patches with this
//! decompressor. A delta patch places reference data in the window before a
//! frame ([`Decoder::set_reference`]), so the frame copies from the bytes it
//! replaces. The format and its edge behavior were derived from kernel build
//! 21256 (`LDICreateDecompression` at `0x80158630` through `0x8015ab5c`).
//!
//! The crate is `no_std` and does not allocate: the caller supplies the
//! window storage.

#![no_std]

mod bits;
mod decoder;
mod error;
mod huffman;

pub use decoder::Decoder;
pub use error::Error;

#[cfg(test)]
mod tests;
