//! The AI Models workspace (Ctrl/⌘+4): the language and image models on this computer, found
//! where Ollama, LM Studio and Hugging Face keep them, in Downloads, in folders the user adds and
//! in the Storage Analysis scan. Grouped by model, with copies and unused Ollama downloads
//! marked (see `crate::models`).

use super::files::Workspace;
use super::*;
use crate::fsys::PathShow;
use crate::inspect::{self, How, Inspection};
use crate::kit::{self, Tone};
use crate::models::{self, Model};
use std::path::Path;
use std::sync::Mutex;

/// The model list, and the search that fills it.
#[derive(Default)]
pub(super) struct ModelsState {
    job: Option<ModelsJob>,
    pub list: Option<Vec<Model>>,
    /// The model picked in the list (index into `list`).
    pub chosen: Option<usize>,
    /// Folders looked in by the last search, as (label, folder).
    pub places: Vec<(String, PathBuf)>,
    /// The model being inspected, shown instead of the list.
    page: Option<ModelPage>,
}

/// A model's own page: what's inside it.
struct ModelPage {
    model: usize,
    tab: PageTab,
    rx: Option<mpsc::Receiver<Inspection>>,
    data: Option<Arc<Inspection>>,
    filter: String,
    sort: TensorCol,
    descending: bool,
    /// The model as layers, for the Structure tab.
    structure: Option<Arc<inspect::Structure>>,
    /// The group (index into `structure.groups`) the map is zoomed into.
    zoom: Option<usize>,
    colour: ColourBy,
    /// Tensors to show, in order (indexes into `data.tensors`), for `shown_for`.
    shown: Vec<usize>,
    shown_for: Option<(String, TensorCol, bool)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColourBy {
    Part,
    Kind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PageTab {
    Overview,
    Structure,
    Tensors,
    Metadata,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TensorCol {
    /// File order.
    Order,
    Name,
    Kind,
    Shape,
    Size,
}

struct ModelsJob {
    progress: Arc<models::Progress>,
    /// The scan of the folder being looked in, to cancel it.
    scan: Arc<Mutex<Option<Arc<scan::Progress>>>>,
    /// What's happening, for the status line.
    status: Arc<Mutex<String>>,
    rx: mpsc::Receiver<Option<Vec<Model>>>,
}

const ROW: f32 = 2.0 * ROW_H + 6.0;

impl App {
    /// Looks for models again: the known places, the user's folders and the analysis scan.
    fn start_models(&mut self) {
        self.cancel_models();
        let mut places: Vec<(String, PathBuf)> = models::known_places().into_iter().map(|(l, p)| (l.to_string(), p)).collect();
        for p in &self.files.prefs.model_folders {
            places.push(("Added".into(), p.clone()));
        }
        // The Storage Analysis scan is used as it is, without scanning again.
        let analysis = self.tree.clone().filter(|_| self.job.is_none()).map(|t| (t, self.scanned_path.clone()));
        let roots: Vec<PathBuf> = places.iter().map(|(_, p)| p.clone()).collect();
        let (tx, rx) = mpsc::channel();
        let job = ModelsJob {
            progress: Arc::new(models::Progress::default()),
            scan: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new("Looking for AI models…".into())),
            rx,
        };
        let (progress, slot, status) = (job.progress.clone(), job.scan.clone(), job.status.clone());
        std::thread::spawn(move || {
            let _ = tx.send(search(roots, analysis, &progress, &slot, &status));
        });
        self.models = ModelsState { job: Some(job), list: None, chosen: None, places, page: None };
    }

    pub(super) fn poll_models(&mut self, ctx: &egui::Context) {
        if let Some(j) = &self.models.job {
            match j.rx.try_recv() {
                Ok(list) => {
                    self.models.list = list;
                    self.models.job = None;
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => self.models.job = None,
            }
        }
    }

    fn cancel_models(&mut self) {
        if let Some(j) = self.models.job.take() {
            j.progress.cancel.store(true, Ordering::Relaxed);
            if let Some(s) = j.scan.lock().unwrap().as_ref() {
                s.cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    /// The toolbar while the AI Models workspace is on screen.
    pub(super) fn models_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            style::roomy(ui);
            self.workspace_switch(ui);
            let busy = self.models.job.is_some();
            if ui.add_enabled(!busy, egui::Button::new("⟳ Look again")).on_hover_text("Look for models again in every place on the left").clicked() {
                self.start_models();
            }
            if ui.button("✚ Add folder…").on_hover_text("Also look in a folder of your choice, such as a ComfyUI or Stable Diffusion folder").clicked()
                && let Some(dir) = rfd::FileDialog::new().pick_folder()
            {
                if !self.files.prefs.model_folders.contains(&dir) {
                    self.files.prefs.model_folders.push(dir);
                }
                self.start_models();
            }
            if help::link_button(ui, "Help for the AI Models workspace") {
                self.help.show_topic(topics::AI_MODELS);
            }
        });
    }

    /// The AI Models workspace below the toolbar.
    pub(super) fn models_workspace(&mut self, ui: &mut egui::Ui) {
        if self.models.list.is_none() && self.models.job.is_none() && self.models.places.is_empty() {
            self.start_models();
        }
        let pal = style::pal(ui);
        let side = egui::Frame::new().fill(pal.side).inner_margin(egui::Margin { left: 8, right: 6, top: 8, bottom: 6 });
        let r = egui::Panel::left("model-places").default_size(230.0).min_size(160.0).resizable(true).frame(side).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.models_places(ui));
        });
        self.help_regions.push((r.response.rect, topics::AI_MODELS));
        let chosen = self.models.chosen.filter(|&i| self.models.list.as_ref().is_some_and(|l| i < l.len()));
        if chosen.is_some() && self.models.page.is_none() {
            let frame = egui::Frame::new().fill(pal.chrome).inner_margin(egui::Margin::same(10));
            let r = egui::Panel::right("model-details").default_size(340.0).min_size(240.0).resizable(true).frame(frame).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.model_details(ui));
            });
            self.help_regions.push((r.response.rect, topics::AI_MODELS));
        }
        let frame = egui::Frame::new().fill(pal.chrome).inner_margin(egui::Margin::same(10));
        let r = egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            if self.models.page.is_some() && self.models.job.is_none() {
                self.model_page(ui);
            } else {
                self.models_main(ui);
            }
        });
        self.help_regions.push((r.response.rect, topics::AI_MODELS));
    }

    /// Left: where models were looked for.
    fn models_places(&mut self, ui: &mut egui::Ui) {
        ui.strong("Looked in");
        ui.add_space(4.0);
        let mut remove = None;
        for (label, p) in &self.models.places {
            ui.horizontal(|ui| {
                ui.label(label.as_str());
                if label == "Added"
                    && let Some(i) = self.files.prefs.model_folders.iter().position(|f| f == p)
                    && ui.small_button("✖").on_hover_text("Stop looking here").clicked()
                {
                    remove = Some(i);
                }
            });
            ui.add(egui::Label::new(egui::RichText::new(p.shown()).small().weak()).wrap());
            ui.add_space(4.0);
        }
        if self.tree.is_some() && self.job.is_none() {
            ui.label("Storage Analysis scan");
            ui.add(egui::Label::new(egui::RichText::new(self.scanned_path.shown()).small().weak()).wrap());
        }
        if self.models.places.is_empty() && self.tree.is_none() {
            ui.weak("No Ollama, LM Studio or Hugging Face folders on this computer.");
        }
        ui.add_space(8.0);
        ui.weak("Models elsewhere? Use Add folder, or scan a drive in Storage Analysis and look again.");
        if let Some(i) = remove {
            self.files.prefs.model_folders.remove(i);
            self.start_models();
        }
    }

    /// Middle: the summary and the list.
    fn models_main(&mut self, ui: &mut egui::Ui) {
        if let Some(j) = &self.models.job {
            let (read, total) = (j.progress.read.load(Ordering::Relaxed), j.progress.to_read.load(Ordering::Relaxed));
            let status = if total > 0 {
                format!("Checking copies… {} of {}", scan::human(read), scan::human(total))
            } else {
                j.status.lock().unwrap().clone()
            };
            let mut cancel = false;
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(status);
                cancel = ui.small_button("Cancel").clicked();
            });
            if cancel {
                self.cancel_models();
            }
            return;
        }
        let Some(list) = self.models.list.take() else {
            ui.label("The search was cancelled.");
            if ui.button("Look again").clicked() {
                self.start_models();
            }
            return;
        };
        self.models_list(ui, &list);
        self.models.list = Some(list);
    }

    fn models_list(&mut self, ui: &mut egui::Ui, list: &[Model]) {
        if list.is_empty() {
            ui.add_space(6.0);
            ui.heading("No AI models found");
            ui.label(
                "Filescape looks where Ollama, LM Studio and Hugging Face keep their models, in Downloads, \
                 in folders you add and in the Storage Analysis scan, for model files (.gguf, .safetensors, \
                 .ckpt, .pt, .onnx …) of 16 MB or more.",
            );
            return;
        }
        let total: u64 = list.iter().map(|m| m.size).sum();
        let extra: u64 = list.iter().map(|m| m.extra).sum();
        let unused: u64 = list.iter().filter(|m| m.orphan).map(|m| m.size).sum();
        let count = list.iter().filter(|m| !m.orphan).count() as u64;
        ui.horizontal_wrapped(|ui| {
            ui.heading(format!("{} model{} · {}", group(count), if count == 1 { "" } else { "s" }, scan::human(total)));
            if extra > 0 {
                ui.colored_label(ui.visuals().warn_fg_color, format!("  {} in extra copies", scan::human(extra)))
                    .on_hover_text("Models marked \"copy\" are byte-for-byte the same as another model higher in the list.");
            }
            if unused > 0 {
                ui.colored_label(ui.visuals().warn_fg_color, format!("  {} in Ollama downloads no model uses", scan::human(unused)))
                    .on_hover_text("Left behind by removed models or stopped downloads. Ollama doesn't need them.");
            }
        });
        ui.add_space(6.0);

        let mut chosen = None;
        let mut open = None;
        let mut inspect = None;
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW, list.len(), |ui, range| {
            for (k, m) in list[range.clone()].iter().enumerate() {
                let i = range.start + k;
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                let vis = ui.visuals().clone();
                let selected = self.models.chosen == Some(i);
                if selected {
                    ui.painter().rect_filled(rect, 4.0, vis.selection.bg_fill);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 4.0, vis.widgets.hovered.weak_bg_fill);
                } else if i % 2 == 1 {
                    ui.painter().rect_filled(rect, 0.0, vis.faint_bg_color);
                }
                let font = FontId::proportional(13.5);
                let small = FontId::proportional(12.5);
                let top = rect.top() + ROW_H / 2.0 + 3.0;
                let bottom = top + ROW_H;
                let clip = Rect::from_min_max(rect.left_top(), pos2(rect.right() - 90.0, rect.bottom()));
                let p = ui.painter().with_clip_rect(clip.intersect(ui.clip_rect()));
                let name_color = if selected { vis.selection.stroke.color } else { vis.strong_text_color() };
                p.text(pos2(rect.left() + 8.0, top), Align2::LEFT_CENTER, &m.name, font.clone(), name_color);
                ui.painter().text(pos2(rect.right() - 8.0, top), Align2::RIGHT_CENTER, scan::human(m.size), font.clone(), vis.text_color());
                ui.painter().text(pos2(rect.right() - 8.0, bottom), Align2::RIGHT_CENTER, m.source.label(), small.clone(), vis.weak_text_color());
                let mut job = egui::text::LayoutJob::default();
                let weak = egui::TextFormat { font_id: small.clone(), color: vis.weak_text_color(), ..Default::default() };
                let warn = egui::TextFormat { font_id: small.clone(), color: vis.warn_fg_color, ..Default::default() };
                job.append(&m.details(), 0.0, weak);
                if let Some(note) = note(m) {
                    job.append(&format!("   {note}"), 0.0, warn);
                }
                let galley = ui.painter().layout_job(job);
                p.galley(pos2(rect.left() + 8.0, bottom - galley.size().y / 2.0), galley, vis.text_color());
                let resp = resp.on_hover_text(m.files.first().map(|f| f.path.shown()).unwrap_or_default());
                if resp.clicked() {
                    chosen = Some(i);
                }
                if resp.double_clicked() {
                    inspect = Some(i);
                }
                resp.context_menu(|ui| {
                    if ui.button("Inspect").clicked() {
                        inspect = Some(i);
                        ui.close();
                    }
                    if ui.button("Show in Files").clicked() {
                        open = Some(i);
                        ui.close();
                    }
                    if ui.button("Copy path").clicked() {
                        if let Some(f) = m.files.first() {
                            ui.ctx().copy_text(f.path.shown());
                        }
                        ui.close();
                    }
                });
            }
        });
        if let Some(i) = chosen {
            self.models.chosen = Some(i);
        }
        if let Some(i) = inspect
            && let Some(m) = list.get(i)
        {
            self.models.chosen = Some(i);
            self.inspect_model(i, m);
        }
        if let Some(i) = open
            && let Some(f) = list.get(i).and_then(|m| m.files.first())
        {
            self.show_model_file(&f.path.clone());
        }
    }

    /// Opens the Files workspace at a model file, selected.
    fn show_model_file(&mut self, path: &Path) {
        let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else { return };
        self.set_workspace(Workspace::Files);
        self.files.reveal(dir.to_path_buf(), vec![name.to_string_lossy().into()]);
    }

    /// Right: everything known about the chosen model.
    fn model_details(&mut self, ui: &mut egui::Ui) {
        let Some(list) = self.models.list.take() else { return };
        let Some(m) = self.models.chosen.and_then(|i| list.get(i)) else {
            self.models.list = Some(list);
            return;
        };
        let mut open = None;
        let mut pick = None;
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(egui::RichText::new(&m.name).heading()).wrap());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                if ui.small_button("✖").on_hover_text("Close").clicked() {
                    self.models.chosen = None;
                }
            });
        });
        if let Some(n) = note(m) {
            ui.colored_label(ui.visuals().warn_fg_color, n);
        }
        ui.add_space(6.0);
        let i = &m.info;
        let precision = i.quant.as_deref().map(|q| format!("{q} ({})", precision_words(q)));
        let rows: Vec<(&str, Option<String>)> = vec![
            ("Format", Some(m.format.label().to_string())),
            ("Family", i.arch.clone()),
            ("Parameters", i.params.clone()),
            ("Precision", precision),
            ("Context", i.context.map(|c| format!("{} tokens", group(c)))),
            ("Kind", i.companion.then(|| "Add-on, used with a model".to_string())),
            ("From", Some(m.source.label().to_string())),
            ("Size", Some(scan::human(m.size))),
        ];
        egui::Grid::new("model-facts").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
            for (k, v) in rows {
                if let Some(v) = v {
                    ui.weak(k);
                    ui.label(v);
                    ui.end_row();
                }
            }
        });
        ui.add_space(8.0);
        ui.strong(if m.files.len() == 1 { "File" } else { "Files" });
        for f in &m.files {
            let r = ui.add(egui::Label::new(egui::RichText::new(f.path.shown())).wrap().sense(Sense::click())).on_hover_text("Click to show it in Files");
            if r.clicked() {
                open = Some(f.path.clone());
            }
            ui.weak(format!("{}{}", scan::human(f.size), if f.not_local { " · ☁ in the cloud, not read" } else { "" }));
            ui.add_space(2.0);
        }
        if !m.copies.is_empty() {
            ui.add_space(8.0);
            ui.strong("The same files are also in");
            for &c in &m.copies {
                if let Some(o) = list.get(c) {
                    let r = ui.add(egui::Label::new(format!("{} ({})", o.name, o.source.label())).sense(Sense::click()));
                    if r.on_hover_text(o.files.first().map(|f| f.path.shown()).unwrap_or_default()).clicked() {
                        pick = Some(c);
                    }
                }
            }
        }
        if m.orphan {
            ui.add_space(8.0);
            ui.label(
                "No installed Ollama model uses this file. It's usually left behind by a removed model or a \
                 stopped download, and Ollama doesn't need it.",
            );
        }
        ui.add_space(10.0);
        let mut look = false;
        ui.horizontal(|ui| {
            look = ui.button("🔍 Inspect").on_hover_text("See what's inside this model: its architecture, tensors and metadata").clicked();
            if ui.button("Show in Files").clicked() {
                open = m.files.first().map(|f| f.path.clone());
            }
        });
        if look && let Some(i) = self.models.chosen {
            self.inspect_model(i, m);
        }
        self.models.list = Some(list);
        if pick.is_some() {
            self.models.chosen = pick;
        }
        if let Some(p) = open {
            self.show_model_file(&p);
        }
    }
}

