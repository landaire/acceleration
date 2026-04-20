# lzxc

LZX compression **encoder** in pure Rust. Produces byte streams that
round-trip through the [`lzxd`](https://crates.io/crates/lzxd) decoder.

Targets the LZX variant used by Xbox 360 XEX2 executables, CAB archives,
CHM help files, and Microsoft Patch (MS-PATCH) files.

## What's in this crate

- **`Encoder`** -- low-level chunk-at-a-time primitive. Feed up to 32 KB
  of input per call, get back one compressed chunk. Use this when you
  need custom chunk framing.
- **`EncoderWriter<W: Write>`** -- high-level `std::io::Write` sink.
  Buffers input into 32 KB slabs, compresses each, forwards as
  `u16 BE chunk_size | chunk_bytes` frames.
- **Canonical Huffman** with 16-bit length cap (falls back to
  length-limited package-merge when the classical merge overflows).
- **Pretree delta+RLE** tree transmission with proactive symbol-19
  (same-run) emission.
- **Stateful hash-chain match finder** that carries window history across
  chunk boundaries so matches can span them.
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

## Benchmarks

Measured with `cargo bench -p lzxc` on an Apple M4 Max, single thread. All
corpora are generated deterministically inside the bench binary, so results
reproduce on any host without shipping real executables.

Corpora:

- `text-256k`: 256 KB of English-ish prose built word-at-a-time from a
  ~200-word vocabulary via LCG. Byte distribution and bigram frequencies
  look like natural language; no phrase-level periodicity.
- `text-256k-pathological`: 256 KB of a single 107-byte phrase repeated
  end-to-end. Kept as a sanity-check ceiling on what LZ77+Huffman can do
  on a perfectly periodic signal — it is **not** representative of
  realistic text.
- `structured-1m`: 1 MB mix of zero-runs, a repeating 64-byte "jump table"
  template, biased opcode-like bytes, and LCG random. Roughly mirrors the
  compressibility profile of a typical executable without impersonating
  any specific one.
- `random-256k`: 256 KB of LCG output. Worst case for LZ matching.

Compression ratio is `input_len / compressed_len` (higher is better).

### Strategy comparison (64 KB window)

| Strategy     | Corpus                   |   Ratio | Throughput |
|--------------|--------------------------|--------:|-----------:|
| Greedy       | text-256k                |   2.50x |  127 MiB/s |
| Greedy       | text-256k-pathological   | 271.93x | 2.93 GiB/s |
| Greedy       | structured-1m            |   2.90x |  129 MiB/s |
| Greedy       | random-256k              |   1.00x |   59 MiB/s |
| LiteralOnly  | text-256k                |   1.92x |  216 MiB/s |
| LiteralOnly  | text-256k-pathological   |   1.78x |  212 MiB/s |
| LiteralOnly  | structured-1m            |   1.39x |  283 MiB/s |
| LiteralOnly  | random-256k              |   1.00x |  236 MiB/s |
| Uncompressed | text-256k                |   1.00x | 35.4 GiB/s |
| Uncompressed | structured-1m            |   1.00x | 42.5 GiB/s |
| Uncompressed | random-256k              |   1.00x | 34.0 GiB/s |

Notes:

- `Greedy` on realistic English prose (`text-256k`) lands at 2.50x, within
  the 2-3x range typical of LZ77+Huffman on natural language.
- `Greedy` on binary-shaped data (`structured-1m`) reaches 2.90x; this is
  the workload `lzxc` was tuned for.
- `text-256k-pathological` — one 107-byte phrase repeated for 256 KB —
  shows the theoretical ceiling: 272x because every match collapses to
  the same main-tree symbol. Useful as a bound, not a headline.
- Random input triggers the encoder's uncompressed-block fallback inside
  `Greedy`, and compresses to 1.00x under every strategy.
- `Uncompressed` is essentially `memcpy` with a 28-byte block header per
  32 KB chunk.

### Window-size sweep (Greedy)

Window controls how far back matches can reach. Ratio plateaus quickly on
these corpora because the dominant match distances are well under 32 KB;
larger windows then pay throughput for bigger hash-chain / history
buffers that don't find new work.

| Window |          text-256k |       structured-1m |
|--------|-------------------:|--------------------:|
| 32 KB  | 2.51x / 130 MiB/s  | 2.90x / 155 MiB/s   |
| 64 KB  | 2.50x / 127 MiB/s  | 2.90x / 129 MiB/s   |
| 128 KB | 2.50x / 117 MiB/s  | 2.90x / 106 MiB/s   |
| 512 KB | 2.51x / 114 MiB/s  | 2.90x /  93 MiB/s   |
| 1 MB   | 2.51x / 115 MiB/s  | 2.90x /  92 MiB/s   |
| 2 MB   | 2.51x / 107 MiB/s  | 2.90x /  88 MiB/s   |

Payloads with long-range repetition (e.g. multi-megabyte binaries with
duplicated data sections) are where larger windows earn their cost; see
[`WindowSize`](https://docs.rs/lzxc/latest/lzxc/enum.WindowSize.html) for
the full list.

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
before its match finder silently degrades to literal-only output. Internal
hash-chain positions are stored as `u32`, so once total bytes fed through
one encoder exceed `u32::MAX`, the stale-entry guard rejects every match
candidate. The output stream remains valid LZX (it decodes correctly via
`lzxd`), but compression ratio collapses to ~1.0x for all bytes past the
4 GiB mark.

In practice XEX payloads and other LZX targets are orders of magnitude
below this bound. If you need to compress larger streams, segment the
input and construct a fresh `Encoder` per segment.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at
your option.
