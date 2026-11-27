//! The panes at the right of the Storage Analysis map, and the rail they fold into.
//!
//! The Preview pane (the clicked tile's preview and details) and the Insights panel (File types,
//! Sizes, Changes, Duplicates, Sequences) can each be folded away to give the map room. Folded,
//! they leave a narrow rail of buttons at the window's right edge that bring them back. The
//! Preview pane can be pinned open; unpinned, it opens by itself when a tile is picked and folds
//! away when nothing is. Both remember whether they're open and how wide they are.

use super::*;
use crate::kit::{self, Icon};

/// Width of the rail.
const RAIL_W: f32 = 36.0;

/// The Preview pane's tabs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) enum PreviewTab {
    /// A large preview, the name and a few facts.
    #[default]
    Preview,
    /// Everything known: size, dates, storage, metadata, what's inside a folder.
    Details,
}

/// The Insights tabs, with the icon each has on the rail.
pub(super) const INSIGHTS: [(SideTab, Icon, &str); 5] = [
    (SideTab::Types, Icon::Palette, "File types"),
    (SideTab::Sizes, Icon::Ruler, "Sizes"),
    (SideTab::Changes, Icon::Clock, "Changes"),
    (SideTab::Duplicates, Icon::Copy, "Duplicates"),
    (SideTab::Sequences, Icon::List, "Sequences"),
];

/// A square icon button for the rail and pane headers; `on` draws it selected.
pub(super) fn rail_button(ui: &mut egui::Ui, icon: Icon, tip: &str, on: bool) -> egui::Response {
    let t = kit::tokens(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if on { t.accent_soft } else if resp.hovered() { t.pal.hover } else { Color32::TRANSPARENT };
        ui.painter().rect_filled(rect, kit::CONTROL_RADIUS, fill);
        let fg = if on { t.accent } else if resp.hovered() { t.strong } else { t.weak };
        kit::icons::paint(ui.painter(), rect.shrink(7.0), icon, fg);
        kit::focus_ring(ui, rect, &resp, kit::CONTROL_RADIUS as f32);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, tip));
    resp.on_hover_text(tip)
}

