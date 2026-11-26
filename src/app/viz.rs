//! The controls around the Storage Analysis map: one toolbar row (view, size measure, colours,
//! filters, search, display), the Filters panel, and the active filters as removable chips.
//!
//! The map itself, the scan, the filter work and its results are the same ones the Files
//! workspace's dock uses; this file only decides how they're offered. Everything here reads state
//! that already exists (`App::filter`, `filter_stats`, the settings), so nothing is counted or
//! scanned per frame, and changing a filter re-uses the background filter work as before.

use super::*;
use crate::kit::{self, Icon};
use eframe::egui::RichText;
use crate::scan::Stored;
use custom::FilterMode;
use dock::unix_now;

/// Below these toolbar widths, controls move into the ⋯ menu instead of crowding the row.
const FOLD_COLOUR: f32 = 820.0;
const FOLD_MEASURE: f32 = 700.0;
const FOLD_DISPLAY: f32 = 560.0;

/// The Filters panel's width.
const PANEL_W: f32 = 392.0;

const DAY: i64 = 86_400;

/// Size presets offered in the Filters panel: (label, larger than, smaller than).
const SIZE_PRESETS: [(&str, u64, u64); 6] = [
    ("Any size", 0, 0),
    ("Under 1 MB", 0, 1 << 20),
    ("Over 10 MB", 10 << 20, 0),
    ("Over 100 MB", 100 << 20, 0),
    ("Over 1 GB", 1 << 30, 0),
    ("Over 10 GB", 10 << 30, 0),
];

/// Last-changed presets: (label, newer than N days, older than N days).
const AGE_PRESETS: [(&str, i64, i64); 6] = [
    ("Any time", 0, 0),
    ("Last 7 days", 7, 0),
    ("Last 30 days", 30, 0),
    ("Last year", 365, 0),
    ("Over a year ago", 0, 365),
    ("Over 5 years ago", 0, 5 * 365),
];

/// What the Storage Analysis toolbar remembers while the app runs.
#[derive(Default)]
pub(super) struct VizUi {
    /// The Filters panel is showing.
    pub filters_open: bool,
    /// Where the Filters button was last drawn, to hang the panel under it.
    filters_anchor: Option<Rect>,
}

/// One active filter, as a chip under the toolbar.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Chip {
    Kind(Category),
    Stored(Stored),
    Larger,
    Smaller,
    Newer,
    Older,
    Custom,
    Dupes,
}

/// A category's name without its emoji ("🖼 Images" → "Images").
pub(super) fn plain(label: &str) -> &str {
    match label.split_once(' ') {
        Some((head, rest)) if !head.chars().any(char::is_alphanumeric) => rest,
        _ => label,
    }
}

/// "7 days", "1 year", "5 years".
fn span_words(days: i64) -> String {
    match days {
        d if d >= 365 && d % 365 == 0 => {
            let y = d / 365;
            format!("{y} year{}", if y == 1 { "" } else { "s" })
        }
        d => format!("{d} day{}", if d == 1 { "" } else { "s" }),
    }
}

/// Whole days between a cutoff and now, for the age chips.
fn days_from(cutoff: i64, now: i64) -> i64 {
    ((now - cutoff) as f64 / DAY as f64).round().max(0.0) as i64
}

/// The active filters, in the order they read: kinds first, then where, size, age and name.
pub(super) fn chips_of(f: &ViewFilter, dupes: bool) -> Vec<(Chip, String)> {
    let now = unix_now();
    let mut v = Vec::new();
    for c in Category::ALL {
        if f.kinds & c.bit() != 0 {
            v.push((Chip::Kind(c), plain(c.label()).to_string()));
        }
    }
    for s in Stored::ALL {
        if f.stored & s.bit() != 0 {
            v.push((Chip::Stored(s), s.label().to_string()));
        }
    }
    if f.larger > 0 {
        v.push((Chip::Larger, format!("Larger than {}", scan::human(f.larger))));
    }
    if f.smaller > 0 {
        v.push((Chip::Smaller, format!("Smaller than {}", scan::human(f.smaller))));
    }
    if f.newer != 0 {
        v.push((Chip::Newer, format!("Changed in the last {}", span_words(days_from(f.newer, now)))));
    }
    if f.older != 0 {
        v.push((Chip::Older, format!("Unchanged for {}", span_words(days_from(f.older, now)))));
    }
    if let Some(c) = &f.custom {
        v.push((Chip::Custom, format!("Name: {}", c.source().trim())));
    }
    if dupes {
        v.push((Chip::Dupes, "Duplicates only".to_string()));
    }
    v
}

