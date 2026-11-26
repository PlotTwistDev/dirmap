//! User preferences, saved between runs, and the Settings window: a sidebar of pages with a
//! search box, each page built from the design kit in [`crate::kit`].

mod pages;
mod preview;
mod search;

use crate::elevate::AdminMode;
use crate::kit::{self, Icon, Kind, Tone};
use eframe::egui::{self, Color32, Id, Margin, RichText};
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

/// How treemap tiles are drawn.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum TileStyle {
    /// Soft pastel tiles with a gentle glow, dark folder name tags and plain labels: the same
    /// look as the map on filescape.app.
    #[default]
    Modern,
    /// The cushion treemap: shaded, bevelled tiles in strong colours, tuned with the sliders.
    Classic,
}

/// How strongly the treemap lights up under the pointer.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LightLevel {
    Subtle,
    Normal,
    Strong,
}

impl LightLevel {
    pub fn factor(self) -> f32 {
        match self {
            LightLevel::Subtle => 0.55,
            LightLevel::Normal => 1.0,
            LightLevel::Strong => 1.6,
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // General
    pub theme: Theme,
    pub units: SizeUnits,
    pub stripes: bool,
    pub live_tree: bool,
    /// The size Storage Analysis opens with: on disk (default) or logical. The switch above the
    /// map changes it.
    pub size_metric: crate::scan::Metric,
    /// Status icons for cloud files (online only, downloaded, pinned…) in the lists and tree.
    pub cloud_icons: bool,
    /// The info badge after the names of system files and folders Filescape can explain
    /// (pagefile.sys, WinSxS, .Spotlight-V100, /proc…), with what they are and whether to delete them.
    pub system_notes: bool,
    /// The bar under the map that splits the scan into local files, downloaded cloud files and
    /// data only in the cloud.
    pub cloud_breakdown: bool,
    /// The Stored column in the Storage Analysis list, when the scan has cloud files.
    pub cloud_column: bool,
    /// At Size: Logical, fade the map tiles of online-only files (partly downloaded ones half
    /// as much), so what's on this computer stands out.
    pub cloud_fade: bool,
    /// Ask before anything Filescape does would download cloud files (previews, players,
    /// comparisons). Off downloads without asking.
    pub cloud_confirm: bool,
    /// Walk into mounted cloud drives (rclone, FUSE, Google Drive's G: drive) found inside a
    /// scanned folder. A mounted drive that is itself the scanned folder is always scanned.
    pub cloud_mounts: bool,
    /// Ask filescape.app for the newest version at launch and show a notice when there's a newer
    /// one. Nothing about the user or the computer is sent.
    pub check_updates: bool,
    /// Size of everything on screen (1 = 100%). Ctrl + and Ctrl − change it too.
    pub ui_scale: f32,

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
    /// Modern (soft, as on the website) or Classic (cushion shading).
    pub tile_style: TileStyle,
    pub brightness: f32,
    pub cushion: f32,
    pub height: f32,
    pub scale: f32,
    pub light_x: f32,
    pub light_y: f32,
    pub show_grid: bool,
    /// Write file names on treemap tiles big enough to hold them.
    pub show_labels: bool,
    /// Tiles big enough on screen show what's in the file (pictures, PDF pages, text).
    pub tile_previews: bool,
    pub grid_color: Color32,
    pub selection_color: Color32,

    // Treemap navigation
    /// Animate zooming and panning (off: changes apply at once).
    pub smooth_zoom: bool,
    /// Zoom per wheel notch or trackpad distance, relative to the default.
    pub zoom_sensitivity: f32,
    /// Animation speed, relative to the default (2 = twice as fast).
    pub anim_speed: f32,

    // Treemap effects
    /// A soft light follows the pointer across the tiles.
    pub lighting: bool,
    pub light_level: LightLevel,
    /// Very slight bevel on tile edges.
    pub depth_effect: bool,

    // Previews & thumbnails
    /// Keep thumbnails on disk between runs (the system's cache folder).
    pub thumb_cache: bool,
    /// The most the thumbnail folder may hold, in MB.
    pub thumb_cache_mb: f32,
    /// A filmstrip of frames from across a video under its preview.
    pub video_strip: bool,
    /// When a video's first frame is dull (black, a fade), look for a better one.
    pub video_better_poster: bool,
    /// Draw 3D models and MIDI files for Icons view and map tiles.
    pub rich_thumbs: bool,

    // Advanced
    pub one_file_system: bool,
    pub skip_hidden: bool,
    /// Scanner threads (0 = one per CPU core).
    pub threads: usize,
    /// Whether to restart as administrator at start. Kept in its own file (see `elevate`), not
    /// with the rest, so it can be read before the window opens.
    #[serde(skip)]
    pub admin: AdminMode,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            units: SizeUnits::Binary,
            stripes: true,
            live_tree: true,
            size_metric: crate::scan::Metric::Disk,
            cloud_icons: true,
            system_notes: true,
            cloud_breakdown: true,
            cloud_column: true,
            cloud_fade: true,
            cloud_confirm: true,
            cloud_mounts: false,
            check_updates: true,
            ui_scale: 1.0,
            exclude_folders: String::new(),
            exclude_files: String::new(),
            include_files: String::new(),
            min_file_size: 0,
            max_age_days: 0,
            col_percent: true,
            col_files: true,
            col_modified: true,
            tile_style: TileStyle::Modern,
            brightness: 0.88,
            cushion: 0.87,
            height: 0.38,
            scale: 0.91,
            light_x: -1.0,
            light_y: -1.0,
            show_grid: false,
            show_labels: true,
            tile_previews: true,
            grid_color: Color32::BLACK,
            selection_color: Color32::WHITE,
            smooth_zoom: true,
            zoom_sensitivity: 1.0,
            anim_speed: 1.0,
            lighting: true,
            light_level: LightLevel::Normal,
            depth_effect: true,
            thumb_cache: true,
            thumb_cache_mb: 512.0,
            video_strip: true,
            video_better_poster: true,
            rich_thumbs: true,
            one_file_system: true,
            skip_hidden: false,
            threads: 0,
            admin: AdminMode::Ask,
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
            self.tile_style as u32,
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


impl Settings {
    /// Put the settings on one page back to their defaults, leaving the other pages alone.
    pub fn reset_page(&mut self, page: Tab) {
        let def = Settings::default();
        match page {
            Tab::General => {
                (self.theme, self.units, self.stripes, self.live_tree, self.check_updates, self.ui_scale, self.system_notes) =
                    (def.theme, def.units, def.stripes, def.live_tree, def.check_updates, def.ui_scale, def.system_notes);
            }
            Tab::Filtering => {
                self.exclude_folders = def.exclude_folders;
                self.exclude_files = def.exclude_files;
                self.include_files = def.include_files;
                self.min_file_size = def.min_file_size;
                self.max_age_days = def.max_age_days;
            }
            Tab::FolderList => {
                (self.col_percent, self.col_files, self.col_modified) = (def.col_percent, def.col_files, def.col_modified);
            }
            Tab::Treemap => {
                let admin = self.admin;
                let keep = std::mem::take(self);
                *self = Settings {
                    tile_style: def.tile_style,
                    brightness: def.brightness,
                    cushion: def.cushion,
                    height: def.height,
                    scale: def.scale,
                    light_x: def.light_x,
                    light_y: def.light_y,
                    show_grid: def.show_grid,
                    show_labels: def.show_labels,
                    tile_previews: def.tile_previews,
                    grid_color: def.grid_color,
                    selection_color: def.selection_color,
                    smooth_zoom: def.smooth_zoom,
                    zoom_sensitivity: def.zoom_sensitivity,
                    anim_speed: def.anim_speed,
                    lighting: def.lighting,
                    light_level: def.light_level,
                    depth_effect: def.depth_effect,
                    ..keep
                };
                self.admin = admin;
            }
            Tab::Advanced => {
                (self.one_file_system, self.skip_hidden, self.threads, self.admin) = (def.one_file_system, def.skip_hidden, def.threads, def.admin);
            }
            Tab::Cloud => {
                (self.size_metric, self.cloud_icons, self.cloud_breakdown, self.cloud_column, self.cloud_fade, self.cloud_confirm, self.cloud_mounts) =
                    (def.size_metric, def.cloud_icons, def.cloud_breakdown, def.cloud_column, def.cloud_fade, def.cloud_confirm, def.cloud_mounts);
            }
            Tab::Previews => {
                (self.thumb_cache, self.thumb_cache_mb, self.video_strip, self.video_better_poster, self.rich_thumbs) =
                    (def.thumb_cache, def.thumb_cache_mb, def.video_strip, def.video_better_poster, def.rich_thumbs);
            }
            Tab::Licensing => {}
        }
    }

    /// Send the preview settings to the parts that make thumbnails and pictures of videos (they
    /// run on worker threads, away from the settings).
    pub fn apply_previews(&self) {
        crate::thumbcache::set_enabled(self.thumb_cache);
        crate::thumbcache::set_limit((self.thumb_cache_mb.max(16.0) as u64) << 20);
        crate::video::set_better_posters(self.video_better_poster);
        crate::thumbcache::set_rich(self.rich_thumbs);
    }
}

/// Ready-made choices for the Previews & thumbnails page.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PreviewPreset {
    /// Nothing kept on disk, and only the pictures that are quick to make.
    Light,
    Balanced,
    /// Everything, with room for many more thumbnails on disk.
    Everything,
    /// The settings match none of the above.
    Custom,
}

impl PreviewPreset {
    /// (disk cache, its size in MB, filmstrip, better video pictures, 3D and MIDI pictures)
    fn values(self) -> Option<(bool, f32, bool, bool, bool)> {
        match self {
            PreviewPreset::Light => Some((false, 512.0, false, false, false)),
            PreviewPreset::Balanced => Some((true, 512.0, true, true, true)),
            PreviewPreset::Everything => Some((true, 2048.0, true, true, true)),
            PreviewPreset::Custom => None,
        }
    }

    pub fn of(s: &Settings) -> PreviewPreset {
        let now = (s.thumb_cache, s.thumb_cache_mb, s.video_strip, s.video_better_poster, s.rich_thumbs);
        [PreviewPreset::Light, PreviewPreset::Balanced, PreviewPreset::Everything]
            .into_iter()
            .find(|p| p.values().is_some_and(|v| v.0 == now.0 && (!v.0 || v.1 == now.1) && (v.2, v.3, v.4) == (now.2, now.3, now.4)))
            .unwrap_or(PreviewPreset::Custom)
    }

    pub fn apply(self, s: &mut Settings) {
        if let Some(v) = self.values() {
            (s.thumb_cache, s.thumb_cache_mb, s.video_strip, s.video_better_poster, s.rich_thumbs) = v;
        }
    }
}

/// The pages of the Settings window, in sidebar order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum Tab {
    #[default]
    General,
    Filtering,
    FolderList,
    Treemap,
    Cloud,
    Previews,
    Advanced,
    Licensing,
}

impl Tab {
    pub const ALL: [Tab; 8] = [Tab::General, Tab::Filtering, Tab::FolderList, Tab::Treemap, Tab::Cloud, Tab::Previews, Tab::Advanced, Tab::Licensing];

