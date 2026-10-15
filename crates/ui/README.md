# acceleration_ui

Desktop viewer for Xbox 360 XContent/STFS packages, built with egui/eframe on the `stfs` and `xcontent` crates. It opens a CON, LIVE or PIRS package, shows its metadata and file listing, and extracts files or saves the package contents as a zip.

`src/app.rs` implements the viewer; `src/main.rs` is the native entry point.

## Running

From the repository root:

`buck2 run -c rust.profile=release //crates/ui:acceleration_ui-bin`

On Linux the executable links only libc, libm and libgcc_s; it loads the X11 or Wayland libraries when it starts.

## Web build

The web build files in this directory (`setup_web.sh`, `build_web.sh`, `start_server.sh`, `docs/`) target an older eframe API and do not build. Only the native build is supported.
