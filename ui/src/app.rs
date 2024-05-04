use std::io::Cursor;
use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use egui::Image;
use egui::ImageSource;
use egui::Ui;
use egui_extras::Column;

use clipboard::ClipboardContext;
use clipboard::ClipboardProvider;
use egui::Label;
use egui::Sense;
use egui::Spinner;
use egui::TextBuffer;
use log::info;
use rfd::AsyncFileDialog;
#[cfg(not(target_arch = "wasm32"))]
use rfd::FileDialog;
use stfs::vfs::VfsPath;
use xcontent::XContentPackage;

#[cfg(target_arch = "wasm32")]
use eframe::wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use eframe::wasm_bindgen::{
	self,
};
use zip::write::SimpleFileOptions;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
	fn download_file(file: &web_sys::File);
}

#[derive(Debug)]
struct XContentPackageReference {
	data: Arc<Vec<u8>>,
	parsed: XContentPackage,
	fs: VfsPath,
}

enum BackgroundTaskMessage {
	StfsPackageRead(PathBuf, Arc<XContentPackageReference>),
	ZipFileUpdate(String),
	ZipDone,
}

/// We derive Deserialize/Serialize so we can persist app state on shutdown.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)] // if we add new fields, give them default values when deserializing old state
pub struct AccelerationApp {
	active_stfs_file: Option<PathBuf>,

	#[serde(skip)]
	stfs_package: Option<Arc<XContentPackageReference>>,

	#[serde(skip)]
	stfs_package_display_image: Option<Vec<u8>>,

	#[serde(skip)]
	stfs_package_title_image: Option<Vec<u8>>,

	#[serde(skip)]
	clipboard: ClipboardContext,

	#[serde(skip)]
	send: Sender<BackgroundTaskMessage>,

	#[serde(skip)]
	recv: Receiver<BackgroundTaskMessage>,

	#[serde(skip)]
	status_message: Option<String>,

	#[serde(skip)]
	package_files: Vec<StfsFileModel>,
}

#[derive(Debug)]
struct StfsFileModel {
	name: String,
	path: String,
	size: String,
	file_ref: VfsPath,
}

impl Default for AccelerationApp {
	fn default() -> Self {
		let (send, recv) = channel();
		Self {
			active_stfs_file: None,
			stfs_package: None,
			stfs_package_display_image: None,
			stfs_package_title_image: None,
			clipboard: ClipboardProvider::new().unwrap(),
			send,
			recv,
			status_message: None,
			package_files: Vec::new(),
		}
	}
}

impl AccelerationApp {
	/// Called once before the first frame.
	pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
		// This is also where you can customized the look at feel of egui using
		// `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

		// Load previous app state (if any).
		// Note that you must enable the `persistence` feature for this to work.
		if let Some(storage) = cc.storage {
			return eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default();
		}

		Default::default()
	}
}

async fn open_stfs_package(sender: Sender<BackgroundTaskMessage>) -> anyhow::Result<()> {
	let task = AsyncFileDialog::new().pick_file();
	if let Some(file) = task.await {
		#[cfg(not(target_arch = "wasm32"))]
		let file_path = file.path().to_owned();
		#[cfg(target_arch = "wasm32")]
		let file_path = PathBuf::from(file.file_name());

		let data = Arc::new(file.read().await);
		let parsed_package = XContentPackage::try_from(data.as_slice())?;
		let fs = parsed_package.to_vfs_path(Arc::clone(&data));
		let package_ref = XContentPackageReference { data, parsed: parsed_package, fs };

		sender
			.send(BackgroundTaskMessage::StfsPackageRead(file_path, Arc::new(package_ref)))
			.expect("failed to send parsed STFS package to main thread");
	}
	Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn save_file<'a>(file: VfsPath) -> anyhow::Result<()> {
	if let Some(path) = FileDialog::new().set_file_name(file.filename().as_str()).save_file() {
		let mut out_file = std::fs::File::create(path).expect("failed to create output file");
		std::io::copy(&mut file.open_file()?, &mut out_file)?;
	}

	Ok(())
}