    pub fn name(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Filtering => "Filtering",
            Tab::FolderList => "Folder list",
            Tab::Treemap => "Treemap",
            Tab::Cloud => "Cloud storage",
            Tab::Previews => "Previews & thumbnails",
            Tab::Advanced => "Advanced",
            Tab::Licensing => "Licensing",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Tab::General => Icon::Gear,
            Tab::Filtering => Icon::Filter,
            Tab::FolderList => Icon::List,
            Tab::Treemap => Icon::Grid,
            Tab::Cloud => Icon::Cloud,
            Tab::Previews => Icon::File,
            Tab::Advanced => Icon::Sliders,
            Tab::Licensing => Icon::Key,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Tab::General => "Appearance, how sizes are written, scanning and updates.",
            Tab::Filtering => "Choose what scans skip and what they count. These rules apply from the next scan.",
            Tab::FolderList => "Choose the columns of the folder list in Storage Analysis.",
            Tab::Treemap => "How the treemap looks and moves. Every change previews live in the main window.",
            Tab::Cloud => "How sizes are counted for cloud files, how their state is shown, and keeping them in the cloud.",
            Tab::Previews => "How much work goes into previews and thumbnails, and what's kept on disk.",
            Tab::Advanced => "Scanner behaviour, performance, permissions and diagnostics.",
            Tab::Licensing => "Your Filescape licence, its activation on this computer, and buying one.",
        }
    }