/// A compact toolbar control: an optional icon, a label and an optional ▾, in a soft pill.
/// `on` tints it (an open panel, a mode in use); `accent` fills it (filters in effect).
fn pill(ui: &mut egui::Ui, icon: Option<Icon>, text: &str, on: bool, accent: bool, caret: bool) -> egui::Response {
    let t = kit::tokens(ui);
    let font = FontId::proportional(13.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, t.text);
    let icon_w = if icon.is_some() { if text.is_empty() { 14.0 } else { 20.0 } } else { 0.0 };
    let caret_w = if caret { 14.0 } else { 0.0 };
    let pad = if text.is_empty() { 6.0 } else { 10.0 };
    let w = galley.size().x + icon_w + caret_w + pad * 2.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w.max(28.0), 28.0), Sense::click());
    let enabled = ui.is_enabled();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, text));
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered() && enabled;
        let base = style::blend(t.pal.chrome, t.pal.hover, 0.55);
        let (fill, fg) = if accent {
            (if hovered { style::blend(t.accent, Color32::WHITE, 0.10) } else { t.accent }, t.on_accent)
        } else if on {
            (t.accent_soft, t.strong)
        } else if hovered {
            (t.pal.hover, t.strong)
        } else {
            (base, if enabled { t.text } else { t.disabled })
        };
        ui.painter().rect_filled(rect, kit::CONTROL_RADIUS, fill);
        let mut x = rect.left() + pad;
        if let Some(i) = icon {
            kit::icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 7.0, rect.center().y), vec2(14.0, 14.0)), i, fg);
            x += icon_w;
        }
        ui.painter().galley_with_override_text_color(pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
        if caret {
            let c = Rect::from_center_size(pos2(rect.right() - pad - 4.0, rect.center().y + 0.5), vec2(10.0, 10.0));
            kit::icons::paint(ui.painter(), c, Icon::ChevronDown, fg);
        }
        kit::focus_ring(ui, rect, &resp, kit::CONTROL_RADIUS as f32);
    }
    resp
}

/// Two-way switch drawn as joined pills (Treemap | Sunburst, Highlight | Only).
fn switch<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, Option<Icon>, &str, &str)]) -> bool {
    let t = kit::tokens(ui);
    let mut changed = false;
    egui::Frame::new().fill(style::blend(t.pal.chrome, t.pal.hover, 0.55)).corner_radius(kit::CONTROL_RADIUS).inner_margin(2).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.horizontal(|ui| {
            for (v, icon, text, tip) in options {
                let on = *value == *v;
                let font = FontId::proportional(12.5);
                let galley = ui.painter().layout_no_wrap(text.to_string(), font, t.text);
                let icon_w = if icon.is_some() { 19.0 } else { 0.0 };
                let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + icon_w + 16.0, 24.0), Sense::click());
                let resp = resp.on_hover_text(*tip);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, on, *text));
                if on {
                    ui.painter().rect_filled(rect, kit::CONTROL_RADIUS - 1, t.raised);
                    if !t.dark {
                        ui.painter().rect_stroke(rect, kit::CONTROL_RADIUS - 1, egui::Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
                    }
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, kit::CONTROL_RADIUS - 1, t.pal.hover);
                }
                let fg = if on { t.strong } else { t.weak };
                let mut x = rect.left() + 8.0;
                if let Some(i) = icon {
                    kit::icons::paint(ui.painter(), Rect::from_center_size(pos2(x + 7.0, rect.center().y), vec2(14.0, 14.0)), *i, fg);
                    x += icon_w;
                }
                ui.painter().galley_with_override_text_color(pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
                kit::focus_ring(ui, rect, &resp, (kit::CONTROL_RADIUS - 1) as f32);
                if resp.clicked() && !on {
                    *value = *v;
                    changed = true;
                }
            }
        });
    });
    changed
}

/// A small on/off choice in the Filters panel.
fn choice(ui: &mut egui::Ui, on: bool, text: &str, tip: &str) -> egui::Response {
    let t = kit::tokens(ui);
    let font = FontId::proportional(12.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, t.text);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 18.0, 24.0), Sense::click());
    let resp = if tip.is_empty() { resp } else { resp.on_hover_text(tip) };
    let enabled = ui.is_enabled();
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, on, text));
    let (fill, stroke, fg) = if on {
        (t.accent_soft, egui::Stroke::new(1.0, style::blend(t.accent_soft, t.accent, 0.6)), t.strong)
    } else if resp.hovered() && enabled {
        (t.pal.hover, egui::Stroke::new(1.0, t.card_border), t.strong)
    } else {
        (Color32::TRANSPARENT, egui::Stroke::new(1.0, t.card_border), if enabled { t.text } else { t.disabled })
    };
    ui.painter().rect(rect, 12.0, fill, stroke, egui::StrokeKind::Inside);
    ui.painter().galley_with_override_text_color(pos2(rect.left() + 9.0, rect.center().y - galley.size().y / 2.0), galley, fg);
    kit::focus_ring(ui, rect, &resp, 12.0);
    resp
}