impl App {
    /// The shortcut that shows or folds the Preview pane, as this platform writes it.
    pub(super) fn preview_shortcut() -> &'static str {
        if cfg!(target_os = "macos") { "⌥P" } else { "Alt+P" }
    }

    /// The shortcut that shows or folds the Insights panel.
    pub(super) fn insights_shortcut() -> &'static str {
        if cfg!(target_os = "macos") { "⌥I" } else { "Alt+I" }
    }

    /// Keys and automatic opening, once a frame before the panes are laid out.
    pub(super) fn rails_update(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::P)) {
            self.files.prefs.map_preview = !self.files.prefs.map_preview;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, egui::Key::I)) {
            self.files.prefs.insights_open = !self.files.prefs.insights_open;
        }
        // Unpinned, the Preview pane follows the selection: a new pick opens it, and clearing
        // the selection folds it away (and it starts folded with nothing picked).
        if !self.viz.rails_started {
            self.viz.rails_started = true;
            self.viz.seen_selection = self.selected.clone();
            if !self.files.prefs.map_preview_pin {
                self.files.prefs.map_preview = self.selected.is_some();
            }
        }
        if self.viz.seen_selection != self.selected {
            self.viz.seen_selection = self.selected.clone();
            if !self.files.prefs.map_preview_pin {
                self.files.prefs.map_preview = self.selected.is_some();
            }
        }
    }

    /// The rail at the right edge: a button for the Preview pane and one per Insights tab, shown
    /// while either is folded away.
    pub(super) fn right_rail(&mut self, ui: &mut egui::Ui) {
        let prefs = &self.files.prefs;
        if prefs.map_preview && prefs.insights_open {
            return;
        }
        let t = kit::tokens(ui);
        egui::Panel::right("right-rail")
            .resizable(false)
            .exact_size(RAIL_W)
            .frame(egui::Frame::new().fill(t.pal.side).inner_margin(egui::Margin::symmetric(4, 6)))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                if !self.files.prefs.map_preview
                    && rail_button(ui, Icon::Eye, &format!("Show the Preview pane ({})", Self::preview_shortcut()), false).clicked()
                {
                    self.files.prefs.map_preview = true;
                }
                if !self.files.prefs.insights_open {
                    if !self.files.prefs.map_preview {
                        ui.add_space(2.0);
                        let (r, _) = ui.allocate_exact_size(vec2(28.0, 7.0), Sense::hover());
                        ui.painter().hline(r.x_range().shrink(4.0), r.center().y, egui::Stroke::new(1.0, t.divider));
                    }
                    for (tab, icon, name) in INSIGHTS {
                        if rail_button(ui, icon, &format!("{name} ({})", Self::insights_shortcut()), false).clicked() {
                            self.side_tab = tab;
                            self.files.prefs.insights_open = true;
                        }
                    }
                }
            });
    }

    /// The Insights panel, when it's open.
    pub(super) fn insights_panel(&mut self, ui: &mut egui::Ui) {
        if !self.files.prefs.insights_open {
            return;
        }
        let r = egui::Panel::right("exts")
            .default_size(self.files.prefs.insights_width)
            .size_range(240.0..=640.0)
            .resizable(true)
            .show(ui, |ui| self.side_panel(ui));
        self.files.prefs.insights_width = r.response.rect.width();
        self.help_regions.push((r.response.rect, self.side_topic()));
    }

    /// The Preview pane, when it's open.
    pub(super) fn preview_panel(&mut self, ui: &mut egui::Ui) {
        if !self.files.prefs.map_preview {
            return;
        }
        let pal = style::pal(ui);
        let r = egui::Panel::right("map-preview")
            .resizable(true)
            .default_size(self.files.prefs.map_preview_width)
            .size_range(220.0..=560.0)
            .frame(egui::Frame::new().fill(pal.side).inner_margin(egui::Margin { left: 10, right: 10, top: 4, bottom: 8 }))
            .show(ui, |ui| {
                self.preview_header(ui);
                let tall = ui.available_height();
                egui::ScrollArea::vertical().id_salt("map-preview-scroll").auto_shrink(false).show(ui, |ui| self.map_preview_pane(ui, tall));
            });
        self.files.prefs.map_preview_width = r.response.rect.width();
        self.help_regions.push((r.response.rect, topics::TREEMAP_NAV));
    }

    /// The Preview pane's header: its tabs, Pin, and Fold.
    fn preview_header(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let mut tab = self.files.prefs.map_preview_tab;
            kit::segmented(ui, &mut tab, &[(PreviewTab::Preview, "Preview"), (PreviewTab::Details, "Details")], false);
            self.files.prefs.map_preview_tab = tab;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if rail_button(ui, Icon::FoldRight, &format!("Fold the pane away ({})", Self::preview_shortcut()), false).clicked() {
                    self.files.prefs.map_preview = false;
                }
                let pin = self.files.prefs.map_preview_pin;
                let tip = if pin {
                    "Pinned: stays open with nothing selected. Click to let it open and fold by itself"
                } else {
                    "Opens when you pick a tile and folds away when nothing is selected. Click to keep it open"
                };
                if rail_button(ui, Icon::Pin, tip, pin).clicked() {
                    self.files.prefs.map_preview_pin = !pin;
                }
            });
        });
        ui.add_space(4.0);
    }

    /// The Insights panel's header: its tabs and Fold.
    pub(super) fn insights_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Fold and Help keep their place at the right; the tabs wrap in what's left.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if rail_button(ui, Icon::FoldRight, &format!("Fold Insights away ({})", Self::insights_shortcut()), false).clicked() {
                    self.files.prefs.insights_open = false;
                }
                if help::link_button(ui, "Help for this tab") {
                    self.help.show_topic(self.side_topic());
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true), |ui| {
                    for (tab, _, label) in INSIGHTS {
                        let tip = match tab {
                            SideTab::Types => "Space used by each file type; click one to highlight it on the map",
                            SideTab::Sizes => "How many files fall into each size range",
                            SideTab::Changes => "Compare with a saved snapshot to see what grew or shrank",
                            SideTab::Duplicates => "Find files with exactly the same contents",
                            SideTab::Sequences => "Numbered file sequences (renders, scans) and their missing frames",
                        };
                        ui.selectable_value(&mut self.side_tab, tab, label).on_hover_text(tip);
                    }
                });
            });
        });
    }
}