    /// Whether the page has settings that "Restore page defaults" would change.
    fn has_settings(self) -> bool {
        self != Tab::Licensing
    }
}

/// What the window needs from the app besides the draft.
pub struct Env<'a> {
    /// The settings in force (Apply is offered when the draft differs).
    pub current: &'a Settings,
    /// The settings the scan on screen was made with, to say when filters need a rescan.
    pub scanned: Option<&'a Settings>,
    /// How the app draws, for diagnostics.
    pub renderer: &'static str,
}

/// A search result being shown: its section is outlined for a moment, and scrolled to once.
#[derive(Clone, Copy)]
struct Target {
    section: &'static str,
    until: f64,
    scroll: bool,
}

/// The Settings window. Edits a draft; returns Apply when OK or Apply was pressed.
pub struct Dialog {
    pub open: bool,
    pub draft: Settings,
    /// The page on show; remembered between runs.
    pub tab: Tab,
    /// Set when "Restart as administrator now" was pressed; the app acts on it and clears it.
    pub restart_as_admin: bool,
    /// Set when the help button (or F1 over the window) asks for a help page.
    pub help_request: Option<&'static str>,
    /// Where the window was last drawn, for F1.
    pub rect: Option<egui::Rect>,
    query: String,
    target: Option<Target>,
    confirm_reset_all: bool,
    /// Whether macOS Full Disk Access is missing, checked when the window opens (it probes
    /// folders, so not every frame).
    fda_missing: bool,
    preview: preview::Cache,
    /// Focus the search box on the next frame (Ctrl+F).
    focus_search: bool,
}