#[cfg(target_arch = "wasm32")]
fn save_file<'a>(file: StfsFileEntry, stfs_package: &'a StfsPackage<'a>) {
	let mut out = Vec::with_capacity(file.file_size);
	stfs_package.extract_file(&mut out, &file).expect("failed to save file");

	unsafe {
		download_file(gloo_file::File::new(file.name.as_str(), out.as_slice()).as_ref());
	}
}

#[cfg(not(target_arch = "wasm32"))]
fn extract_all<'a>(root: VfsPath) -> anyhow::Result<()> {
	use std::fs::File;

	if let Some(folder_root) = FileDialog::new().set_file_name("GAME_NAME_HERE").pick_folder() {
		// We're extracting a dir
		for file in root.walk_dir()? {
			let file = file?;
			let target_path = folder_root.join(&file.as_str().strip_prefix(root.parent().as_str()).unwrap()[1..]);
			if file.is_dir()? {
				std::fs::create_dir_all(&target_path)?;
			} else {
				let mut out_file = File::create(&target_path)?;
				println!("writing output file: {:?}, {:?}", target_path, file.metadata()?);
				std::io::copy(&mut file.open_file()?, &mut out_file)?;
			}
		}
	}

	Ok(())
}

fn create_zip<'a>(path: VfsPath, sender: Sender<BackgroundTaskMessage>) -> anyhow::Result<Vec<u8>> {
	let mut zip_contents = Vec::new();
	let writer = Cursor::new(&mut zip_contents);
	let mut zip = zip::ZipWriter::new(writer);
	let options =
		SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).unix_permissions(0o755);

	for file in path.read_dir()? {
		let path_str = file.as_str().to_owned();
		match file.metadata()?.file_type {
			stfs::vfs::VfsFileType::File => {
				zip.start_file(path_str.clone(), options)?;
				let mut reader = file.open_file()?;
				std::io::copy(&mut reader, &mut zip)?;
			}
			stfs::vfs::VfsFileType::Directory => {
				zip.add_directory(path_str.clone(), options)?;
			}
		}

		sender.send(BackgroundTaskMessage::ZipFileUpdate(path_str));
	}

	zip.finish().expect("failed to finish zip");
	drop(zip);

	sender.send(BackgroundTaskMessage::ZipDone);

	Ok(zip_contents)
}

#[cfg(not(target_arch = "wasm32"))]
fn save_as_zip(path: VfsPath, sender: Sender<BackgroundTaskMessage>) -> anyhow::Result<()> {
	if let Some(zip_path) = FileDialog::new().set_file_name("package.zip").save_file() {
		std::fs::write(zip_path, create_zip(path, sender)?.as_slice()).expect("failed to write out zip file");
	}

	Ok(())
}

#[cfg(target_arch = "wasm32")]
fn save_as_zip<'a>(stfs_package: &'a StfsPackage<'a>, sender: Sender<BackgroundTaskMessage>) {
	let contents = create_zip(stfs_package, sender);
	unsafe {
		download_file(
			gloo_file::File::new(
				format!("{}.zip", stfs_package.header.display_name.as_str()).as_str(),
				contents.as_slice(),
			)
			.as_ref(),
		);
	}
}

fn human_readable_size(size: u64) -> String {
	const KB: u64 = 1024;
	const MB: u64 = KB * KB;
	const GB: u64 = KB * KB * KB;

	const BYTES_END: u64 = KB - 1;
	const KB_END: u64 = MB - 1;
	const MB_END: u64 = GB - 1;

	match size {
		0..=BYTES_END => {
			format!("{} Bytes", size)
		}
		KB..=KB_END => {
			format!("{} KB", size / KB)
		}
		MB..=MB_END => {
			format!("{} MB", size / MB)
		}
		_default => {
			format!("{} GB", size / GB)
		}
	}
}

impl eframe::App for AccelerationApp {
	/// Called by the frame work to save state before shutdown.
	fn save(&mut self, storage: &mut dyn eframe::Storage) {
		eframe::set_value(storage, eframe::APP_KEY, self);
	}

	/// Called each time the UI needs repainting, which may be many times per second.
	/// Put your widgets into a `SidePanel`, `TopPanel`, `CentralPanel`, `Window` or `Area`.
	fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
		let Self {
			active_stfs_file,
			stfs_package,
			stfs_package_display_image,
			stfs_package_title_image,
			clipboard,
			send,
			recv,
			status_message,
			package_files,
		} = self;

