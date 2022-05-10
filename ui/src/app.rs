use std::{
    cell::RefCell,
    fs::File,
    io::{Cursor, Write},
    ops::Deref,
    path::PathBuf,
    pin::Pin,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc,
    },
};

use clipboard::{ClipboardContext, ClipboardProvider};
use egui::{Label, Sense, Spinner, TextBuffer};
use egui_extras::RetainedImage;
use log::{debug, info};
use ouroboros::self_referencing;
use parking_lot::{Mutex, RwLock};
use rfd::{AsyncFileDialog, FileDialog};
use stfs::{StfsEntry, StfsFileEntry, StfsPackage};
use zip::write::FileOptions;

enum BackgroundTaskMessage {
    StfsPackageRead(PathBuf, Arc<RwLock<StfsPackageReference>>),
    ZipFileUpdate(PathBuf),
    ZipDone,
}

/// We derive Deserialize/Serialize so we can persist app state on shutdown.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)] // if we add new fields, give them default values when deserializing old state
pub struct AccelerationApp {
    active_stfs_file: Option<PathBuf>,

    #[serde(skip)]
    stfs_package: Option<Arc<RwLock<StfsPackageReference>>>,

    #[serde(skip)]
    stfs_package_display_image: Option<RetainedImage>,

    #[serde(skip)]
    stfs_package_title_image: Option<RetainedImage>,

    #[serde(skip)]
    clipboard: ClipboardContext,

    #[serde(skip)]
    send: Sender<BackgroundTaskMessage>,

    #[serde(skip)]
    recv: Receiver<BackgroundTaskMessage>,

    #[serde(skip)]
    status_message: Option<String>,

    #[serde(skip)]
    package_files: RefCell<Vec<StfsFileModel>>,
}

#[derive(Debug)]
struct StfsFileModel {
    name: String,
    path: PathBuf,
    size: String,
    file_ref: stfs::StfsEntryRef,
}

#[self_referencing]
struct StfsPackageReference {
    stfs_package_data: Vec<u8>,

    #[borrows(stfs_package_data)]
    #[covariant]
    parsed_stfs_package: Result<StfsPackage<'this>, stfs::StfsError>,
}

impl<'package> Default for AccelerationApp {
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
            package_files: RefCell::new(Vec::new()),
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

async fn open_stfs_package(sender: Sender<BackgroundTaskMessage>) {
    let task = AsyncFileDialog::new().pick_file();
    if let Some(file) = task.await {
        #[cfg(not(target_arch = "wasm32"))]
        let file_path = file.path().to_owned();
        #[cfg(target_arch = "wasm32")]
        let file_path = PathBuf::from(file.file_name());

        let file_data = file.read().await;
        let package_reference = StfsPackageReferenceBuilder {
            stfs_package_data: file_data,
            parsed_stfs_package_builder: |package_data| {
                StfsPackage::try_from(package_data.as_slice())
            },
        }
        .build();

        if package_reference.borrow_parsed_stfs_package().is_ok() {
            sender
                .send(BackgroundTaskMessage::StfsPackageRead(
                    file_path,
                    Arc::new(RwLock::new(package_reference)),
                ))
                .expect("failed to send parsed STFS package to main thread");
        }
    }
}

fn save_file<'a>(file: StfsFileEntry, stfs_package: &'a StfsPackage<'a>) {
    if let Some(path) = FileDialog::new()
        .set_file_name(file.name.as_str())
        .save_file()
    {
        let mut out_file = std::fs::File::create(path).expect("failed to create output file");
        stfs_package
            .extract_file(&mut out_file, &file)
            .expect("failed to save file");
    }
}

fn extract_all<'a>(stfs_package: &'a StfsPackage<'a>) {
    if let Some(folder_root) = FileDialog::new()
        .set_file_name(stfs_package.header.display_name.as_str())
        .pick_folder()
    {
        let mut path = PathBuf::new();
        let mut queue = Vec::with_capacity(256);
        if let StfsEntry::Folder { entry: _, files } = &*stfs_package.files.lock() {
            queue.extend(std::iter::repeat(0usize).zip(files.iter().cloned()));
        }

        let mut last_depth = 0;
        while let Some((depth, file)) = queue.pop() {
            if depth < last_depth {
                path.pop();
                last_depth -= 1;
            }

            let file = file.lock();
            if let StfsEntry::File(entry) = &*file {
                let file_path = path.join(entry.name.as_str());
                let mut directory_path = folder_root.join(&path);
                std::fs::create_dir_all(&directory_path).expect("failed to create path!");
                directory_path.push(entry.name.as_str());

                let mut file =
                    std::fs::File::create(file_path).expect("failed to create output file");

                stfs_package
                    .extract_file(&mut file, entry)
                    .expect("failed to save file");
            }

            if let StfsEntry::Folder { entry, files } = &*file {
                path.push(entry.name.as_str());
                queue.extend(std::iter::repeat(depth + 1).zip(files.iter().cloned()));
                last_depth += 1;
            }
        }
    }
}