impl App {
    /// Opens a model's page and starts reading what's inside it.
    fn inspect_model(&mut self, i: usize, m: &Model) {
        let (tx, rx) = mpsc::channel();
        let m = m.clone();
        std::thread::spawn(move || {
            let _ = tx.send(inspect::inspect(&m));
        });
        self.models.page = Some(ModelPage {
            model: i,
            tab: PageTab::Overview,
            rx: Some(rx),
            data: None,
            filter: String::new(),
            sort: TensorCol::Order,
            descending: false,
            shown: Vec::new(),
            shown_for: None,
            structure: None,
            zoom: None,
            colour: ColourBy::Part,
        });
    }

    /// The model page, in place of the list.
    fn model_page(&mut self, ui: &mut egui::Ui) {
        let Some(list) = self.models.list.take() else { return };
        let Some(mut page) = self.models.page.take() else {
            self.models.list = Some(list);
            return;
        };
        let Some(m) = list.get(page.model) else {
            self.models.list = Some(list);
            return;
        };
        if let Some(rx) = &page.rx {
            match rx.try_recv() {
                Ok(d) => {
                    if !d.tensors.is_empty() {
                        page.structure = Some(Arc::new(inspect::structure(&d)));
                    }
                    page.data = Some(Arc::new(d));
                    page.rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(Duration::from_millis(50)),
                Err(mpsc::TryRecvError::Disconnected) => page.rx = None,
            }
        }
        let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape)) && !ui.ctx().egui_wants_keyboard_input();
        let mut open = None;
        ui.horizontal(|ui| {
            close |= ui.button("⏴ All models").on_hover_text("Back to the list (Esc)").clicked();
            ui.add_space(8.0);
            ui.add(egui::Label::new(egui::RichText::new(&m.name).heading()).truncate());
        });
        ui.horizontal_wrapped(|ui| {
            ui.weak(format!("{} · {} · {}", m.details(), scan::human(m.size), m.source.label()));
            if let Some(n) = note(m) {
                ui.colored_label(ui.visuals().warn_fg_color, n);
            }
        });
        ui.add_space(6.0);
        let tensors = page.data.as_ref().map_or(0, |d| d.tensors.len());
        let metas = page.data.as_ref().map_or(0, |d| d.meta.len());
        let tensors_label = format!("Tensors ({})", group(tensors as u64));
        let meta_label = format!("Metadata ({metas})");
        let mut tabs = vec![(PageTab::Overview, "Overview")];
        if tensors > 0 {
            tabs.push((PageTab::Structure, "Structure"));
            tabs.push((PageTab::Tensors, tensors_label.as_str()));
        }
        if metas > 0 {
            tabs.push((PageTab::Metadata, meta_label.as_str()));
        }
        if tabs.len() > 1 {
            kit::segmented(ui, &mut page.tab, &tabs, false);
            ui.add_space(8.0);
        }
        match (&page.data, page.tab) {
            (None, _) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Reading the model…");
                });
            }
            (Some(d), PageTab::Overview) => {
                let d = d.clone();
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    if let Some(p) = page_overview(ui, m, &d) {
                        open = Some(p);
                    }
                });
            }
            (Some(d), PageTab::Structure) => {
                let d = d.clone();
                if let Some(st) = page.structure.clone() {
                    page_structure(ui, &mut page, &d, &st);
                }
            }
            (Some(d), PageTab::Tensors) => {
                let d = d.clone();
                page_tensors(ui, &mut page, m, &d);
            }
            (Some(d), PageTab::Metadata) => {
                let d = d.clone();
                page_metadata(ui, &mut page.filter, &d);
            }
        }
        self.models.list = Some(list);
        if !close {
            self.models.page = Some(page);
        }
        if let Some(p) = open {
            self.show_model_file(&p);
        }
    }
}

