# lzxc

LZX compression **encoder** in pure Rust. Produces byte streams that
round-trip through the [`lzxd`](https://crates.io/crates/lzxd) decoder.

Targets the LZX variant used by Xbox 360 XEX2 executables, CAB archives,
CHM help files, and Microsoft Patch (MS-PATCH) files.

## What's in this crate

- **`Encoder`** -- low-level primitive. `encode_chunk` takes up to 32 KB
  and returns one compressed chunk holding one complete block;
  `encode_stream` takes a whole input and returns one compressed chunk per
  32 KB, with blocks free to span chunks.
- **`EncoderWriter<W: Write>`** -- high-level `std::io::Write` sink.
  Buffers input into 32 KB slabs, compresses each, forwards as
  `u16 BE chunk_size | chunk_bytes` frames.
- **Optimal parsing**: a hash-chain match finder collects every useful
  match length per position, and a shortest-path parser prices literals,
  repeated-offset matches, and new matches with the block's own Huffman
  code lengths, tracking R0/R1/R2 along the chosen path.
- **Verbatim and aligned offset blocks**, whichever is smaller per block,
  with an uncompressed-block fallback for incompressible data.
  `encode_stream` merges consecutive chunks into one block when that is
  smaller than separate trees.
- **Canonical Huffman** with length caps (package-merge when the classical
  merge overflows) and **pretree delta+RLE** tree transmission.
- **E8 preprocessing** for x86 call-target translation (off by default;
  not useful for Xbox 360 PPC payloads).

## Usage

```rust,no_run
use lzxc::{EncoderWriter, WindowSize};
use std::io::Write;

let mut out = Vec::<u8>::new();
let mut enc = EncoderWriter::new(&mut out, WindowSize::KB64);
enc.write_all(b"hello world, hello world, hello world!").unwrap();
enc.finish().unwrap();
// `out` now holds `u16 BE size | compressed chunk` frames back-to-back.
```

For raw chunk-at-a-time output (e.g. custom framing):

```rust,no_run
use lzxc::{Encoder, WindowSize, MAX_CHUNK_SIZE};

let input: &[u8] = b"...";
let mut enc = Encoder::new(WindowSize::KB64);
for chunk in input.chunks(MAX_CHUNK_SIZE) {
    let compressed = enc.encode_chunk(chunk);
    // compressed[..] is decodable by lzxd::Lzxd::decompress_next.
}
```

For a whole input, `Encoder::encode_stream` returns the chunks at once
and compresses better, since blocks can span chunks.

## Benchmarks

Measured with `cargo bench -p lzxc -- --quick` on an AMD Ryzen 9 7950X3D,
single thread. All corpora are generated deterministically inside the
bench binary, so results reproduce on any host without shipping real
executables. The bench drives `encode_chunk`.

Corpora:

- `text-256k`: 256 KB of English-ish prose built word-at-a-time from a
  ~200-word vocabulary via LCG.
- `text-256k-pathological`: 256 KB of a single 102-byte phrase repeated
  end-to-end. A ceiling on what LZ77+Huffman can do on a perfectly
  periodic signal; not representative of realistic text.
- `structured-1m`: 1 MB mix of zero-runs, a repeating 64-byte "jump table"
  template, biased opcode-like bytes, and LCG random.
- `random-256k`: 256 KB of LCG output. Worst case for LZ matching.

Compression ratio is `input_len / compressed_len` (higher is better).

### Strategy comparison (64 KB window)

| Strategy     | Corpus                   |   Ratio | Throughput |
|--------------|--------------------------|--------:|-----------:|
| Optimal      | text-256k                |   3.30x |  1.5 MiB/s |
| Optimal      | text-256k-pathological   | 359.10x |   30 MiB/s |
| Optimal      | structured-1m            |   3.14x |  1.4 MiB/s |
| Optimal      | random-256k              |   1.00x |  8.6 MiB/s |
| LiteralOnly  | text-256k                |   1.92x |  233 MiB/s |
| LiteralOnly  | text-256k-pathological   |   1.78x |  272 MiB/s |
| LiteralOnly  | structured-1m            |   2.00x |  327 MiB/s |
| LiteralOnly  | random-256k              |   1.00x |  976 MiB/s |
| Uncompressed | text-256k                |   1.00x |  5.7 GiB/s |
| Uncompressed | structured-1m            |   1.00x |  6.5 GiB/s |
| Uncompressed | random-256k              |   1.00x |  5.7 GiB/s |

Random input falls back to uncompressed blocks under every strategy.

### Xbox 360 XEX images

`encode_stream` over the PE image, framed as an XEX "Normal" data region
by the `xex2` crate:

| Image                       | Window | Data region | Time   |
|-----------------------------|--------|------------:|-------:|
| stock xbdm.xex (700 KB)     | 32 KB  |      245760 | 0.19 s |
| stock xbdm.xex (700 KB)     | 64 KB  |      243712 | 0.23 s |
| Microsoft's compressor      | 32 KB  |      249856 |        |

### Window-size sweep (Optimal)

| Window | text-256k | structured-1m |
|--------|----------:|--------------:|
| 32 KB  |     3.22x |         3.13x |
| 64 KB  |     3.30x |         3.14x |
| 128 KB |     3.36x |         3.14x |
| 512 KB |     3.38x |         3.14x |
| 1 MB   |     3.38x |         3.14x |
| 2 MB   |     3.38x |         3.14x |

To reproduce:

```
cargo bench -p lzxc
```

The bench binary also prints one `ratio: ...` line per corpus/strategy
pair to stderr so the numbers can be regenerated without opening the HTML
reports.

## Limitations

### 4 GiB input cap per encoder

A single `Encoder` / `EncoderWriter` can compress **up to 4 GiB of input**
before its match finder degrades to literal-only output. Internal
hash-chain positions are stored as `u32`, so once total bytes fed through
one encoder exceed `u32::MAX`, the stale-entry guard rejects every match
candidate. The output stream remains valid LZX (it decodes correctly via
`lzxd`), but compression ratio collapses to ~1.0x for all bytes past the
4 GiB mark. This applies to release builds. Debug builds instead panic on a
debug assertion (`input crossed 4 GiB; construct a fresh Encoder per
segment`).

In practice XEX payloads and other LZX targets are orders of magnitude
below this bound. If you need to compress larger streams, segment the
input and construct a fresh `Encoder` per segment.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at
your option.