fn create_zip<'a>(
    stfs_package: &'a StfsPackage<'a>,
    sender: Sender<BackgroundTaskMessage>,
) -> Vec<u8> {
    let mut zip_contents = Vec::new();
    let writer = Cursor::new(&mut zip_contents);
    let mut zip = zip::ZipWriter::new(writer);
    let options = FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflate)
        .unix_permissions(0o755);

    let mut path = PathBuf::new();
    let mut queue = Vec::with_capacity(256);
    if let StfsEntry::Folder { entry: _, files } = &*stfs_package.files.lock() {
        queue.extend(std::iter::repeat(0usize).zip(files.iter().cloned()));
    }

    let mut last_depth = 0;
    let mut buffer = Vec::new();
    while let Some((depth, file)) = queue.pop() {
        if depth < last_depth {
            path.pop();
            last_depth -= 1;
        }

        let file = file.lock();
        if let StfsEntry::File(entry) = &*file {
            let file_path = path.join(entry.name.as_str());
            sender
                .send(BackgroundTaskMessage::ZipFileUpdate(file_path.clone()))
                .expect("failed to send file update");
            info!("Adding file {:?} to zip", file_path);

            zip.start_file(file_path.as_os_str().to_str().unwrap(), options)
                .expect("failed to add file to zip");

            stfs_package
                .extract_file(&mut buffer, entry)
                .expect("failed to extract file");
            zip.write_all(buffer.as_slice())
                .expect("failed to write file to zip");

            buffer.clear();
        }

        if let StfsEntry::Folder { entry, files } = &*file {
            path.push(entry.name.as_str());
            info!("Adding folder {:?} to zip", path);
            zip.add_directory(path.as_os_str().to_str().unwrap(), options)
                .expect("failed to create directory");
            queue.extend(std::iter::repeat(depth + 1).zip(files.iter().cloned()));
            last_depth += 1;
        }
    }

    zip.finish().expect("failed to finish zip");
    drop(zip);

    sender.send(BackgroundTaskMessage::ZipDone);

    zip_contents
}

fn save_as_zip<'a>(stfs_package: &'a StfsPackage<'a>, sender: Sender<BackgroundTaskMessage>) {
    if let Some(zip_path) = FileDialog::new()
        .set_file_name(format!("{}.zip", stfs_package.header.display_name).as_str())
        .save_file()
    {
        std::fs::write(zip_path, create_zip(stfs_package, sender).as_slice())
            .expect("failed to write out zip file");
    }
}

