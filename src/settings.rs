//! User preferences, saved between runs, and the tabbed Settings window.

use eframe::egui::{self, Color32};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Theme {
    System,
    Dark,
    Light,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SizeUnits {
    /// KiB/MiB/GiB, powers of 1024 (what Windows Explorer shows, labelled KB/MB/GB).
    Binary,
    /// kB/MB/GB, powers of 1000 (what drive makers and macOS use).
    Decimal,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // General
    pub theme: Theme,
    pub units: SizeUnits,
    pub stripes: bool,
    pub live_tree: bool,

    // Filtering (applied while scanning)
    /// One path per line; these folders and everything under them are skipped.
    pub exclude_folders: String,
    /// One glob per line (`*.tmp`, `~$*`); matching files are skipped.
    pub exclude_files: String,
    /// One glob per line; if any are given, only matching files are counted.
    pub include_files: String,
    /// Skip files smaller than this many bytes (0 = off).
    pub min_file_size: u64,
    /// Skip files not modified for this many days (0 = off).
    pub max_age_days: u32,

    // Folder list
    pub col_percent: bool,
    pub col_files: bool,
    pub col_modified: bool,

    // Treemap
    pub brightness: f32,
    pub cushion: f32,
    pub height: f32,
    pub scale: f32,
    pub light_x: f32,
    pub light_y: f32,
    pub show_grid: bool,
    /// Write file names on treemap tiles big enough to hold them.
    pub show_labels: bool,
    pub grid_color: Color32,
    pub selection_color: Color32,

    // Advanced
    pub one_file_system: bool,
    pub skip_hidden: bool,
    /// Scanner threads (0 = one per CPU core).
    pub threads: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            units: SizeUnits::Binary,
            stripes: true,
            live_tree: true,
            exclude_folders: String::new(),
            exclude_files: String::new(),
            include_files: String::new(),
            min_file_size: 0,
            max_age_days: 0,
            col_percent: true,
            col_files: true,
            col_modified: true,
            brightness: 0.88,
            cushion: 0.87,
            height: 0.38,
            scale: 0.91,
            light_x: -1.0,
            light_y: -1.0,
            show_grid: false,
            show_labels: true,
            grid_color: Color32::BLACK,
            selection_color: Color32::WHITE,
            one_file_system: true,
            skip_hidden: false,
            threads: 0,
        }
    }
}

impl Settings {
    /// Settings that change what a scan finds (so a rescan is needed to see them).
    pub fn scan_key(&self) -> impl PartialEq + use<> {
        (
            self.exclude_folders.clone(),
            self.exclude_files.clone(),
            self.include_files.clone(),
            self.min_file_size,
            self.max_age_days,
            self.one_file_system,
            self.skip_hidden,
        )
    }

    /// Settings that change how the treemap looks.
    pub fn treemap_key(&self) -> [u32; 9] {
        [
            self.brightness.to_bits(),
            self.cushion.to_bits(),
            self.height.to_bits(),
            self.scale.to_bits(),
            self.light_x.to_bits(),
            self.light_y.to_bits(),
            self.show_grid as u32,
            u32::from_ne_bytes(self.grid_color.to_array()),
            0,
        ]
    }

    pub fn apply_theme(&self, ctx: &egui::Context) {
        ctx.set_theme(match self.theme {
            Theme::System => egui::ThemePreference::System,
            Theme::Dark => egui::ThemePreference::Dark,
            Theme::Light => egui::ThemePreference::Light,
        });
    }
}

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Tab {
    #[default]
    General,
    Filtering,
    FolderList,
    Treemap,
    Advanced,
}

/// The Settings window. Edits a draft; returns true when OK or Apply was pressed.
pub struct Dialog {
    pub open: bool,
    pub draft: Settings,
    tab: Tab,
}

impl Default for Dialog {
    fn default() -> Self {
        Self { open: false, draft: Settings::default(), tab: Tab::General }
    }
}

pub enum Outcome {
    None,
    /// Draft changed; preview it (treemap look) without committing.
    Preview,
    Apply,
}

impl Dialog {
    pub fn open(&mut self, current: &Settings) {
        self.draft = current.clone();
        self.open = true;
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Outcome {
        if !self.open {
            return Outcome::None;
        }
        let before = self.draft.clone();
        let mut outcome = Outcome::None;
        let mut open = true;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (t, label) in [
                        (Tab::General, "General"),
                        (Tab::Filtering, "Filtering"),
                        (Tab::FolderList, "Folder list"),
                        (Tab::Treemap, "Treemap"),
                        (Tab::Advanced, "Advanced"),
                    ] {
                        ui.selectable_value(&mut self.tab, t, label);
                    }
                });
                ui.separator();
                let d = &mut self.draft;
                ui.vertical(|ui| {
                    // Same height on every tab, so the buttons don't jump around.
                    ui.set_min_height(320.0);
                    match self.tab {
                        Tab::General => general(ui, d),
                        Tab::Filtering => filtering(ui, d),
                        Tab::FolderList => folder_list(ui, d),
                        Tab::Treemap => treemap(ui, d),
                        Tab::Advanced => advanced(ui, d),
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Reset all").on_hover_text("Restore every setting to its default").clicked() {
                        *d = Settings::default();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Apply").clicked() {
                            outcome = Outcome::Apply;
                        }
                        if ui.button("Cancel").clicked() {
                            self.open = false;
                        }
                        if ui.button("  OK  ").clicked() {
                            outcome = Outcome::Apply;
                            self.open = false;
                        }
                    });
                });
            });
        if !open {
            self.open = false;
        }
        if matches!(outcome, Outcome::None) && self.draft != before {
            outcome = Outcome::Preview;
        }
        outcome
    }
}

