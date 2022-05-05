use std::{fs::File, path::PathBuf};

use clipboard::{ClipboardContext, ClipboardProvider};
use egui::{Label, Sense, TextBuffer};
use egui_extras::RetainedImage;
use log::{debug, info};
use memmap::MmapOptions;
use ouroboros::self_referencing;
use rfd::FileDialog;
use stfs::StfsPackage;

/// We derive Deserialize/Serialize so we can persist app state on shutdown.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)] // if we add new fields, give them default values when deserializing old state
pub struct AccelerationApp {
    // Example stuff:
    label: String,

    active_stfs_file: Option<PathBuf>,

    #[serde(skip)]
    stfs_package: Option<StfsPackageReference>,

    #[serde(skip)]
    stfs_package_display_image: Option<RetainedImage>,

    #[serde(skip)]
    stfs_package_title_image: Option<RetainedImage>,

    #[serde(skip)]
    clipboard: ClipboardContext,
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
        Self {
            // Example stuff:
            label: "Hello World!".to_owned(),
            active_stfs_file: None,
            stfs_package: None,
            stfs_package_display_image: None,
            stfs_package_title_image: None,
            clipboard: ClipboardProvider::new().unwrap(),
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

fn open_stfs_package(stfs_package: &mut Option<StfsPackageReference>) -> Result<PathBuf, ()> {
    info!("Showing dialog");

    if let Some(file) = FileDialog::new().pick_file() {
        if let Ok(file_data) = std::fs::read(&file) {
            let package_reference = StfsPackageReferenceBuilder {
                stfs_package_data: file_data,
                parsed_stfs_package_builder: |package_data| {
                    StfsPackage::try_from(package_data.as_slice())
                },
            }
            .build();

            if package_reference.borrow_parsed_stfs_package().is_ok() {
                *stfs_package = Some(package_reference);
                return Ok(file);
            }
        }
    }

    Err(())
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
            label,
            active_stfs_file,
            stfs_package,
            stfs_package_display_image,
            stfs_package_title_image,
            clipboard,
        } = self;

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
                        if let Ok(file_path) = open_stfs_package(stfs_package) {
                            *active_stfs_file = Some(file_path);

                            info!("set the active STFS file");
                            info!("{}", stfs_package.is_some());
                            if let Some(parsed_package) = stfs_package
                                .as_ref()
                                .map(|package| package.borrow_parsed_stfs_package().as_ref().ok())
                                .flatten()
                            {
                                info!("Parsing images");
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
                            }
                        }

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

            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label("powered by ");
                    ui.hyperlink_to("egui", "https://github.com/emilk/egui");
                    ui.label(" and ");
                    ui.hyperlink_to("eframe", "https://github.com/emilk/egui/tree/master/eframe");
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            // The central panel the region left after adding TopPanel's and SidePanel's

            ui.heading("eframe template");
            ui.hyperlink("https://github.com/emilk/eframe_template");
            ui.add(egui::github_link_file!(
                "https://github.com/emilk/eframe_template/blob/master/",
                "Source code."
            ));
            egui::warn_if_debug_build(ui);
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