fn human_readable_size(size: usize) -> String {
    const KB: usize = 1024;
    const MB: usize = KB * KB;
    const GB: usize = KB * KB * KB;

    const BYTES_END: usize = KB - 1;
    const KB_END: usize = MB - 1;
    const MB_END: usize = GB - 1;

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
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
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

        // We open the file on another thread. Check if that thread has sent us any data yet.
        match recv.try_recv() {
            Ok(BackgroundTaskMessage::StfsPackageRead(file_path, received_stfs_package)) => {
                // We have a file!
                *active_stfs_file = Some(file_path);
                if let Ok(parsed_package) = received_stfs_package
                    .read()
                    .borrow_parsed_stfs_package()
                    .as_ref()
                {
                    *stfs_package_display_image = RetainedImage::from_image_bytes(
                        "display_image",
                        parsed_package.header.thumbnail_image,
                    )
                    .ok();

                    *stfs_package_display_image = RetainedImage::from_image_bytes(
                        "display_image",
                        parsed_package.header.title_image,
                    )
                    .ok();

                    // Populate the files
                    let mut path = PathBuf::new();
                    let mut queue = Vec::with_capacity(256);
                    if let StfsEntry::Folder { entry: _, files } = &*parsed_package.files.lock() {
                        queue.extend(std::iter::repeat(0usize).zip(files.iter().cloned()));
                    }

                    let mut last_depth = 0;
                    while let Some((depth, file)) = queue.pop() {
                        if depth < last_depth {
                            path.pop();
                            last_depth -= 1;
                        }

                        let arc_file = file.clone();
                        let file = file.lock();
                        if let StfsEntry::File(entry) = &*file {
                            let mut package_files = package_files.borrow_mut();
                            package_files.push(StfsFileModel {
                                name: entry.name.clone(),
                                path: path.join(entry.name.as_str()),
                                size: human_readable_size(entry.file_size),
                                file_ref: arc_file,
                            });
                        }

                        if let StfsEntry::Folder { entry, files } = &*file {
                            path.push(entry.name.as_str());
                            queue.extend(std::iter::repeat(depth + 1).zip(files.iter().cloned()));
                            last_depth += 1;
                        }
                    }

                    // Sort the package files by their entry ID
                    let mut package_files = package_files.borrow_mut();
                    package_files.sort_by(|a, b| {
                        a.file_ref
                            .lock()
                            .entry()
                            .index
                            .cmp(&b.file_ref.lock().entry().index)
                    });
                }

                *stfs_package = Some(received_stfs_package);
            }
            Ok(BackgroundTaskMessage::ZipFileUpdate(path)) => {
                *status_message =
                    Some(format!("Extracting {}", path.as_os_str().to_str().unwrap()));
            }
            Ok(BackgroundTaskMessage::ZipDone) => {
                *status_message = None;
            }
            Err(_) => {
                // Do nothing
            }
        }

        if let Some(file_path) = active_stfs_file.as_ref() {
            frame.set_window_title(&format!("acceleration - {:?}", file_path));
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
                        if ui.button("Extract All").clicked() {
                            extract_all(
                                stfs_package
                                    .read()
                                    .borrow_parsed_stfs_package()
                                    .as_ref()
                                    .unwrap(),
                            );

                            ui.close_menu();
                        }
                        if ui.button("Save As Zip").clicked() {
                            let stfs_package = stfs_package.clone();
                            let sender = send.clone();
                            info!("Spawning thread...");
                            std::thread::spawn(move || {
                                save_as_zip(
                                    stfs_package
                                        .read()
                                        .borrow_parsed_stfs_package()
                                        .as_ref()
                                        .unwrap(),
                                    sender,
                                )
                            });

                            ui.close_menu();
                        }
                    }
                    if ui.button("Quit").clicked() {
                        frame.quit();
                    }
                });
            });
        });

        egui::SidePanel::left("side_panel").show(ctx, |ui| {
            ui.heading("STFS Metadata");

            if let Some(image) = stfs_package_display_image {
                image.show_max_size(ui, ui.available_size());
            }

            if let Some(image) = stfs_package_title_image {
                image.show_max_size(ui, ui.available_size());
            }

            if let Some(stfs_package_ref) = stfs_package.as_ref() {
                if let Ok(parsed_package) = stfs_package_ref.read().borrow_parsed_stfs_package() {
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        if ui
                            .add(
                                Label::new(parsed_package.header.display_name.as_str())
                                    .sense(Sense::click()),
                            )
                            .double_clicked()
                        {
                            let _ = clipboard
                                .set_contents(parsed_package.header.display_name.to_owned());
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Description:");
                        if ui
                            .add(
                                Label::new(parsed_package.header.display_description.as_str())
                                    .wrap(true)
                                    .sense(Sense::click()),
                            )
                            .double_clicked()
                        {
                            let _ = clipboard
                                .set_contents(parsed_package.header.display_description.to_owned());
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Title ID:");
                        let label_str = format!("{:#X}", parsed_package.header.title_id);

                        if ui
                            .add(Label::new(&label_str).sense(Sense::click()))
                            .double_clicked()
                        {
                            let _ = clipboard.set_contents(label_str);
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Profile ID:");
                        let profile_id = parsed_package
                            .header
                            .profile_id
                            .iter()
                            .fold(String::new(), |display_str, b| {
                                display_str + &format!("{:02x}", *b)
                            });
                        if ui
                            .add(Label::new(&profile_id).sense(Sense::click()))
                            .double_clicked()
                        {
                            let _ = clipboard.set_contents(profile_id);
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Console ID:");
                        let console_id = parsed_package
                            .header
                            .console_id
                            .iter()
                            .fold(String::new(), |display_str, b| {
                                display_str + &format!("{:02x}", *b)
                            });
                        if ui
                            .add(Label::new(&console_id).sense(Sense::click()))
                            .double_clicked()
                        {
                            let _ = clipboard.set_contents(console_id);
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Content Type:");
                        let content_type = format!("{:?}", parsed_package.header.content_type);
                        if ui
                            .add(Label::new(&content_type).sense(Sense::click()))
                            .double_clicked()
                        {
                            let _ = clipboard.set_contents(content_type);
                        }
                    });
                }
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            use egui_extras::{Size, TableBuilder};

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
                    .cell_layout(
                        egui::Layout::left_to_right().with_cross_align(egui::Align::Center),
                    )
                    .column(Size::initial(60.0).at_least(40.0))
                    .column(Size::initial(60.0).at_least(40.0))
                    .column(Size::remainder().at_least(60.0))
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
                        if let Some(stfs_package) = stfs_package {
                            let package_files = package_files.borrow();
                            for file in &*package_files {
                                body.row(18.0, |mut row| {
                                    row.col(|ui| {
                                        ui.label(file.name.as_str());
                                    })
                                    .context_menu(|ui| {
                                        if ui.button("Extract").clicked() {
                                            let stfs_package = stfs_package.read();
                                            save_file(
                                                file.file_ref.lock().entry().clone(),
                                                stfs_package
                                                    .borrow_parsed_stfs_package()
                                                    .as_ref()
                                                    .unwrap(),
                                            );

                                            ui.close_menu();
                                        }
                                    });

                                    row.col(|ui| {
                                        ui.label(file.size.as_str());
                                    });

                                    row.col(|ui| {
                                        ui.label(file.path.as_os_str().to_str().unwrap());
                                    });
                                })
                            }
                        }
                    });

                // ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            });
        });

        if false {
            egui::Window::new("Window").show(ctx, |ui| {
                ui.label("Windows can be moved by dragging them.");
                ui.label("They are automatically sized based on contents.");
                ui.label("You can turn on resizing and scrolling if you like.");
                ui.label("You would normally chose either panels OR windows.");
            });
        }
    }
}