/// Overview: the facts with where each comes from, the number types and the files. Returns a file
/// to show in Files, when one is clicked.
fn page_overview(ui: &mut egui::Ui, m: &Model, d: &Inspection) -> Option<PathBuf> {
    let mut open = None;
    if let Some(why) = &d.unread {
        kit::callout(ui, Tone::Neutral, why.clone());
        ui.add_space(8.0);
    }
    for p in &d.problems {
        kit::callout(ui, Tone::Warn, p.clone());
        ui.add_space(4.0);
    }
    if !d.facts.is_empty() {
        egui::Grid::new("inspect-facts").num_columns(3).spacing([14.0, 5.0]).striped(true).show(ui, |ui| {
            for f in &d.facts {
                ui.weak(f.label);
                let tone = match f.how {
                    How::Verified => Tone::Success,
                    How::Calculated => Tone::Accent,
                    How::Documented => Tone::Neutral,
                };
                kit::badge(ui, f.how.label(), tone).on_hover_text(f.how.explain());
                ui.add(egui::Label::new(&f.value).wrap());
                ui.end_row();
            }
        });
        ui.add_space(4.0);
        ui.weak("file: read from the model file · calculated: worked out from it · publisher: stated beside it, not checked");
    }
    if d.kinds.len() > 1 {
        ui.add_space(12.0);
        ui.strong("How the numbers are stored");
        ui.add_space(4.0);
        let total: u64 = d.kinds.iter().map(|k| k.bytes).sum::<u64>().max(1);
        egui::Grid::new("inspect-kinds").num_columns(4).spacing([14.0, 4.0]).show(ui, |ui| {
            for k in &d.kinds {
                let frac = k.bytes as f32 / total as f32;
                ui.label(&k.kind).on_hover_text(precision_words(&k.kind));
                ui.weak(format!("{} tensor{}", group(k.tensors as u64), if k.tensors == 1 { "" } else { "s" }));
                ui.label(scan::human(k.bytes));
                let (rect, _) = ui.allocate_exact_size(vec2(160.0, 10.0), Sense::hover());
                ui.painter().rect_filled(rect, 3.0, ui.visuals().faint_bg_color);
                let mut bar = rect;
                bar.set_width((rect.width() * frac).max(1.0));
                ui.painter().rect_filled(bar, 3.0, ui.visuals().selection.bg_fill);
                ui.end_row();
            }
        });
    }
    ui.add_space(12.0);
    ui.strong(if m.files.len() == 1 { "File" } else { "Files" });
    for f in &m.files {
        let r = ui.add(egui::Label::new(egui::RichText::new(f.path.shown())).wrap().sense(Sense::click())).on_hover_text("Click to show it in Files");
        if r.clicked() {
            open = Some(f.path.clone());
        }
        ui.weak(format!("{}{}", scan::human(f.size), if f.not_local { " · ☁ in the cloud, not read" } else { "" }));
        ui.add_space(2.0);
    }
    open
}

