//! Showing what system files and folders are (from `crate::known`): an info badge after their names
//! in the lists and tree, a card on hover, a section in the preview pane, a note in the status bar
//! over the treemap, and a "What Is This?" window from the right-click menus.

use super::*;
use crate::known::{self, Known, Verdict};
use crate::kit::icons::{self, Icon};
use egui::text::{LayoutJob, TextFormat};
use std::path::Path;

pub(super) fn verdict_color(v: Verdict) -> Color32 {
    let [r, g, b] = v.rgb();
    Color32::from_rgb(r, g, b)
}

/// Draw the ⓘ badge for `k` in `rect`, with its card on hover. Clicking it opens the window.
pub(super) fn paint_badge(ui: &egui::Ui, rect: Rect, k: &'static Known, id: egui::Id) -> bool {
    icons::paint(ui.painter(), rect, Icon::Info, verdict_color(k.verdict));
    let over = ui.rect_contains_pointer(rect.expand(2.0));
    if over {
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), id, egui::PopupAnchor::Pointer).gap(12.0).show(|ui| {
            ui.set_max_width(360.0);
            card(ui, k, None);
            ui.add_space(4.0);
            ui.weak("Click the icon to keep this open.");
        });
    }
    over && ui.input(|i| i.pointer.primary_clicked())
}

/// Title, verdict, what it is and what to do. `inside` names the item when the card is about
/// a folder around it.
pub(super) fn card(ui: &mut egui::Ui, k: &Known, inside: Option<&str>) {
    let c = verdict_color(k.verdict);
    if let Some(name) = inside {
        ui.label(egui::RichText::new(format!("{name} is inside:")).weak());
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(k.title).strong().size(14.0));
        ui.label(egui::RichText::new(k.verdict.label()).color(c).strong());
    });
    ui.add_space(2.0);
    ui.add(egui::Label::new(k.what).wrap());
    ui.add_space(4.0);
    rich(ui, k.advice);
}

/// Text with `commands` in monospace, so they can be read and copied.
fn rich(ui: &mut egui::Ui, text: &str) {
    let color = ui.visuals().text_color();
    let code_bg = ui.visuals().code_bg_color;
    let size = egui::TextStyle::Body.resolve(ui.style()).size;
    let mut job = LayoutJob::default();
    for (i, part) in text.split('`').enumerate() {
        let f = if i % 2 == 1 {
            TextFormat { font_id: FontId::monospace(size - 1.0), color, background: code_bg, ..Default::default() }
        } else {
            TextFormat { font_id: FontId::proportional(size), color, ..Default::default() }
        };
        job.append(part, 0.0, f);
    }
    job.wrap.max_width = ui.available_width();
    ui.add(egui::Label::new(job).wrap());
}

/// For the status bar: "Windows page file: let the system manage it", or the folder it's in.
pub(super) fn status_note(path: &Path) -> Option<String> {
    let (k, inside) = known::about(path)?;
    Some(if inside {
        format!("in {}: {}", k.title, k.verdict.label().to_lowercase())
    } else {
        format!("{}: {}", k.title, k.verdict.label().to_lowercase())
    })
}

/// The "What Is This?" window's subject.
pub(super) struct Explaining {
    pub path: PathBuf,
}

impl App {
    /// Open the window for `path` if Filescape knows it or a folder around it.
    pub(super) fn explain_path(&mut self, path: PathBuf) {
        if known::about(&path).is_some() {
            self.explaining = Some(Explaining { path });
        }
    }

    pub(super) fn explain_window(&mut self, ctx: &egui::Context) {
        let Some(ex) = &self.explaining else { return };
        let Some((k, inside)) = known::about(&ex.path) else {
            self.explaining = None;
            return;
        };
        let name = ex.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| ex.path.display().to_string());
        let mut open = true;
        let mut help = false;
        egui::Window::new("What Is This?")
            .id(egui::Id::new("explain-window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.add(egui::Label::new(egui::RichText::new(ex.path.display().to_string()).monospace().weak()).wrap());
                ui.add_space(6.0);
                card(ui, k, inside.then_some(name.as_str()));
                ui.add_space(8.0);
                help = ui.link("About system files and the verdicts…").clicked();
            });
        if help {
            self.help.show_topic(topics::SYSTEM_FILES);
        }
        if !open {
            self.explaining = None;
        }
    }
}

/// The preview pane's "What is this?" section.
pub(super) fn preview_section(ui: &mut egui::Ui, path: &Path, name: &str) {
    let Some((k, inside)) = known::about(path) else { return };
    ui.add_space(10.0);
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(Stroke::new(1.0, verdict_color(k.verdict).gamma_multiply(0.6)))
        .corner_radius(4.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Info, verdict_color(k.verdict));
                ui.label(egui::RichText::new("What is this?").strong());
            });
            card(ui, k, inside.then_some(name));
        });
}