impl Default for Dialog {
    fn default() -> Self {
        Self {
            open: false,
            draft: Settings::default(),
            tab: Tab::General,
            restart_as_admin: false,
            help_request: None,
            rect: None,
            query: String::new(),
            target: None,
            confirm_reset_all: false,
            fda_missing: false,
            preview: preview::Cache::default(),
            focus_search: false,
        }
    }
}

pub enum Outcome {
    None,
    /// Draft changed; preview it (treemap look) without committing.
    Preview,
    Apply,
}

/// What a page is given to draw with.
pub(crate) struct PageCx<'a> {
    pub d: &'a mut Settings,
    pub env: &'a Env<'a>,
    pub restart: &'a mut bool,
    pub help: &'a mut Option<&'static str>,
    pub preview: &'a mut preview::Cache,
    pub fda_missing: bool,
    target: Option<Target>,
}

impl PageCx<'_> {
    /// How the section `id` should be drawn: outlined (and scrolled to) when a search led here.
    pub fn focus(&self, id: &str) -> kit::Focus {
        self.focuser().of(id)
    }

    /// [`Self::focus`] as a copy that doesn't borrow the page.
    pub fn focuser(&self) -> Focuser {
        Focuser(self.target)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Focuser(Option<Target>);

impl Focuser {
    pub fn of(self, id: &str) -> kit::Focus {
        match self.0 {
            Some(t) if t.section == id => kit::Focus { highlight: true, scroll: t.scroll },
            _ => kit::Focus::default(),
        }
    }
}

const SIDEBAR_W: f32 = 228.0;

impl Dialog {
    pub fn open(&mut self, current: &Settings) {
        if !self.open {
            self.fda_missing = crate::elevate::missing_full_disk_access();
            self.query.clear();
            self.target = None;
            self.confirm_reset_all = false;
        }
        self.draft = current.clone();
        self.open = true;
    }

    /// Settings > Licensing.
    pub fn open_licensing(&mut self, current: &Settings) {
        self.open(current);
        self.tab = Tab::Licensing;
    }

    /// The help page for the page on show.
    pub fn topic(&self) -> &'static str {
        use crate::help::topics;
        match self.tab {
            Tab::General => topics::SETTINGS_GENERAL,
            Tab::Filtering => topics::SETTINGS_FILTERING,
            Tab::FolderList => topics::SETTINGS_FOLDER_LIST,
            Tab::Treemap => topics::SETTINGS_TREEMAP,
            Tab::Cloud => topics::SETTINGS_CLOUD,
            Tab::Previews => topics::SETTINGS_PREVIEWS,
            Tab::Advanced => topics::SETTINGS_ADVANCED,
            Tab::Licensing => topics::LICENSING,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, licence: &mut crate::licence::Licensing, env: &Env) -> Outcome {
        if !self.open {
            self.rect = None;
            return Outcome::None;
        }
        let before = self.draft.clone();
        let mut outcome = Outcome::None;
        let mut close = false;
        let topic = self.topic();
        let mut help = false;
        let now = ctx.input(|i| i.time);
        if self.target.is_some_and(|t| now > t.until) {
            self.target = None;
        }
        // Esc cancels, unless it's leaving a text field or closing the confirmation.
        if !self.confirm_reset_all && !ctx.egui_wants_keyboard_input() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::F))) {
            self.focus_search = true;
        }

        let t = kit::tokens_for(ctx.global_style().visuals.dark_mode);
        let screen = ctx.content_rect();
        let size = egui::vec2(880.0, 660.0).min(screen.size() - egui::vec2(32.0, 32.0)).max(egui::vec2(560.0, 400.0));
        let changed = self.draft != *env.current;
        let frame = egui::Frame::window(&ctx.global_style()).inner_margin(Margin::ZERO).fill(t.content).corner_radius(10);
        let shown = egui::Window::new("Settings")
            .id(Id::new("settings-window"))
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .frame(frame)
            .default_size(size)
            .min_size([600.0, 420.0])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .constrain(true)
            .show(ctx, |ui| {
                let t = kit::tokens(ui);
                // Header: the window's name and a close button.
                egui::Panel::top("settings-head")
                    .resizable(false)
                    .show_separator_line(false)
                    .frame(egui::Frame::new().fill(t.sidebar).inner_margin(Margin::symmetric(18, 12)).corner_radius(egui::CornerRadius { nw: 10, ne: 10, sw: 0, se: 0 }))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 10.0;
                            kit::icons::show(ui, Icon::Gear, 18.0, t.strong);
                            ui.label(RichText::new("Filescape Settings").size(16.0).strong().color(t.strong));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if kit::icon_button(ui, Icon::Close, "Close without keeping changes (Esc)").clicked() {
                                    close = true;
                                }
                            });
                        });
                    });
                // Footer: the action area.
                egui::Panel::bottom("settings-foot")
                    .resizable(false)
                    .show_separator_line(false)
                    .frame(egui::Frame::new().fill(t.sidebar).inner_margin(Margin::symmetric(16, 12)).corner_radius(egui::CornerRadius { nw: 0, ne: 0, sw: 10, se: 10 }))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            help = kit::button(ui, Kind::Ghost, Some(Icon::Help), "Help").on_hover_text("What each setting on this page does (F1)").clicked();
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if kit::button(ui, Kind::Primary, None, "OK").on_hover_text("Keep the changes and close").clicked() {
                                    outcome = Outcome::Apply;
                                    close = true;
                                }
                                let apply = ui
                                    .add_enabled_ui(changed, |ui| kit::button(ui, Kind::Secondary, None, "Apply"))
                                    .inner
                                    .on_hover_text("Keep the changes and leave the window open")
                                    .on_disabled_hover_text("Nothing has changed since the window opened or was last applied");
                                if apply.clicked() {
                                    outcome = Outcome::Apply;
                                }
                                if kit::button(ui, Kind::Secondary, None, "Cancel").on_hover_text("Close and undo every change not applied, including the live preview (Esc)").clicked() {
                                    close = true;
                                }
                                ui.add_space(6.0);
                                if kit::button(ui, Kind::Ghost, None, "Restore all…").on_hover_text("Put every setting on every page back to its default").clicked() {
                                    self.confirm_reset_all = true;
                                }
                                if self.tab.has_settings() {
                                    let page = self.tab;
                                    let at_default = {
                                        let mut d = self.draft.clone();
                                        d.reset_page(page);
                                        d == self.draft
                                    };
                                    let r = ui
                                        .add_enabled_ui(!at_default, |ui| kit::button(ui, Kind::Ghost, Some(Icon::Reset), "Restore page defaults"))
                                        .inner
                                        .on_hover_text(format!("Put the settings on the {} page back to their defaults", page.name()))
                                        .on_disabled_hover_text("Everything on this page is already at its default");
                                    if r.clicked() {
                                        self.draft.reset_page(page);
                                    }
                                }
                            });
                        });
                    });
                // Sidebar: search and the pages.
                egui::Panel::left("settings-nav")
                    .resizable(false)
                    .exact_size(SIDEBAR_W)
                    .show_separator_line(false)
                    .frame(egui::Frame::new().fill(t.sidebar).inner_margin(Margin { left: 14, right: 14, top: 6, bottom: 12 }))
                    .show(ui, |ui| self.sidebar(ui, now));
                egui::CentralPanel::default().frame(egui::Frame::new().fill(t.content)).show(ui, |ui| {
                    let line = ui.max_rect();
                    ui.painter().vline(line.left(), line.y_range(), egui::Stroke::new(1.0, t.divider));
                    ui.painter().hline(line.x_range(), line.top(), egui::Stroke::new(1.0, t.divider));
                    egui::ScrollArea::vertical().id_salt(("settings-page", self.tab)).auto_shrink([false, false]).show(ui, |ui| {
                        egui::Frame::new().inner_margin(Margin { left: kit::PAGE_PAD, right: kit::PAGE_PAD, top: 20, bottom: kit::PAGE_PAD }).show(ui, |ui| {
                            ui.set_max_width(760.0);
                            kit::page_header(ui, self.tab.name(), self.tab.description());
                            let mut cx = PageCx {
                                d: &mut self.draft,
                                env,
                                restart: &mut self.restart_as_admin,
                                help: &mut self.help_request,
                                preview: &mut self.preview,
                                fda_missing: self.fda_missing,
                                target: self.target,
                            };
                            match self.tab {
                                Tab::General => pages::general(ui, &mut cx, licence),
                                Tab::Filtering => pages::filtering(ui, &mut cx),
                                Tab::FolderList => pages::folder_list(ui, &mut cx),
                                Tab::Treemap => pages::treemap(ui, &mut cx),
                                Tab::Cloud => pages::cloud(ui, &mut cx),
                                Tab::Previews => pages::previews(ui, &mut cx),
                                Tab::Advanced => pages::advanced(ui, &mut cx),
                                Tab::Licensing => licence.panel(ui, &|id| cx.focus(id)),
                            }
                        });
                    });
                });
            });
        if let Some(t) = &mut self.target {
            t.scroll = false;
        }
        self.rect = shown.as_ref().map(|r| r.response.rect);
        if self.confirm_reset_all {
            self.reset_all_modal(ctx);
        }
        if help {
            self.help_request = Some(topic);
        }
        if close {
            self.open = false;
        }
        if matches!(outcome, Outcome::None) && self.draft != before {
            outcome = Outcome::Preview;
        }
        outcome
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, now: f64) {
        let t = kit::tokens(ui);
        ui.add_space(8.0);
        let r = kit::search_field(ui, &mut self.query, "Find a setting…");
        if std::mem::take(&mut self.focus_search) {
            r.request_focus();
        }
        ui.add_space(14.0);
        let q = self.query.trim().to_string();
        if q.is_empty() {
            ui.label(RichText::new("PREFERENCES").size(kit::SMALL).color(t.weak));
            ui.add_space(6.0);
            for tab in Tab::ALL {
                if tab == Tab::Licensing {
                    ui.add_space(8.0);
                    ui.label(RichText::new("ACCOUNT").size(kit::SMALL).color(t.weak));
                    ui.add_space(6.0);
                }
                if kit::nav_item(ui, tab.icon(), tab.name(), self.tab == tab).clicked() {
                    self.tab = tab;
                }
                ui.add_space(2.0);
            }
        } else {
            let hits = search::find(&q);
            ui.label(RichText::new(if hits.is_empty() { "NO MATCHES".to_string() } else { format!("{} MATCH{}", hits.len(), if hits.len() == 1 { "" } else { "ES" }) }).size(kit::SMALL).color(t.weak));
            ui.add_space(6.0);
            if hits.is_empty() {
                kit::note(ui, "Try a different word, such as zoom, threads, hidden or licence.");
            }
            egui::ScrollArea::vertical().id_salt("settings-hits").auto_shrink([false, true]).show(ui, |ui| {
                for h in hits {
                    if search::result(ui, h, self.tab == h.page && self.target.is_some_and(|t| t.section == h.section)).clicked() {
                        self.tab = h.page;
                        self.target = Some(Target { section: h.section, until: now + 1.6, scroll: true });
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(1700));
                    }
                }
            });
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.label(RichText::new(format!("Filescape {}", env!("CARGO_PKG_VERSION"))).size(kit::SMALL).color(t.weak));
        });
    }

    fn reset_all_modal(&mut self, ctx: &egui::Context) {
        let r = egui::Modal::new(Id::new("settings-reset-all")).show(ctx, |ui| {
            ui.set_max_width(380.0);
            let t = kit::tokens(ui);
            ui.horizontal(|ui| {
                kit::icons::show(ui, Icon::Warning, 18.0, t.warn);
                ui.label(RichText::new("Restore every setting?").size(kit::HEADING + 1.0).strong().color(t.strong));
            });
            ui.add_space(6.0);
            kit::note(ui, "Every setting on every page goes back to its default, including the scan filters and the administrator choice. Your licence isn't affected.");
            ui.add_space(4.0);
            kit::note(ui, "Nothing is kept until you press OK or Apply, so Cancel still undoes it.");
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if kit::button(ui, Kind::Danger, Some(Icon::Reset), "Restore all").clicked() {
                    self.draft = Settings::default();
                    self.confirm_reset_all = false;
                }
                if kit::button(ui, Kind::Secondary, None, "Keep my settings").clicked() {
                    self.confirm_reset_all = false;
                }
            });
        });
        if r.should_close() {
            self.confirm_reset_all = false;
        }
    }
}

/// The colour for a status message in a page.
pub(crate) fn tone_of(error: bool) -> Tone {
    if error { Tone::Danger } else { Tone::Success }
}