/// Colours for the parts of a model, readable in both themes.
fn part_colour(p: inspect::Part) -> Color32 {
    use inspect::Part::*;
    match p {
        Embeddings => Color32::from_rgb(0x3d, 0xb8, 0x86),
        Attention => Color32::from_rgb(0x4d, 0x8e, 0xf7),
        FeedForward => Color32::from_rgb(0xee, 0x8a, 0x3c),
        Experts => Color32::from_rgb(0xe0, 0xb3, 0x2e),
        Norm => Color32::from_rgb(0x8c, 0x91, 0x9b),
        Output => Color32::from_rgb(0xa0, 0x72, 0xf0),
        Other => Color32::from_rgb(0x2f, 0xb5, 0xc4),
    }
}

/// Colours for number types, by their place in `Inspection::kinds` (largest first).
fn kind_colour(i: usize) -> Color32 {
    const C: [Color32; 8] = [
        Color32::from_rgb(0x4d, 0x8e, 0xf7),
        Color32::from_rgb(0xee, 0x8a, 0x3c),
        Color32::from_rgb(0x3d, 0xb8, 0x86),
        Color32::from_rgb(0xa0, 0x72, 0xf0),
        Color32::from_rgb(0xe0, 0xb3, 0x2e),
        Color32::from_rgb(0x2f, 0xb5, 0xc4),
        Color32::from_rgb(0xe0, 0x5d, 0x7a),
        Color32::from_rgb(0x8c, 0x91, 0x9b),
    ];
    C[i.min(C.len() - 1)]
}

