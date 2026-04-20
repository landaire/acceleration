#![warn(clippy::all, rust_2018_idioms)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

// When compiling natively:
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
	// Log to stdout (if you run with `RUST_LOG=debug`).
	tracing_subscriber::fmt::init();

	let native_options = eframe::NativeOptions {
		viewport: egui::ViewportBuilder::default()
			.with_inner_size([600.0, 400.0])
			.with_min_inner_size([400.0, 300.0])
			.with_title(format!("{} v{}", acceleration_ui::APP_NAME, env!("CARGO_PKG_VERSION"))),
		..Default::default()
	};
	eframe::run_native(
		acceleration_ui::APP_NAME,
		native_options,
		Box::new(|cc| Box::new(acceleration_ui::AccelerationApp::new(cc))),
	)
}