fn general(ui: &mut egui::Ui, d: &mut Settings) {
    ui.heading("Appearance");
    ui.horizontal(|ui| {
        ui.label("Theme:");
        ui.radio_value(&mut d.theme, Theme::System, "Follow system");
        ui.radio_value(&mut d.theme, Theme::Dark, "Dark");
        ui.radio_value(&mut d.theme, Theme::Light, "Light");
    });
    ui.horizontal(|ui| {
        ui.label("Sizes:");
        ui.radio_value(&mut d.units, SizeUnits::Binary, "Binary (1 KB = 1024 bytes, like Explorer)");
        ui.radio_value(&mut d.units, SizeUnits::Decimal, "Decimal (1 kB = 1000 bytes)");
    });
    ui.checkbox(&mut d.stripes, "Striped rows in lists");
    ui.add_space(8.0);
    ui.heading("Scanning");
    ui.checkbox(&mut d.live_tree, "Show folders and files as they are found");
}

fn filtering(ui: &mut egui::Ui, d: &mut Settings) {
    ui.label("These apply to the next scan. One entry per line. Patterns use * and ?, e.g. *.tmp");
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.label("Folders to skip");
            let hint = if cfg!(windows) { "C:\\Windows\\WinSxS" } else { "/proc" };
            ui.add(egui::TextEdit::multiline(&mut d.exclude_folders).desired_rows(9).desired_width(250.0).hint_text(hint));
        });
        ui.vertical(|ui| {
            ui.label("File names to skip");
            ui.add(egui::TextEdit::multiline(&mut d.exclude_files).desired_rows(4).desired_width(250.0).hint_text("*.tmp\n~$*"));
            ui.label("Only count file names matching");
            ui.add(egui::TextEdit::multiline(&mut d.include_files).desired_rows(3).desired_width(250.0).hint_text("(empty = all files)"));
        });
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Skip files smaller than");
        let mut kb = d.min_file_size / 1024;
        if ui.add(egui::DragValue::new(&mut kb).range(0..=u64::MAX / 1024).suffix(" KB")).changed() {
            d.min_file_size = kb * 1024;
        }
        ui.weak("(0 = off)");
    });
    ui.horizontal(|ui| {
        ui.label("Skip files not changed for");
        ui.add(egui::DragValue::new(&mut d.max_age_days).range(0..=36500).suffix(" days"));
        ui.weak("(0 = off)");
    });
}

fn folder_list(ui: &mut egui::Ui, d: &mut Settings) {
    ui.heading("Columns");
    ui.label("Name and Size are always shown.");
    ui.checkbox(&mut d.col_percent, "Percentage of parent (with bar)");
    ui.checkbox(&mut d.col_files, "Files");
    ui.checkbox(&mut d.col_modified, "Last change");
}

fn treemap(ui: &mut egui::Ui, d: &mut Settings) {
    ui.label("Changes preview live in the main window.");
    egui::Grid::new("tm").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        let pct = |ui: &mut egui::Ui, label: &str, v: &mut f32| {
            ui.label(label);
            ui.add(egui::Slider::new(v, 0.0..=1.0).custom_formatter(|x, _| format!("{:.0}", x * 100.0)));
            ui.end_row();
        };
        pct(ui, "Brightness", &mut d.brightness);
        pct(ui, "Cushion shading", &mut d.cushion);
        pct(ui, "Height", &mut d.height);
        pct(ui, "Scale factor", &mut d.scale);
        ui.label("Light from");
        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut d.light_x, -2.0..=2.0).text("x"));
            ui.add(egui::Slider::new(&mut d.light_y, -2.0..=2.0).text("y"));
        });
        ui.end_row();
        ui.label("Grid");
        ui.horizontal(|ui| {
            ui.checkbox(&mut d.show_grid, "Show");
            ui.color_edit_button_srgba(&mut d.grid_color);
        });
        ui.end_row();
        ui.label("Labels");
        ui.checkbox(&mut d.show_labels, "Show file names on tiles with room for them");
        ui.end_row();
        ui.label("Selection border");
        ui.color_edit_button_srgba(&mut d.selection_color);
        ui.end_row();
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Presets:");
        if ui.button("Classic").clicked() {
            let def = Settings::default();
            (d.brightness, d.cushion, d.height, d.scale, d.light_x, d.light_y) =
                (def.brightness, def.cushion, def.height, def.scale, def.light_x, def.light_y);
        }
        if ui.button("Soft").clicked() {
            (d.brightness, d.cushion, d.height, d.scale) = (0.9, 0.5, 0.25, 0.85);
        }
        if ui.button("Flat").clicked() {
            (d.brightness, d.cushion, d.height, d.scale, d.show_grid) = (0.75, 0.0, 0.0, 0.0, true);
        }
    });
}

fn advanced(ui: &mut egui::Ui, d: &mut Settings) {
    ui.heading("Scanning");
    ui.checkbox(&mut d.one_file_system, "Stay on the scanned drive (don't follow other mounted drives)");
    ui.checkbox(&mut d.skip_hidden, "Skip hidden files and folders");
    ui.label("Symbolic links and junctions are never followed, so nothing is counted twice.");
    ui.add_space(6.0);
    ui.heading("Performance");
    ui.horizontal(|ui| {
        ui.label("Scanner threads");
        ui.add(egui::DragValue::new(&mut d.threads).range(0..=256));
        ui.weak(format!("(0 = automatic, {} on this PC)", std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)));
    });
    ui.label("Network drives often scan faster with more threads, e.g. 32.");
}