fn tensor_colour(d: &Inspection, t: usize, by: ColourBy) -> Color32 {
    let t = &d.tensors[t];
    match by {
        ColourBy::Part => part_colour(inspect::part_of(&t.name)),
        ColourBy::Kind => kind_colour(d.kinds.iter().position(|k| k.kind == t.kind).unwrap_or(7)),
    }
}

/// The colour bands for one group, (colour, bytes), in a steady order.
fn bands(d: &Inspection, st: &inspect::Structure, g: usize, by: ColourBy) -> Vec<(Color32, u64)> {
    match by {
        ColourBy::Part => {
            let p = st.group_parts(d, g);
            inspect::Part::ALL.iter().enumerate().filter(|(i, _)| p[*i] > 0).map(|(i, &part)| (part_colour(part), p[i])).collect()
        }
        ColourBy::Kind => {
            let mut v = vec![0u64; d.kinds.len().max(1)];
            for &t in &st.groups[g].tensors {
                let k = d.kinds.iter().position(|k| k.kind == d.tensors[t].kind).unwrap_or(0);
                v[k] += d.tensors[t].bytes;
            }
            v.into_iter().enumerate().filter(|(_, b)| *b > 0).map(|(i, b)| (kind_colour(i), b)).collect()
        }
    }
}

/// Structure: a strip of the model's layers in order, then the model as a treemap of its layers
/// and their tensors.
fn page_structure(ui: &mut egui::Ui, page: &mut ModelPage, d: &Inspection, st: &inspect::Structure) {
    ui.horizontal(|ui| {
        ui.weak("Colour by");
        kit::segmented(ui, &mut page.colour, &[(ColourBy::Part, "What it does"), (ColourBy::Kind, "Number type")], false);
        ui.add_space(12.0);
        // Legend.
        match page.colour {
            ColourBy::Part => {
                for (i, &p) in inspect::Part::ALL.iter().enumerate() {
                    if st.parts[i] > 0 {
                        swatch(ui, part_colour(p), p.label()).on_hover_text(format!("{}\n{}", p.explain(), scan::human(st.parts[i])));
                    }
                }
            }
            ColourBy::Kind => {
                for (i, k) in d.kinds.iter().enumerate() {
                    swatch(ui, kind_colour(i), &k.kind).on_hover_text(format!("{}\n{}", precision_words(&k.kind), scan::human(k.bytes)));
                }
            }
        }
    });
    ui.add_space(6.0);
    let mut zoom_to = None;

    // The strip: each layer as a column, height by its size, banded by colour.
    let layers: Vec<usize> = (0..st.groups.len()).filter(|&g| st.groups[g].layer.is_some()).collect();
    if !layers.is_empty() {
        ui.weak(format!("{} layers, in order. Taller layers take more space; click one to see it below.", st.layers()));
        let h = 56.0;
        let w = ui.available_width();
        let (strip, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
        let max = layers.iter().map(|&g| st.groups[g].bytes).max().unwrap_or(1).max(1);
        let col_w = (w / layers.len() as f32).max(1.0);
        let gap = if col_w >= 6.0 { 1.0 } else { 0.0 };
        let pointer = ui.ctx().pointer_hover_pos();
        for (k, &g) in layers.iter().enumerate() {
            let grp = &st.groups[g];
            let x0 = strip.left() + k as f32 * col_w;
            let col = Rect::from_min_max(pos2(x0, strip.top()), pos2(x0 + col_w - gap, strip.bottom()));
            let height = h * grp.bytes as f32 / max as f32;
            let mut y = strip.bottom();
            let total = grp.bytes.max(1) as f32;
            for (c, b) in bands(d, st, g, page.colour) {
                let bh = height * b as f32 / total;
                ui.painter().rect_filled(Rect::from_min_max(pos2(col.left(), y - bh), pos2(col.right(), y)), 0.0, c);
                y -= bh;
            }
            let hovered = pointer.is_some_and(|p| col.contains(p));
            if page.zoom == Some(g) || hovered {
                ui.painter().rect_stroke(col.expand(1.0), 1.0, Stroke::new(1.5, ui.visuals().strong_text_color()), egui::StrokeKind::Outside);
            }
            let resp = ui.interact(col, ui.id().with(("layer-col", g)), Sense::click());
            if resp.clicked() {
                zoom_to = Some(Some(g));
            }
            resp.on_hover_text(group_tip(d, st, g));
        }
        ui.add_space(8.0);
    }

    // The map.
    ui.horizontal(|ui| {
        if let Some(g) = page.zoom {
            if ui.button("⏴ Whole model").clicked() {
                zoom_to = Some(None);
            }
            ui.strong(&st.groups[g].name);
            ui.weak(format!("{} · {}", scan::human(st.groups[g].bytes), models::params_label(st.groups[g].elements)));
        } else {
            ui.weak("Each box is a layer or part of the model, sized by the space it takes, divided into its tensors. Click one to look closer.");
        }
    });
    ui.add_space(4.0);
    let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    let map = paint_model_map(ui, rect, page, d, st);
    if let Some(hit) = map {
        match hit {
            MapHit::Group(g) => {
                if resp.clicked() {
                    zoom_to = Some(Some(g));
                }
                let tip = group_tip(d, st, g);
                resp.on_hover_ui_at_pointer(|ui| {
                    ui.label(tip);
                });
            }
            MapHit::Tensor(t) => {
                let tt = &d.tensors[t];
                let open = resp.clicked();
                let tip = format!(
                    "{}\n{} · {} · {} values · {}\n{}\nClick to find it in Tensors",
                    tt.name,
                    tt.kind,
                    tt.shape_text(),
                    group(tt.elements),
                    scan::human(tt.bytes),
                    inspect::part_of(&tt.name).label()
                );
                resp.on_hover_ui_at_pointer(|ui| {
                    ui.label(tip);
                });
                if open {
                    page.filter = tt.name.clone();
                    page.tab = PageTab::Tensors;
                }
            }
        }
    } else if resp.secondary_clicked() {
        zoom_to = Some(None);
    }
    if let Some(z) = zoom_to {
        page.zoom = z;
    }
}

fn group_tip(d: &Inspection, st: &inspect::Structure, g: usize) -> String {
    let grp = &st.groups[g];
    let mut s = format!("{}\n{} · {} parameters · {} tensors", grp.name, scan::human(grp.bytes), models::params_label(grp.elements), grp.tensors.len());
    let parts = st.group_parts(d, g);
    for (i, p) in inspect::Part::ALL.iter().enumerate() {
        if parts[i] > 0 {
            s.push_str(&format!("\n{}: {}", p.label(), scan::human(parts[i])));
        }
    }
    let mut kinds: Vec<(&str, u64)> = Vec::new();
    for &t in &grp.tensors {
        let k = d.tensors[t].kind.as_str();
        match kinds.iter_mut().find(|(n, _)| *n == k) {
            Some(e) => e.1 += d.tensors[t].bytes,
            None => kinds.push((k, d.tensors[t].bytes)),
        }
    }
    kinds.sort_by(|a, b| b.1.cmp(&a.1));
    s.push_str(&format!("\nStored as {}", kinds.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")));
    s
}

fn swatch(ui: &mut egui::Ui, c: Color32, label: &str) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
        ui.painter().rect_filled(r, 2.0, c);
        ui.label(egui::RichText::new(label).size(12.5));
    })
    .response
}