/// A removable chip: its words and a ×. Returns (body response, × clicked).
fn chip(ui: &mut egui::Ui, text: &str, tip: &str) -> bool {
    let t = kit::tokens(ui);
    let font = FontId::proportional(12.5);
    let shown: String = if text.chars().count() > 40 { format!("{}…", text.chars().take(39).collect::<String>()) } else { text.to_string() };
    let galley = ui.painter().layout_no_wrap(shown, font, t.text);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 36.0, 24.0), Sense::click());
    let x_rect = Rect::from_center_size(pos2(rect.right() - 13.0, rect.center().y), vec2(16.0, 16.0));
    let over_x = resp.hover_pos().is_some_and(|p| x_rect.expand(3.0).contains(p));
    let fill = if resp.hovered() { style::blend(t.accent_soft, t.accent, 0.18) } else { t.accent_soft };
    ui.painter().rect_filled(rect, 12.0, fill);
    ui.painter().galley_with_override_text_color(pos2(rect.left() + 11.0, rect.center().y - galley.size().y / 2.0), galley, t.strong);
    if over_x {
        ui.painter().circle_filled(x_rect.center(), 8.0, style::blend(t.accent_soft, t.accent, 0.4));
    }
    kit::icons::paint(ui.painter(), x_rect.shrink(4.0), Icon::Close, if over_x { t.strong } else { t.weak });
    kit::focus_ring(ui, rect, &resp, 12.0);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Remove filter: {text}")));
    let resp = resp.on_hover_text(format!("{tip}Click to remove this filter."));
    resp.clicked() || (resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)))
}

/// A section heading in the Filters panel.
fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(10.0);
    style::section_title(ui, text);
    ui.add_space(4.0);
}

impl App {
    // ---------- the app toolbar: where you are ----------

    /// The drive list, folder box, Browse and Scan: inline before the first scan, and in the
    /// location popover after it.
    pub(super) fn scan_controls(&mut self, ui: &mut egui::Ui, width: f32) {
        if ui.button("🖴 Drives…").clicked() {
            self.show_drives = !self.show_drives;
            if self.show_drives {
                self.refresh_drives();
            }
            ui.close();
        }
        let edit = ui.add(egui::TextEdit::singleline(&mut self.path_input).desired_width(width).hint_text("Folder to scan"));
        if ui.button("Browse…").clicked() {
            let d = dialogs::Dialog::new().set_directory(self.path_input.trim());
            self.ask_path(d, dialogs::Pick::Folder, |app, dir| {
                app.path_input = dir.shown();
                app.start_scan();
            });
            ui.close();
        }
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("⟳ Scan").clicked() || enter {
            self.start_scan();
            ui.close();
        }
    }

