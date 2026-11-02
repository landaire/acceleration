## Downloads

| File | Contents |
|---|---|
| `acceleration-<tag>-x86_64-unknown-linux-gnu.tar.gz` | Executables for Linux x86_64 |
| `acceleration-<tag>-aarch64-apple-darwin.tar.gz` | Executables for macOS arm64 |
| `acceleration-<tag>-x86_64-pc-windows-msvc.zip` | Executables for Windows x86_64 |
| `acceleration`, `acceleration_ui`, `shadowboot`, `xex2ool`, `xflash` | [DotSlash](https://dotslash-cli.com/) files for every platform |

Each archive holds all five tools in an `acceleration` directory.

The files without an extension are not executables. Each one is a small text file that names the archive for each platform with its size and hash. With DotSlash installed, a DotSlash file runs like the tool itself. DotSlash downloads the archive for your platform, checks it, and caches it.

```text
chmod +x xflash
./xflash /path/to/nand.bin info
```

On Windows, run `dotslash xflash /path/to/nand.bin info`.