enum MapHit {
    Group(usize),
    Tensor(usize),
}

/// Draws the model (or the zoomed-in group) as a treemap and says what the pointer is over.
fn paint_model_map(ui: &mut egui::Ui, rect: Rect, page: &ModelPage, d: &Inspection, st: &inspect::Structure) -> Option<MapHit> {
    use crate::treemap::{child_rects, DRect};
    let painter = ui.painter_at(rect);
    let vis = ui.visuals().clone();
    painter.rect_filled(rect, 4.0, vis.extreme_bg_color);
    let pointer = ui.ctx().pointer_hover_pos().filter(|p| rect.contains(*p));
    let to_rect = |r: &DRect| Rect::from_min_max(pos2(r.x0 as f32, r.y0 as f32), pos2(r.x1 as f32, r.y1 as f32));
    let world = DRect::new(rect.left() as f64, rect.top() as f64, rect.right() as f64, rect.bottom() as f64);
    let mut hit = None;
    let label_font = FontId::proportional(12.0);
    let small = FontId::proportional(11.0);
    // Draws the tensors of one group inside `area`.
    let tensors = |g: usize, node: &scan::Node, area: DRect, labels: bool, hit: &mut Option<MapHit>| {
        let kids = child_rects(node, &area, 2.0, None);
        for (i, r) in kids.iter().enumerate() {
            let t = st.groups[g].tensors[i];
            let rr = to_rect(r).shrink(0.5);
            let mut c = tensor_colour(d, t, page.colour);
            let over = pointer.is_some_and(|p| rr.contains(p));
            if over {
                c = c.gamma_multiply(1.25);
                *hit = Some(MapHit::Tensor(t));
            }
            painter.rect_filled(rr, 1.5, c);
            if labels && rr.width() > 70.0 && rr.height() > 18.0 {
                let name = &d.tensors[t].name;
                // Inside a layer, its prefix adds nothing.
                let short = inspect::layer_of(name).map_or(name.as_str(), |(p, n)| name.get(p.len() + n.to_string().len() + 2..).unwrap_or(name));
                let text = Color32::from_black_alpha(220);
                let p = painter.with_clip_rect(rr.shrink(3.0));
                p.text(rr.left_top() + vec2(5.0, 4.0), Align2::LEFT_TOP, short, label_font.clone(), text);
                if rr.height() > 34.0 {
                    p.text(rr.left_top() + vec2(5.0, 19.0), Align2::LEFT_TOP, format!("{} · {}", d.tensors[t].kind, scan::human(d.tensors[t].bytes)), small.clone(), text);
                }
            }
        }
    };
    match page.zoom {
        Some(g) => {
            let k = st.order.iter().position(|&o| o == g)?;
            tensors(g, &st.tree.children[k], world, true, &mut hit);
        }
        None => {
            let kids = child_rects(&st.tree, &world, 4.0, None);
            for (k, r) in kids.iter().enumerate() {
                let g = st.order[k];
                let gr = to_rect(r);
                let inner = DRect::new(r.x0 + 1.5, r.y0 + 1.5, r.x1 - 1.5, r.y1 - 1.5);
                if inner.w() <= 1.0 || inner.h() <= 1.0 {
                    continue;
                }
                let mut tensor_hit = None;
                tensors(g, &st.tree.children[k], inner, false, &mut tensor_hit);
                let over = pointer.is_some_and(|p| gr.contains(p));
                if over {
                    hit = Some(MapHit::Group(g));
                }
                painter.rect_stroke(gr.shrink(0.5), 2.0, Stroke::new(if over { 2.0 } else { 1.0 }, if over { vis.strong_text_color() } else { vis.extreme_bg_color }), egui::StrokeKind::Inside);
                if gr.width() > 56.0 && gr.height() > 22.0 {
                    let name = &st.groups[g].name;
                    let text = format!("{name}  {}", scan::human(st.groups[g].bytes));
                    let galley = painter.layout_no_wrap(text, label_font.clone(), Color32::WHITE);
                    let w = galley.size().x.min(gr.width() - 8.0);
                    let chip = Rect::from_min_size(gr.left_top() + vec2(3.0, 3.0), vec2(w + 8.0, galley.size().y + 4.0));
                    painter.rect_filled(chip, 3.0, Color32::from_black_alpha(150));
                    painter.with_clip_rect(chip).galley(chip.left_top() + vec2(4.0, 2.0), galley, Color32::WHITE);
                }
            }
        }
    }
    hit
}