		egui_extras::install_image_loaders(ctx);

		// We open the file on another thread. Check if that thread has sent us any data yet.
		match recv.try_recv() {
			Ok(BackgroundTaskMessage::StfsPackageRead(file_path, received_stfs_package)) => {
				let root = &received_stfs_package.fs;
				for file in root.walk_dir().unwrap() {
					let file = file.unwrap();
					package_files.push(StfsFileModel {
						name: file.filename(),
						path: file.as_str().to_owned(),
						size: human_readable_size(file.metadata().unwrap().len),
						file_ref: file,
					});
				}
				// let display_image = ImageReader::new(Cursor::new(
				// 	received_stfs_package.parsed.header.metadata.thumbnail_image.as_slice(),
				// ))
				// .decode()
				// .expect("display image is not a PNG");
				// let size = [display_image.width() as _, display_image.height() as _];
				// let display_image_buffer = display_image.to_rgba8();
				// let pixels = display_image_buffer.as_flat_samples();

				// *stfs_package_display_image = Some(ColorImage::from_rgba_unmultiplied(size, pixels.as_slice()));
				*stfs_package_display_image =
					Some(received_stfs_package.parsed.header.metadata.thumbnail_image.clone());

				// let title_image =
				// 	ImageReader::new(Cursor::new(received_stfs_package.parsed.header.metadata.title_image.as_slice()))
				// 		.decode()
				// 		.expect("title image is not a PNG");
				// let size = [title_image.width() as _, title_image.height() as _];
				// let title_image_buffer = title_image.to_rgba8();
				// let pixels = title_image_buffer.as_flat_samples();

				// *stfs_package_title_image = Some(ColorImage::from_rgba_unmultiplied(size, pixels.as_slice()));
				*stfs_package_title_image = Some(received_stfs_package.parsed.header.metadata.title_image.clone());

				*active_stfs_file = Some(file_path);
				*stfs_package = Some(received_stfs_package);
			}
			Ok(BackgroundTaskMessage::ZipFileUpdate(path)) => {
				*status_message = Some(format!("Extracting {}", path));
			}
			Ok(BackgroundTaskMessage::ZipDone) => {
				*status_message = None;
			}
			Err(_) => {
				// Do nothing
			}
		}

		if let Some(file_path) = active_stfs_file.as_ref() {

			// frame.set_window_title(&format!("acceleration - {:?}", file_path));
		}

		// Examples of how to create different panels and windows.
		// Pick whichever suits you.
		// Tip: a good default choice is to just keep the `CentralPanel`.
		// For inspiration and more examples, go to https://emilk.github.io/egui

		egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
			// The top panel is often a good place for a menu bar:
			egui::menu::bar(ui, |ui| {
				ui.menu_button("File", |ui| {
					if ui.button("Open").clicked() {
						let task = open_stfs_package(send.clone());

						#[cfg(target_arch = "wasm32")]
						wasm_bindgen_futures::spawn_local(task);
						#[cfg(not(target_arch = "wasm32"))]
						std::thread::spawn(move || futures::executor::block_on(task));

						ui.close_menu();
					}
					if let Some(stfs_package) = stfs_package.as_ref() {
						#[cfg(not(target_arch = "wasm32"))]
						if ui.button("Extract All").clicked() {
							extract_all(stfs_package.fs.clone());

							ui.close_menu();
						}
						if ui.button("Save As Zip").clicked() {
							let stfs_package = stfs_package.clone();
							let sender = send.clone();
							info!("Spawning thread...");

							#[cfg(target_arch = "wasm32")]
							// wasm_bindgen_futures::spawn_local(async move {
							save_as_zip(stfs_package.read().borrow_parsed_stfs_package().as_ref().unwrap(), sender);
							// });

							#[cfg(not(target_arch = "wasm32"))]
							std::thread::spawn(move || save_as_zip(stfs_package.fs.clone(), sender));

							ui.close_menu();
						}
					}
					if ui.button("Quit").clicked() {
						ctx.send_viewport_cmd(egui::ViewportCommand::Close);
					}
				});
			});
		});

		egui::SidePanel::left("side_panel").show(ctx, |ui| {
			ui.heading("STFS Metadata");

			if let Some(image) = stfs_package_display_image {
				// TODO: Cache
				ui.add(Image::new(ImageSource::Bytes { uri: "bytes://asdf".into(), bytes: image.clone().into() }));
			}

			if let Some(image) = stfs_package_title_image {
				// TODO: Cache
				ui.add(Image::new(ImageSource::Bytes { uri: "bytes://asdf2".into(), bytes: image.clone().into() }));
			}

			if let Some(stfs_package_ref) = stfs_package.as_ref() {
				let parsed_package = &stfs_package_ref.parsed;
				let metadata = &parsed_package.header.metadata;

				ui.horizontal(|ui| {
					ui.label("Name:");
					let display_name = metadata.display_name[0].to_string();
					if ui.add(Label::new(&display_name).sense(Sense::click())).double_clicked() {
						let _ = clipboard.set_contents(display_name);
					}
				});

				ui.horizontal(|ui| {
					ui.label("Description:");
					let description = metadata.display_description[0].to_string();
					if ui.add(Label::new(&description).wrap(true).sense(Sense::click())).double_clicked() {
						let _ = clipboard.set_contents(description);
					}
				});

				ui.horizontal(|ui| {
					ui.label("Title ID:");
					let label_str = format!("{:08X}", metadata.title_id);

					if ui.add(Label::new(&label_str).sense(Sense::click())).double_clicked() {
						let _ = clipboard.set_contents(label_str);
					}
				});

				ui.horizontal(|ui| {
					ui.label("Profile ID:");
					let profile_id = format!("{:016X}", metadata.creator_xuid);
					if ui.add(Label::new(&profile_id).sense(Sense::click())).double_clicked() {
						let _ = clipboard.set_contents(profile_id);
					}
				});

				ui.horizontal(|ui| {
					ui.label("Console ID:");
					let console_id = metadata
						.console_id
						.iter()
						.fold(String::new(), |display_str, b| display_str + &format!("{:02x}", *b));
					if ui.add(Label::new(&console_id).sense(Sense::click())).double_clicked() {
						let _ = clipboard.set_contents(console_id);
					}
				});

				ui.horizontal(|ui| {
					ui.label("Content Type:");
					let content_type = format!("{:?}", metadata.content_type);
					if ui.add(Label::new(&content_type).sense(Sense::click())).double_clicked() {
						let _ = clipboard.set_contents(content_type);
					}
				});
			}
		});

		egui::CentralPanel::default().show(ctx, |ui| {
			use egui_extras::TableBuilder;

			ui.vertical(|ui| {
				if let Some(status_message) = status_message.as_ref() {
					ui.horizontal(|ui| {
						// ui.spacing_mut().item_spacing.x = 0.0;
						ui.add(Spinner::new());

						ui.label(status_message);
					});
				}

				TableBuilder::new(ui)
					.striped(true)
					.cell_layout(egui::Layout::left_to_right(egui::Align::Center).with_cross_align(egui::Align::Center))
					.column(Column::initial(60.0).at_least(40.0))
					.column(Column::initial(60.0).at_least(40.0))
					.column(Column::remainder().at_least(60.0))
					.resizable(true)
					.header(20.0, |mut header| {
						header.col(|ui| {
							ui.heading("Name");
						});
						header.col(|ui| {
							ui.heading("Size");
						});
						header.col(|ui| {
							ui.heading("Path");
						});
					})
					.body(|mut body| {
						if let Some(stfs_package) = &stfs_package {
							for file in package_files {
								body.row(18.0, |mut row| {
									let do_extract = |ui: &mut Ui| {
										if ui.button("Extract").clicked() {
											save_file(file.file_ref.clone());

											ui.close_menu();
										}
									};

									let (_, response) = row.col(|ui| {
										ui.label(file.name.as_str()).context_menu(do_extract);
									});
									response.context_menu(do_extract);

									row.col(|ui| {
										ui.label(file.size.as_str());
									});

									row.col(|ui| {
										ui.label(file.path.as_str());
									});
								})
							}
						}
					});

				// ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
			});
		});
	}
}