    /// Back, Forward, Up and Whole map, then the breadcrumb of the folder on the map, whose last
    /// part opens the scan controls.
    pub(super) fn analysis_nav(&mut self, ui: &mut egui::Ui) {
        let ready = self.tree.is_some() && self.job.is_none();
        let feel = self.feel();
        let treemap = self.map_kind == MapKind::Treemap;
        ui.add_enabled_ui(ready, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let back = ui.add_enabled_ui(treemap && self.map.can_go_back(), |ui| kit::icon_button(ui, Icon::ArrowLeft, "Back (Alt+Left, or the mouse's back button)")).inner;
            if back.clicked() {
                self.map.back(&feel);
            }
            let fwd = ui.add_enabled_ui(treemap && self.map.can_go_forward(), |ui| kit::icon_button(ui, Icon::ArrowRight, "Forward (Alt+Right)")).inner;
            if fwd.clicked() {
                self.map.forward(&feel);
            }
            if kit::icon_button(ui, Icon::ArrowUp, "Up one level (right-click on the map)").clicked() {
                if let (true, Some(tree)) = (treemap, self.shown_tree()) {
                    self.map.zoom_out(&tree, &feel);
                } else {
                    self.zoom.pop();
                }
            }
            if kit::icon_button(ui, Icon::Home, "Whole map: back to the whole scan (Home)").clicked() {
                if treemap {
                    self.map.reset(&feel);
                } else {
                    self.zoom.clear();
                }
            }
            let has_sel = self.selected.is_some();
            let zoom_sel = ui.add_enabled_ui(has_sel, |ui| kit::icon_button(ui, Icon::Target, "Zoom to the selection: bring the selected file or folder into view")).inner;
            if zoom_sel.clicked()
                && let (Some(sel), Some(tree)) = (self.selected.clone(), self.shown_tree())
            {
                if treemap {
                    self.map.frame_path(&tree, &sel, &feel);
                } else {
                    let mut z = sel;
                    if !tree.at(&z).is_some_and(|n| n.is_dir) {
                        z.pop();
                    }
                    self.zoom = z;
                }
            }
        });
        ui.add_space(8.0);
        self.breadcrumb(ui);
    }

    /// "Home / Users / rich ▾": each part goes back to that folder; the last one opens the
    /// scan controls (another folder, a drive, Browse).
    fn breadcrumb(&mut self, ui: &mut egui::Ui) {
        let t = kit::tokens(ui);
        let mut names: Vec<String> = Vec::new();
        match self.shown_tree() {
            Some(tree) => {
                names.push(tree.name.to_string());
                let mut n: &Node = &tree;
                for &i in &self.zoom {
                    let Some(c) = n.children.get(i as usize) else { break };
                    n = c;
                    names.push(n.name.to_string());
                }
            }
            None => names.push(self.job.as_ref().map(|j| j.root.shown()).unwrap_or_else(|| "Nothing scanned".into())),
        }
        let mut go = None;
        let last = names.len() - 1;
        // Long paths keep the first part and the last three, with the rest in a ⋯ menu.
        let hidden = if names.len() > 4 { 1..names.len() - 3 } else { 0..0 };
        ui.spacing_mut().item_spacing.x = 2.0;
        for (k, name) in names.iter().enumerate() {
            if hidden.contains(&k) {
                if k == hidden.start {
                    let r = ui.add(egui::Button::new(RichText::new("…").color(t.weak)).frame_when_inactive(false));
                    egui::Popup::menu(&r).show(|ui| {
                        for j in hidden.clone() {
                            if ui.button(&names[j]).clicked() {
                                go = Some(j);
                            }
                        }
                    });
                    ui.label(RichText::new("/").color(t.weak));
                }
                continue;
            }
            if k == last {
                let r = ui.add(egui::Button::new(RichText::new(format!("{name}  ⏷")).strong().color(t.strong)).frame_when_inactive(false));
                let r = r.on_hover_text(format!("{}\n\nClick to scan another folder or drive.", self.full_path(&self.zoom).shown()));
                egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                    ui.set_min_width(460.0);
                    ui.horizontal(|ui| self.scan_controls(ui, 260.0));
                });
            } else {
                let r = ui.add(egui::Button::new(RichText::new(name).color(t.text)).frame_when_inactive(false));
                if r.clicked() {
                    go = Some(k);
                }
                ui.label(RichText::new("/").color(t.weak));
            }
        }
        if let Some(k) = go {
            self.zoom.truncate(k);
        }
    }

    // ---------- the map's toolbar ----------

    /// One row over the map: view, size measure, colours, Filters, search and display options.
    /// Narrow windows fold the less used ones into a ⋯ menu instead of wrapping.
    pub(super) fn viz_toolbar(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        let fold_colour = width < FOLD_COLOUR;
        let fold_measure = width < FOLD_MEASURE;
        let fold_display = width < FOLD_DISPLAY;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let mut kind = self.map_kind;
            switch(
                ui,
                &mut kind,
                &[
                    (MapKind::Treemap, Some(Icon::Treemap), "Treemap", "Rectangles sized by space used; zoom and pan freely"),
                    (MapKind::Sunburst, Some(Icon::Sunburst), "Sunburst", "Rings: the centre is the folder, each ring one level deeper"),
                ],
            );
            self.map_kind = kind;
            if !fold_measure {
                self.measure_menu(ui);
            }
            if !fold_colour {
                self.colour_menu(ui);
            }
            self.filters_button(ui);
            // Leave room on the right for Display and the ⋯ menu.
            let reserve = if fold_display { 0.0 } else { 112.0 } + if fold_colour { 40.0 } else { 0.0 } + 8.0;
            let search_w = (ui.available_width() - reserve).clamp(120.0, 340.0);
            self.custom_search(ui, search_w);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if fold_colour {
                    let r = pill(ui, Some(Icon::More), "", false, false, false).on_hover_text("More: size measure, colours and display");
                    egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                        ui.set_min_width(220.0);
                        if fold_measure {
                            style::section_title(ui, "Size");
                            self.metric_switch(ui);
                            ui.separator();
                        }
                        style::section_title(ui, "Colour by");
                        self.colour_choices(ui);
                        if fold_display {
                            ui.separator();
                            style::section_title(ui, "Display");
                            self.display_choices(ui);
                        }
                    });
                }
                if !fold_display {
                    let r = pill(ui, Some(Icon::Eye), "Display", false, false, true).on_hover_text("Names on tiles, previews, the preview pane and other display options");
                    egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                        ui.set_min_width(240.0);
                        self.display_choices(ui);
                    });
                }
            });
        });
    }

    /// Size: On disk ▾ — which size the map, lists and totals use.
    fn measure_menu(&mut self, ui: &mut egui::Ui) {
        let label = match self.metric {
            scan::Metric::Disk => "On disk",
            scan::Metric::Logical => "Logical size",
            scan::Metric::CloudOnly => "Cloud only",
            scan::Metric::Reclaimable => "Reclaimable",
        };
        let r = pill(ui, Some(Icon::Layers), label, false, false, true)
            .on_hover_text("Which size the map, lists and totals use. Switching doesn't rescan.");
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(200.0);
            self.metric_switch(ui);
            ui.separator();
            if ui.button("What do these sizes mean?").clicked() {
                self.help.show_topic(topics::SIZE_METRIC);
                ui.close();
            }
        });
    }

    /// Colour: Type ▾
    fn colour_menu(&mut self, ui: &mut egui::Ui) {
        let label = match self.color_mode {
            ColorMode::Type => "Colour: Type",
            ColorMode::Age => "Colour: Age",
            ColorMode::Change => "Colour: Change",
        };
        let r = pill(ui, Some(Icon::Palette), label, false, false, true).on_hover_text("What the colours on the map mean");
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(200.0);
            self.colour_choices(ui);
        });
    }

    pub(super) fn colour_choices(&mut self, ui: &mut egui::Ui) {
        let before = self.color_mode;
        ui.selectable_value(&mut self.color_mode, ColorMode::Type, "File type").on_hover_text("A colour per file type, as in the File types list");
        ui.selectable_value(&mut self.color_mode, ColorMode::Age, "Age (last changed)").on_hover_text("How long since each file last changed");
        let has_diff = self.diff.is_some();
        ui.add_enabled_ui(has_diff, |ui| {
            ui.selectable_value(&mut self.color_mode, ColorMode::Change, "Change since snapshot")
                .on_disabled_hover_text("Compare with a saved snapshot first (Changes tab on the right)");
        });
        if self.color_mode != before {
            self.paint_gen += 1;
        }
    }

    /// The Display menu: what's drawn on and beside the map.
    pub(super) fn display_choices(&mut self, ui: &mut egui::Ui) {
        let mut labels = self.settings.show_labels;
        if ui.checkbox(&mut labels, "File names on tiles").changed() {
            self.settings.show_labels = labels;
            self.dialog.draft.show_labels = labels;
        }
        let mut previews = self.settings.tile_previews;
        if ui.checkbox(&mut previews, "Previews on tiles").on_hover_text("Zoomed-in tiles show what's in the file: pictures, PDF pages, text").changed() {
            self.settings.tile_previews = previews;
            self.dialog.draft.tile_previews = previews;
        }
        let has_cloud = self.tree.as_ref().is_some_and(|t| t.flags & (scan::store::CLOUD | scan::store::NOT_LOCAL) != 0);
        if has_cloud {
            let mut icons = self.settings.cloud_icons;
            if ui.checkbox(&mut icons, "Cloud marks on tiles").on_hover_text("A small cloud on tiles of files that aren't (fully) on this computer").changed() {
                self.settings.cloud_icons = icons;
                self.dialog.draft.cloud_icons = icons;
            }
            let mut fade = self.settings.cloud_fade;
            if ui.checkbox(&mut fade, "Fade online-only files").on_hover_text("At Logical size, files that are only in the cloud are drawn faintly").changed() {
                self.settings.cloud_fade = fade;
                self.dialog.draft.cloud_fade = fade;
                self.paint_gen += 1;
            }
            let mut bar = self.settings.cloud_breakdown;
            if ui.checkbox(&mut bar, "Cloud breakdown under the map").changed() {
                self.settings.cloud_breakdown = bar;
                self.dialog.draft.cloud_breakdown = bar;
            }
        }
        ui.separator();
        let shortcut = if cfg!(target_os = "macos") { "⌥P" } else { "Alt+P" };
        let mut pane = self.files.prefs.map_preview;
        if ui.checkbox(&mut pane, format!("Preview pane ({shortcut})")).on_hover_text("The clicked tile's preview and details, beside the map").changed() {
            self.files.prefs.map_preview = pane;
        }
        ui.separator();
        if ui.button("Moving around the map…").clicked() {
            self.help.show_topic(match self.map_kind {
                MapKind::Treemap => topics::TREEMAP_NAV,
                MapKind::Sunburst => topics::SUNBURST,
            });
            ui.close();
        }
        if ui.button("All settings…").clicked() {
            self.dialog.open(&self.settings);
            ui.close();
        }
    }

    /// How many filters are on, as the chips count them.
    pub(super) fn filter_count(&self) -> usize {
        chips_of(&self.filter, self.dupes_only).len()
    }

    /// Filters (2): opens the Filters panel; filled while filters are on.
    fn filters_button(&mut self, ui: &mut egui::Ui) {
        let n = self.filter_count();
        let text = if n > 0 { format!("Filters  {n}") } else { "Filters".to_string() };
        let r = pill(ui, Some(Icon::Filter), &text, self.viz.filters_open, n > 0, false)
            .on_hover_text("Show only some files: by type, where they're stored, size, age or name");
        if r.clicked() {
            self.viz.filters_open = !self.viz.filters_open;
        }
        self.viz.filters_anchor = Some(r.rect);
    }

    // ---------- the chips row ----------

    /// Whether the chips row has anything to show.
    pub(super) fn chips_showing(&self) -> bool {
        self.filter.active() || self.dupes_only || self.custom.error.is_some() || self.color_mode != ColorMode::Type
    }

    /// The active filters as chips, whether they highlight or hide the rest, the match count
    /// and Clear all. Also the key for the Age and Change colours.
    pub(super) fn chips_row(&mut self, ui: &mut egui::Ui) {
        let t = kit::tokens(ui);
        let total = self.tree.as_ref().map_or(0, |t| t.size);
        let chips = chips_of(&self.filter, self.dupes_only);
        if !chips.is_empty() || self.custom.error.is_some() {
            let mut remove = None;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                let mut mode = self.custom.prefs.mode;
                let changed = switch(
                    ui,
                    &mut mode,
                    &[
                        (FilterMode::Highlight, None, "Highlight", "Keep the layout; everything that doesn't match is dimmed"),
                        (FilterMode::Only, None, "Only matches", "Rebuild the maps and folder list from the matching files alone"),
                    ],
                );
                if changed {
                    self.custom.prefs.mode = mode;
                }
                for (c, text) in &chips {
                    let tip = match c {
                        Chip::Kind(k) => format!("{}\n\n", k.tooltip()),
                        Chip::Stored(s) => format!("{}\n\n", s.explain()),
                        Chip::Custom => "Typed in the search box.\n\n".to_string(),
                        Chip::Dupes => "Set in the Duplicates tab.\n\n".to_string(),
                        _ => String::new(),
                    };
                    if chip(ui, text, &tip) {
                        remove = Some(*c);
                    }
                }
                if let Some(e) = &self.custom.error {
                    ui.colored_label(t.warn, format!("⚠ {e}"));
                }
                if ui.add(egui::Button::new(RichText::new("Clear all").color(t.accent)).frame_when_inactive(false)).on_hover_text("Show every file again").clicked() {
                    self.clear_all_filters();
                }
                // The count, or what's still being worked out.
                let current = self.filter_stats.as_ref().filter(|(f, _, _)| *f == self.filter);
                match current {
                    Some((_, 0, _)) => {
                        ui.colored_label(t.warn, "No files match");
                    }
                    Some((_, files, bytes)) => {
                        let pct = *bytes as f64 * 100.0 / total.max(1) as f64;
                        ui.label(RichText::new(format!("{} files · {} ({pct:.1}%)", group(*files), scan::human(*bytes))).color(t.weak))
                            .on_hover_text("The files that pass every filter, and their share of the scan by the size measure in use.");
                        if *bytes == 0 {
                            let why = if self.metric == scan::Metric::Disk { "They take no space on disk, so the map can't show them: choose Logical size." } else { "They're all empty, so the map can't show them." };
                            ui.label(RichText::new(why).color(t.weak));
                        }
                    }
                    None => {}
                }
                if self.filtering_busy() || self.custom.edited.is_some() {
                    ui.spinner();
                    ui.label(RichText::new(if mode == FilterMode::Only { "Filtering…" } else { "Counting…" }).color(t.weak));
                }
            });
            if let Some(c) = remove {
                self.remove_chip(c);
            }
        }
        // What the Age and Change colours mean.
        let legend: Vec<(Color32, &str)> = match self.color_mode {
            ColorMode::Type => Vec::new(),
            ColorMode::Age => AGE_STOPS.iter().map(|(_, c, t)| (Color32::from_rgb(c[0], c[1], c[2]), *t)).collect(),
            ColorMode::Change => vec![
                (change_color(snapshot::NEW, 1), "new"),
                (change_color(1, 3), "grew"),
                (change_color(3, 1), "shrank"),
                (change_color(1, 1), "unchanged"),
            ],
        };
        if !legend.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(if self.color_mode == ColorMode::Age { "Last changed:" } else { "Since the snapshot:" }).color(t.weak));
                for (c, text) in legend {
                    let (r, _) = ui.allocate_exact_size(vec2(14.0, 10.0), Sense::hover());
                    ui.painter().rect_filled(r, 2.0, c);
                    ui.label(text);
                    ui.add_space(6.0);
                }
            });
        }
    }

    pub(super) fn clear_all_filters(&mut self) {
        self.filter = ViewFilter::default();
        self.clear_custom();
        self.dupes_only = false;
    }

    pub(super) fn remove_chip(&mut self, c: Chip) {
        match c {
            Chip::Kind(k) => self.filter.kinds &= !k.bit(),
            Chip::Stored(s) => self.filter.stored &= !s.bit(),
            Chip::Larger => self.filter.larger = 0,
            Chip::Smaller => self.filter.smaller = 0,
            Chip::Newer => self.filter.newer = 0,
            Chip::Older => self.filter.older = 0,
            Chip::Custom => self.clear_custom(),
            Chip::Dupes => self.dupes_only = false,
        }
    }

    // ---------- the Filters panel ----------

    /// The Filters panel, hung under the Filters button. It stays open while you use the map,
    /// so you can watch the filters take effect; Esc, × or the button close it.
    pub(super) fn filters_panel(&mut self, ctx: &egui::Context) {
        if !self.viz.filters_open {
            return;
        }
        let Some(anchor) = self.viz.filters_anchor else { return };
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.viz.filters_open = false;
            return;
        }
        let screen = ctx.content_rect();
        let x = anchor.left().min(screen.right() - PANEL_W - 8.0).max(screen.left() + 8.0);
        let pos = pos2(x, anchor.bottom() + 6.0);
        let max_h = (screen.bottom() - pos.y - 12.0).max(200.0);
        egui::Area::new(egui::Id::new("filters-panel"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                let t = kit::tokens(ui);
                egui::Frame::popup(ui.style()).fill(t.card).corner_radius(kit::RADIUS).inner_margin(egui::Margin { left: 14, right: 14, top: 10, bottom: 12 }).show(ui, |ui| {
                    ui.set_width(PANEL_W - 28.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Filters").size(kit::HEADING).strong().color(t.strong));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if kit::icon_button(ui, Icon::Close, "Close (Esc)").clicked() {
                                self.viz.filters_open = false;
                            }
                            if help::link_button(ui, "Help") {
                                self.help.show_topic(topics::FILTERING);
                            }
                        });
                    });
                    egui::ScrollArea::vertical().max_height(max_h - 90.0).auto_shrink([false, true]).show(ui, |ui| {
                        self.filters_body(ui);
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let any = self.filter.active() || self.dupes_only || self.custom.error.is_some();
                        if ui.add_enabled(any, egui::Button::new("Clear all")).clicked() {
                            self.clear_all_filters();
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let total = self.tree.as_ref().map_or(0, |t| t.size);
                            if let Some((_, files, bytes)) = self.filter_stats.as_ref().filter(|(f, _, _)| *f == self.filter && self.filter.active()) {
                                let pct = *bytes as f64 * 100.0 / total.max(1) as f64;
                                ui.label(RichText::new(format!("{} files · {} ({pct:.1}%)", group(*files), scan::human(*bytes))).strong().color(t.strong));
                            } else if self.filtering_busy() {
                                ui.spinner();
                            }
                        });
                    });
                });
            });
    }

    fn filters_body(&mut self, ui: &mut egui::Ui) {
        let t = kit::tokens(ui);
        let pro = self.licence.allows(crate::licence::Feature::Filters);
        let gate = |app: &mut App, on: bool| on || app.premium(crate::licence::Feature::Filters);
        if !pro {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("🔒 Filtering by type, place, size or age is part of Filescape Professional. Filters already on can always be cleared.").size(kit::DESC).color(t.weak));
            });
        }

        heading(ui, "Show");
        let mut mode = self.custom.prefs.mode;
        if switch(
            ui,
            &mut mode,
            &[
                (FilterMode::Highlight, None, "Highlight matches", "Keep the layout; everything that doesn't match is dimmed"),
                (FilterMode::Only, None, "Only matches", "Rebuild the maps and folder list from the matching files alone"),
            ],
        ) {
            self.custom.prefs.mode = mode;
        }

        heading(ui, "File types");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            if choice(ui, self.filter.kinds == 0, "All files", "").clicked() {
                self.filter.kinds = 0;
            }
            for c in Category::ALL {
                let on = self.filter.kinds & c.bit() != 0;
                if choice(ui, on, plain(c.label()), c.tooltip()).clicked() && gate(self, on) {
                    self.filter.kinds ^= c.bit();
                }
            }
        });

        let has_cloud = self.tree.as_ref().is_some_and(|t| t.flags & (scan::store::CLOUD | scan::store::NOT_LOCAL) != 0);
        if has_cloud || self.filter.stored != 0 {
            heading(ui, "Stored");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                if choice(ui, self.filter.stored == 0, "Anywhere", "").clicked() {
                    self.filter.stored = 0;
                }
                for s in Stored::ALL {
                    let on = self.filter.stored & s.bit() != 0;
                    if choice(ui, on, s.label(), s.explain()).clicked() && gate(self, on) {
                        self.filter.stored ^= s.bit();
                        let away = Stored::OnlineOnly.bit() | Stored::Partial.bit();
                        // Online-only files have no area at On disk; show them at Logical size.
                        if !on && self.filter.stored & away != 0 && self.metric == scan::Metric::Disk {
                            self.set_metric(scan::Metric::Logical);
                        }
                    }
                }
            });
            if self.filter.stored & (Stored::OnlineOnly.bit() | Stored::Partial.bit()) != 0 && self.metric == scan::Metric::Logical {
                ui.label(RichText::new("Sizes are logical, since online-only files take no space on disk.").size(kit::DESC).color(t.weak));
            }
        }

        heading(ui, "Size");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (label, larger, smaller) in SIZE_PRESETS {
                let on = self.filter.larger == larger && self.filter.smaller == smaller;
                let off = larger == 0 && smaller == 0;
                if choice(ui, on, label, "").clicked() && (off || gate(self, false)) {
                    self.filter.larger = larger;
                    self.filter.smaller = smaller;
                }
            }
        });
        ui.add_space(4.0);
        let sizes = pro || self.filter.larger > 0 || self.filter.smaller > 0;
        ui.add_enabled_ui(sizes, |ui| {
            ui.horizontal(|ui| {
                ui.label("At least");
                size_edit(ui, &mut self.filter.larger);
                ui.label("and under");
                size_edit(ui, &mut self.filter.smaller);
            });
        });

        heading(ui, "Last changed");
        let now = unix_now();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (label, newer, older) in AGE_PRESETS {
                let on_newer = if newer == 0 { self.filter.newer == 0 } else { self.filter.newer != 0 && days_from(self.filter.newer, now) == newer };
                let on_older = if older == 0 { self.filter.older == 0 } else { self.filter.older != 0 && days_from(self.filter.older, now) == older };
                let on = on_newer && on_older;
                let off = newer == 0 && older == 0;
                let tip = "By each file's last-modified time. Folders count by the files inside them.";
                if choice(ui, on, label, tip).clicked() && (off || gate(self, on)) {
                    self.filter.newer = if newer == 0 { 0 } else { now - newer * DAY };
                    self.filter.older = if older == 0 { 0 } else { now - older * DAY };
                }
            }
        });

        heading(ui, "Names and patterns");
        ui.label(RichText::new("Type in the search box on the toolbar: names, extensions such as *.psd, frame ranges, or !exclusions.").size(kit::DESC).color(t.weak));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let mut case = self.custom.prefs.case_sensitive;
            if ui.toggle_value(&mut case, "Aa").on_hover_text("Match upper and lower case exactly").changed() {
                self.custom.prefs.case_sensitive = case;
                self.apply_custom();
            }
            let mut all = self.custom.prefs.match_all;
            let any = ui.selectable_value(&mut all, false, "Any term").on_hover_text("A file matches if it matches one of the terms").changed();
            let every = ui.selectable_value(&mut all, true, "All terms").on_hover_text("A file must match every term; ! terms still exclude").changed();
            if any || every {
                self.custom.prefs.match_all = all;
                self.apply_custom();
            }
            ui.label(RichText::new("Presets").color(t.text));
            self.custom_menu(ui);
            if help::link_button(ui, "Syntax") {
                self.help.show_topic(topics::SYNTAX);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_name_every_filter_in_reading_order() {
        let now = unix_now();
        let f = ViewFilter {
            kinds: Category::Images.bit() | Category::Video.bit(),
            larger: 100 << 20,
            stored: Stored::Downloaded.bit(),
            newer: now - 30 * DAY,
            ..Default::default()
        };
        let words: Vec<String> = chips_of(&f, true).into_iter().map(|(_, s)| s).collect();
        assert_eq!(words, ["Video", "Images", "Downloaded", "Larger than 100.0 MB", "Changed in the last 30 days", "Duplicates only"]);
        assert!(chips_of(&ViewFilter::default(), false).is_empty());
    }

    #[test]
    fn plain_drops_only_a_leading_symbol() {
        assert_eq!(plain("🖼 Images"), "Images");
        assert_eq!(plain("Other"), "Other");
        assert_eq!(plain("💿 Disk images"), "Disk images");
    }

    #[test]
    fn spans_read_naturally() {
        assert_eq!(span_words(1), "1 day");
        assert_eq!(span_words(30), "30 days");
        assert_eq!(span_words(365), "1 year");
        assert_eq!(span_words(5 * 365), "5 years");
    }
}