/// Tensors: a searchable list, sorted by any column.
fn page_tensors(ui: &mut egui::Ui, page: &mut ModelPage, m: &Model, d: &Inspection) {
    ui.push_id("tensor-search", |ui| {
        ui.set_max_width(420.0);
        kit::search_field(ui, &mut page.filter, "Filter by name or type, such as attn or Q6_K");
    });
    let key = (page.filter.clone(), page.sort, page.descending);
    if page.shown_for.as_ref() != Some(&key) {
        page.shown = tensor_order(d, &page.filter, page.sort, page.descending);
        page.shown_for = Some(key);
    }
    let bytes: u64 = page.shown.iter().map(|&i| d.tensors[i].bytes).sum();
    let elements: u64 = page.shown.iter().map(|&i| d.tensors[i].elements).sum();
    ui.add_space(4.0);
    ui.weak(format!(
        "{} of {} tensors · {} parameters · {}",
        group(page.shown.len() as u64),
        group(d.tensors.len() as u64),
        models::params_label(elements),
        scan::human(bytes)
    ));
    ui.add_space(4.0);

    let w = ui.available_width();
    let cols: [(TensorCol, &str, f32); 5] = [(TensorCol::Name, "Name", 0.0), (TensorCol::Kind, "Type", 80.0), (TensorCol::Shape, "Shape", 150.0), (TensorCol::Shape, "Parameters", 100.0), (TensorCol::Size, "Size", 90.0)];
    let fixed: f32 = cols.iter().map(|c| c.2).sum();
    let name_w = (w - fixed).max(160.0);
    let x_of = |k: usize| -> f32 { if k == 0 { 0.0 } else { name_w + cols[1..k].iter().map(|c| c.2).sum::<f32>() } };
    let (head, _) = ui.allocate_exact_size(vec2(w, ROW_H + 4.0), Sense::hover());
    for (k, (col, label, cw)) in cols.iter().enumerate() {
        let width = if k == 0 { name_w } else { *cw };
        let r = Rect::from_min_size(pos2(head.left() + x_of(k), head.top()), vec2(width, head.height()));
        let resp = ui.interact(r, ui.id().with(("tensor-col", k)), Sense::click());
        let on = page.sort == *col && (k != 3 || page.sort == TensorCol::Shape);
        let arrow = if on && k != 3 { if page.descending { " ⏷" } else { " ⏶" } } else { "" };
        let align = if k >= 3 { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER };
        let x = if k >= 3 { r.right() - 8.0 } else { r.left() + 8.0 };
        let color = if resp.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() };
        ui.painter().text(pos2(x, r.center().y), align, format!("{label}{arrow}"), FontId::proportional(12.5), color);
        if resp.on_hover_text("Click to sort; click again to reverse").clicked() {
            if page.sort == *col {
                page.descending = !page.descending;
            } else {
                page.sort = *col;
                // Biggest first is the useful start for sizes.
                page.descending = matches!(col, TensorCol::Shape | TensorCol::Size);
            }
        }
    }
    ui.painter().hline(head.x_range(), head.bottom(), ui.visuals().widgets.noninteractive.bg_stroke);

    ui.spacing_mut().item_spacing.y = 0.0;
    let font = FontId::monospace(12.0);
    let plain = FontId::proportional(12.5);
    egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_H, page.shown.len(), |ui, range| {
        for row in range {
            let t = &d.tensors[page.shown[row]];
            let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
            let vis = ui.visuals();
            if resp.hovered() {
                ui.painter().rect_filled(rect, 3.0, vis.widgets.hovered.weak_bg_fill);
            } else if row % 2 == 1 {
                ui.painter().rect_filled(rect, 0.0, vis.faint_bg_color);
            }
            let y = rect.center().y;
            let cell = |k: usize| Rect::from_min_size(pos2(rect.left() + x_of(k), rect.top()), vec2(if k == 0 { name_w } else { cols[k].2 }, rect.height()));
            let name_clip = cell(0).shrink2(vec2(4.0, 0.0)).intersect(ui.clip_rect());
            ui.painter().with_clip_rect(name_clip).text(pos2(rect.left() + 8.0, y), Align2::LEFT_CENTER, &t.name, font.clone(), vis.text_color());
            ui.painter().text(pos2(cell(1).left() + 8.0, y), Align2::LEFT_CENTER, &t.kind, plain.clone(), vis.text_color());
            ui.painter().with_clip_rect(cell(2).intersect(ui.clip_rect())).text(pos2(cell(2).left() + 8.0, y), Align2::LEFT_CENTER, t.shape_text(), plain.clone(), vis.weak_text_color());
            ui.painter().text(pos2(cell(3).right() - 8.0, y), Align2::RIGHT_CENTER, count(t.elements), plain.clone(), vis.text_color());
            ui.painter().text(pos2(cell(4).right() - 8.0, y), Align2::RIGHT_CENTER, scan::human(t.bytes), plain.clone(), vis.text_color());
            let file = if m.files.len() > 1 { m.files.get(t.file).and_then(|f| f.path.file_name()).map(|n| format!("\nIn {}", n.to_string_lossy())).unwrap_or_default() } else { String::new() };
            let resp = resp.on_hover_text(format!("{}\n{} · {} · {} values · {}{file}", t.name, t.kind, t.shape_text(), group(t.elements), scan::human(t.bytes)));
            resp.context_menu(|ui| {
                if ui.button("Copy name").clicked() {
                    ui.ctx().copy_text(t.name.clone());
                    ui.close();
                }
            });
        }
    });
}

