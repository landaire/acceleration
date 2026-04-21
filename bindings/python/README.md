# xex2 (Python)

Python bindings for the [`xex2`](../../crates/xex2/) crate via
[`diplomat`](https://rust-diplomat.github.io/book/) +
[`nanobind`](https://github.com/wjakob/nanobind). Parse Xbox 360 XEX2
executables and extract the inner PE from Python.

## Build pipeline

The pipeline is intentionally small; everything below the `src/xex2.cpp`
glue file is auto-generated or stock tooling.

```
┌─────────────────────────────┐
│ crates/xex2-ffi/src/lib.rs  │  #[diplomat::bridge] — hand-written
└────────────┬────────────────┘
             │ diplomat-tool cpp
             ▼
┌─────────────────────────────────────────────┐
│ crates/xex2-ffi/bindings/cpp/*.hpp          │  auto-generated C++
│ (Xex2.hpp, Xex2Bytes.hpp, Xex2Error.hpp,    │  (gitignored)
│  diplomat_runtime.hpp, ...)                 │
└────────────┬────────────────────────────────┘
             │ #include
             ▼
┌─────────────────────────────┐
│ bindings/python/src/xex2.cpp│  nanobind glue — hand-written
└────────────┬────────────────┘
             │ nanobind + scikit-build-core
             ▼
┌─────────────────────────────┐
│ xex2.cpython-<ver>-<abi>.so │  final Python extension module
└─────────────────────────────┘
```

## Install

From PyPI (once published):

```sh
pip install xex2
# or
uv add xex2
```

Pre-built wheels cover CPython 3.10–3.13 on Linux (manylinux_2_28 x86_64),
macOS (arm64 + x86_64), and Windows x86_64. The extension is statically
linked to `libxex2_ffi.a`, so wheels have no dependency on a separately
installed native library.

## Building from source

Source builds need the full Rust workspace available; they're only
intended for local development.

```sh
# From the workspace root:
cargo build -p xex2-ffi --release
diplomat-tool cpp crates/xex2-ffi/bindings/cpp \
    --entry crates/xex2-ffi/src/lib.rs

# Then:
cd bindings/python
uv pip install -e . --no-build-isolation
```

## Use

```python
import xex2

data = open("game.xex", "rb").read()
x = xex2.Xex2.parse(data)

# Always-present fields (from SecurityInfo / ImageInfo / Xex2Header).
print(f"load_address:  {x.load_address:#010x}")
print(f"image_size:    {x.image_size:#x}")
print(f"module_flags:  {x.module_flags:#x}")
print(f"image_flags:   {x.image_flags:#x}")

# Optional-header-backed fields raise RuntimeError when absent —
# `xex2`'s `Option<T>` is propagated as a typed error rather than a
# silent default. Catch or use .get-style wrappers yourself.
try:
    print(f"entry_point:   {x.entry_point:#010x}")
except RuntimeError:
    print("no entry point set")

# ExecutionInfo-derived fields.
for field in ("title_id", "version", "disc_number", "disc_count"):
    try:
        print(f"{field:14s} {getattr(x, field)!r}")
    except RuntimeError:
        pass

# Fixed-size byte fields come back as `bytes`.
print(f"image_hash:    {x.image_hash.hex()}")
print(f"rsa_signature: {len(x.rsa_signature)} bytes")

# Imports / resources are iterable.
for lib in (x.imports()[i] for i in range(len(x.imports()))):
    print(f"import: {lib.name} v{lib.version:#010x} ({len(lib)} records)")

# Modify: strip restrictions, get a re-signed XEX back as bytes.
limits = xex2.Xex2RemoveLimits.all()
open("game-unlocked.xex", "wb").write(x.modify(limits))

# Extract the decrypted / decompressed PE.
open("game.pe", "wb").write(x.extract_basefile())
```

### Surface

- **Always present:** `load_address`, `image_size`, `header_size`,
  `page_descriptor_count`, `info_size`, `image_flags`,
  `import_table_count`, `export_table_address`, `game_regions`,
  `allowed_media_types`, `module_flags`, `data_offset`,
  `security_offset`, `optional_header_count`.
- **Optional scalars** (raise on absence): `entry_point`,
  `original_base_address`, `default_stack_size`, `default_heap_size`,
  `default_fs_cache_size`, `date_range_not_before`,
  `date_range_not_after`, `bounding_path`.
- **ExecutionInfo** (raise on absence): `title_id`, `exec_media_id`,
  `version`, `base_version`, `platform`, `executable_table`,
  `disc_number`, `disc_count`, `savegame_id`.
- **FileFormatInfo** (raise on absence): `compression_type`,
  `encryption_type`, `window_size`.
- **Fixed-size bytes:** `image_hash` (20), `import_table_hash` (20),
  `header_hash` (20), `media_id` (16), `file_key` (16),
  `rsa_signature` (256).
- **Collections:** `imports()` → `Xex2Imports`, `resources()` →
  `Xex2Resources`. Both support `len()`, `[i]`, and `get(i)`; OOB
  indices raise `RuntimeError`.
- **Operations:** `extract_basefile()` → `bytes` (decrypted/decompressed
  PE); `modify(Xex2RemoveLimits)` → `bytes` (re-signed XEX with the
  requested restrictions stripped).

## Tests

```sh
cd bindings/python
uv run pytest
```

Tests that need a real fixture (`xex_files/afplayer.xex`) skip when the
file isn't present so a fresh clone still runs the suite.
