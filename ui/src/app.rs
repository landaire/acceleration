use std::{
    cell::RefCell,
    fs::File,
    ops::Deref,
    path::PathBuf,
    pin::Pin,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc,
    },
};

use clipboard::{ClipboardContext, ClipboardProvider};
use egui::{Label, Sense, TextBuffer};
use egui_extras::RetainedImage;
use log::{debug, info};
use ouroboros::self_referencing;
use rfd::{AsyncFileDialog, FileDialog};
use stfs::{StfsEntry, StfsFileEntry, StfsPackage};

/// We derive Deserialize/Serialize so we can persist app state on shutdown.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)] // if we add new fields, give them default values when deserializing old state
pub struct AccelerationApp {
    active_stfs_file: Option<PathBuf>,

    #[serde(skip)]
    stfs_package: Option<StfsPackageReference>,

    #[serde(skip)]
    stfs_package_display_image: Option<RetainedImage>,

    #[serde(skip)]
    stfs_package_title_image: Option<RetainedImage>,

    #[serde(skip)]
    clipboard: ClipboardContext,

    #[serde(skip)]
    send: Sender<(PathBuf, StfsPackageReference)>,

    #[serde(skip)]
    recv: Receiver<(PathBuf, StfsPackageReference)>,
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

    package_files: RefCell<Vec<StfsFileModel>>,
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

async fn open_stfs_package(sender: Sender<(PathBuf, StfsPackageReference)>) {
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
            package_files: RefCell::new(Default::default()),
        }
        .build();

        if package_reference.borrow_parsed_stfs_package().is_ok() {
            sender
                .send((file_path, package_reference))
                .expect("failed to send parsed STFS package to main thread");
        }
    }
}

fn save_file<'a>(file: StfsFileEntry, stfs_package: &StfsPackage<'a>) {
    if let Some(path) = FileDialog::new()
        .set_file_name(file.name.as_str())
        .save_file()
    {
        stfs_package
            .extract_file(path.as_ref(), file)
            .expect("failed to save file");
    }
}

fn extract_all<'a>(stfs_package: &StfsPackage<'a>) {
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

            let arc_file = file.clone();
            let file = file.lock();
            if let StfsEntry::File(entry) = &*file {
                let file_path = path.join(entry.name.as_str());
                let mut target_path = folder_root.join(&path);
                std::fs::create_dir_all(&target_path).expect("failed to create path!");
                target_path.push(entry.name.as_str());

                stfs_package
                    .extract_file(target_path.as_ref(), entry.clone())
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
        } = self;

        // We open the file on another thread. Check if that thread has sent us any data yet.
        if let Ok((file_path, received_stfs_package)) = recv.try_recv() {
            // We have a file!
            *active_stfs_file = Some(file_path);
            if let Some(parsed_package) = received_stfs_package
                .borrow_parsed_stfs_package()
                .as_ref()
                .ok()
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
                        let package_files = received_stfs_package.borrow_package_files();
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
                let package_files = received_stfs_package.borrow_package_files();
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
                if let Ok(parsed_package) = stfs_package_ref.borrow_parsed_stfs_package() {
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

            TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::left_to_right().with_cross_align(egui::Align::Center))
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
                        let package_files = stfs_package.borrow_package_files();
                        let package_files = package_files.borrow();
                        for file in &*package_files {
                            body.row(18.0, |mut row| {
                                row.col(|ui| {
                                    ui.label(file.name.as_str());
                                })
                                .context_menu(|ui| {
                                    if ui.button("Extract").clicked() {
                                        save_file(
                                            file.file_ref.lock().entry().clone(),
                                            stfs_package
                                                .borrow_parsed_stfs_package()
                                                .as_ref()
                                                .unwrap(),
                                        );

                                        ui.close_menu();
                                    }
                                    if ui.button("Extract All").clicked() {
                                        extract_all(
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