/// A number of values: exact below a million, then 17M, 8.2B.
fn count(n: u64) -> String {
    if n < 1_000_000 { group(n) } else { models::params_label(n) }
}

/// Indexes of the tensors matching `filter`, in the chosen order.
fn tensor_order(d: &Inspection, filter: &str, col: TensorCol, descending: bool) -> Vec<usize> {
    let words: Vec<String> = filter.split_whitespace().map(str::to_lowercase).collect();
    let mut v: Vec<usize> = (0..d.tensors.len())
        .filter(|&i| {
            let t = &d.tensors[i];
            words.iter().all(|w| t.name.to_lowercase().contains(w) || t.kind.to_lowercase() == *w)
        })
        .collect();
    let ts = &d.tensors;
    let name = |a: usize, b: usize| inspect::natural(&ts[a].name, &ts[b].name);
    // Reversing flips the column, while ties stay in name order.
    let dir = |o: std::cmp::Ordering| if descending { o.reverse() } else { o };
    match col {
        TensorCol::Order if descending => v.reverse(),
        TensorCol::Order => {}
        TensorCol::Name => v.sort_by(|&a, &b| dir(name(a, b))),
        TensorCol::Kind => v.sort_by(|&a, &b| dir(ts[a].kind.cmp(&ts[b].kind)).then_with(|| name(a, b))),
        TensorCol::Shape => v.sort_by(|&a, &b| dir(ts[a].elements.cmp(&ts[b].elements)).then_with(|| name(a, b))),
        TensorCol::Size => v.sort_by(|&a, &b| dir(ts[a].bytes.cmp(&ts[b].bytes)).then_with(|| name(a, b))),
    }
    v
}

/// Metadata: every key and value in the file, filterable.
fn page_metadata(ui: &mut egui::Ui, filter: &mut String, d: &Inspection) {
    ui.push_id("meta-search", |ui| {
        ui.set_max_width(420.0);
        kit::search_field(ui, filter, "Filter keys and values");
    });
    ui.add_space(6.0);
    let f = filter.to_lowercase();
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        egui::Grid::new("inspect-meta").num_columns(2).spacing([14.0, 4.0]).striped(true).show(ui, |ui| {
            for (k, v) in d.meta.iter().filter(|(k, v)| f.is_empty() || k.to_lowercase().contains(&f) || v.to_lowercase().contains(&f)) {
                ui.add(egui::Label::new(egui::RichText::new(k).monospace().size(12.0)).selectable(true));
                ui.add(egui::Label::new(v).wrap().selectable(true));
                ui.end_row();
            }
        });
    });
}

/// The warning shown after a model's details, if any.
fn note(m: &Model) -> Option<&'static str> {
    if m.orphan {
        Some("not used by any Ollama model")
    } else if m.extra > 0 {
        Some("copy")
    } else if !m.copies.is_empty() {
        Some("has copies")
    } else {
        None
    }
}

/// Q4_K_M → "about 4-bit, compressed"; F16 → "16-bit, full precision".
fn precision_words(q: &str) -> String {
    let u = q.to_ascii_uppercase();
    match u.as_str() {
        "F32" => return "32-bit, full precision".into(),
        "F16" | "BF16" => return "16-bit, full precision".into(),
        _ => {}
    }
    let bits: String = u.trim_start_matches(|c: char| c.is_ascii_alphabetic()).chars().take_while(|c| c.is_ascii_digit()).collect();
    if bits.is_empty() { "compressed".into() } else { format!("about {bits}-bit, compressed") }
}

/// Runs in the background: scans each folder, finds the models in it and in the analysis scan,
/// then looks for copies across all of them. `None` when cancelled.
fn search(
    roots: Vec<PathBuf>,
    analysis: Option<(Arc<Node>, PathBuf)>,
    progress: &models::Progress,
    scan_slot: &Mutex<Option<Arc<scan::Progress>>>,
    status: &Mutex<String>,
) -> Option<Vec<Model>> {
    let mut all = Vec::new();
    let opts = scan::Options { one_file_system: true, metric: scan::Metric::Logical, ..Default::default() };
    for root in roots {
        if progress.cancel.load(Ordering::Relaxed) {
            return None;
        }
        *status.lock().unwrap() = format!("Looking in {}…", root.shown());
        let sp = Arc::new(scan::Progress::default());
        *scan_slot.lock().unwrap() = Some(sp.clone());
        let tree = scan::scan(&root, &opts, &sp, &scan::new_live(&root));
        if sp.cancel.load(Ordering::Relaxed) {
            return None;
        }
        all.extend(models::find_in(&tree, &root, &progress.cancel)?);
    }
    if let Some((tree, root)) = analysis {
        *status.lock().unwrap() = "Looking in the Storage Analysis scan…".into();
        all.extend(models::find_in(&tree, &root, &progress.cancel)?);
    }
    *status.lock().unwrap() = "Checking for copies…".into();
    models::finish(&mut all, progress)?;
    Some(all)
}

#[cfg(test)]
mod tests {
    #[test]
    fn precision_words() {
        assert_eq!(super::precision_words("Q4_K_M"), "about 4-bit, compressed");
        assert_eq!(super::precision_words("IQ2_XS"), "about 2-bit, compressed");
        assert_eq!(super::precision_words("BF16"), "16-bit, full precision");
        assert_eq!(super::precision_words("MXFP4"), "about 4-bit, compressed");
    }
}
