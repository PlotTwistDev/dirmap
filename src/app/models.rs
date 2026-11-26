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
use std::sync::atomic::AtomicBool;

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
    /// The Storage view: space by app, and what can be freed.
    pub storage: super::storage::StorageState,
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
    /// The health checks, once the model is read.
    health: Vec<crate::health::Check>,
    /// Checking the files' checksums.
    verify: Option<Verify>,
    /// Each checked file's result: (file index, Ok(matches) or why it couldn't be read).
    verified: Vec<(usize, Result<bool, String>)>,
    /// Will it run: the context length and cache precision chosen, and graphics memory typed in
    /// when none was found (GB).
    context: u64,
    cache_q8: bool,
    vram_typed: f32,
    /// The tensor picked in Tensors, and its weight statistics (being read, or read).
    picked: Option<usize>,
    stats_job: Option<(usize, mpsc::Receiver<Result<crate::weights::Stats, String>>)>,
    stats: Option<(usize, Result<crate::weights::Stats, String>)>,
    /// Checking every tensor's numbers, from Health.
    scan_all: Option<WeightScan>,
    scanned: Option<Vec<WeightFinding>>,
    /// The model as layers, for the Structure tab.
    structure: Option<Arc<inspect::Structure>>,
    /// The group (index into `structure.groups`) the map is zoomed into.
    zoom: Option<usize>,
    colour: ColourBy,
    /// Tensors to show, in order (indexes into `data.tensors`), for `shown_for`.
    shown: Vec<usize>,
    shown_for: Option<(String, TensorCol, bool)>,
    /// Tokens: whether the model has a vocabulary, and the vocabulary once read.
    has_vocab: bool,
    vocab: Option<VocabState>,
    vocab_filter: String,
    vocab_script: Option<crate::vocab::Script>,
    /// Tokens to show (ids), for `vocab_shown_for`.
    vocab_shown: Vec<u32>,
    vocab_shown_for: Option<(String, Option<crate::vocab::Script>)>,
    /// Compare: the other model (index into the list), being read or compared.
    compare_with: Option<usize>,
    compare_rx: Option<mpsc::Receiver<Inspection>>,
    comparison: Option<Arc<(Inspection, crate::modeldiff::Comparison)>>,
    compare_show: CompareShow,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CompareShow {
    Changed,
    OnlyA,
    OnlyB,
    All,
}

enum VocabState {
    Reading(mpsc::Receiver<Result<crate::vocab::Vocab, String>>),
    Read(Arc<crate::vocab::Vocab>, Arc<crate::vocab::Summary>),
    Failed(String),
}

struct WeightScan {
    done: Arc<std::sync::atomic::AtomicU64>,
    total: u64,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Vec<WeightFinding>>,
}

/// What checking every tensor's numbers found: one entry per tensor read.
#[derive(Clone)]
struct WeightFinding {
    tensor: usize,
    stats: Result<crate::weights::Stats, String>,
}

struct Verify {
    read: Arc<std::sync::atomic::AtomicU64>,
    total: u64,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Vec<(usize, Result<bool, String>)>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColourBy {
    Part,
    Kind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PageTab {
    Overview,
    Health,
    Fit,
    Structure,
    Tensors,
    Tokens,
    Metadata,
    Compare,
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
    pub(super) fn start_models(&mut self) {
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
        let mut storage = std::mem::take(&mut self.models.storage);
        storage.forget();
        self.models = ModelsState { job: Some(job), list: None, chosen: None, places, page: None, storage };
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
            ui.add_space(6.0);
            // 0 Library, 1 Ask AI, 2 Lab, 3 Storage.
            let mut view = if self.models.storage.open { 3 } else if !self.ask.open { 0 } else if self.ask.lab { 2 } else { 1 };
            kit::segmented(ui, &mut view, &[(0, "Library"), (1, "💬 Ask AI"), (2, "🔬 Lab"), (3, "🖴 Storage")], false).on_hover_text(
                "Library: the models on this computer. Ask AI: chat with one of them. Lab: see what a model would write next and how sure it is. Storage: how much space models take, and what can be freed.",
            );
            self.ask.open = view == 1 || view == 2;
            self.ask.lab = view == 2;
            self.models.storage.open = view == 3;
            ui.add_space(6.0);
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
        self.storage_confirm(ui.ctx());
        self.storage_moving(ui.ctx());
        self.storage_shrink(ui.ctx());
        if self.ask.open {
            self.ask_workspace(ui);
            return;
        }
        let pal = style::pal(ui);
        if self.models.storage.open {
            let frame = egui::Frame::new().fill(pal.chrome).inner_margin(egui::Margin::same(12));
            let r = egui::CentralPanel::default().frame(frame).show(ui, |ui| self.storage_view(ui));
            self.help_regions.push((r.response.rect, topics::AI_MODELS));
            return;
        }
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
        if let Some((tone, text)) = self.models.storage.outcome.clone() {
            kit::callout(ui, tone, text);
            if ui.small_button("Hide").clicked() {
                self.models.storage.outcome = None;
            }
        }
        ui.add_space(6.0);

        let mut chosen = None;
        let mut open = None;
        let mut inspect = None;
        let mut remove = None;
        let mut shrink = None;
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
                    if m.runnable() && ui.button("Shrink (quantise)…").on_hover_text("Make a smaller copy of this model. The original is kept. Professional.").clicked() {
                        shrink = Some(i);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(if cfg!(windows) { "Move to Recycle Bin…" } else { "Move to Trash…" }).on_hover_text("Remove this model. You see everything it removes first.").clicked() {
                        remove = Some(i);
                        ui.close();
                    }
                });
            }
        });
        if let Some(i) = chosen {
            self.models.chosen = Some(i);
        }
        if let Some(m) = remove.and_then(|i| list.get(i)) {
            self.ask_remove_model(m);
        }
        if let Some(m) = shrink.and_then(|i| list.get(i)) {
            self.ask_shrink(m);
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
        let mut ask = false;
        ui.horizontal(|ui| {
            look = ui.button("🔍 Inspect").on_hover_text("See what's inside this model: its architecture, tensors and metadata").clicked();
            if m.runnable() && ui.button("💬 Ask AI").on_hover_text("Chat with this model, run by Filescape on this computer").clicked() {
                ask = true;
            }
            if ui.button("Show in Files").clicked() {
                open = m.files.first().map(|f| f.path.clone());
            }
        });
        if look && let Some(i) = self.models.chosen {
            self.inspect_model(i, m);
        }
        if ask {
            self.ask_about_model(m);
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
            health: Vec::new(),
            context: 0,
            cache_q8: false,
            picked: None,
            stats_job: None,
            stats: None,
            scan_all: None,
            scanned: None,
            vram_typed: 0.0,
            verify: None,
            verified: Vec::new(),
            zoom: None,
            colour: ColourBy::Part,
            has_vocab: false,
            vocab: None,
            vocab_filter: String::new(),
            vocab_script: None,
            vocab_shown: Vec::new(),
            vocab_shown_for: None,
            compare_with: None,
            compare_rx: None,
            comparison: None,
            compare_show: CompareShow::Changed,
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
                    page.health = crate::health::checks(m, &d);
                    page.has_vocab = !m.info.companion && crate::vocab::present(&d.meta, inspect::publisher_dir(m).as_deref());
                    page.data = Some(Arc::new(d));
                    page.rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(Duration::from_millis(50)),
                Err(mpsc::TryRecvError::Disconnected) => page.rx = None,
            }
        }
        if let (Some(rx), Some(a)) = (&page.compare_rx, &page.data) {
            match rx.try_recv() {
                Ok(b) => {
                    let c = crate::modeldiff::compare(a, &b);
                    page.comparison = Some(Arc::new((b, c)));
                    page.compare_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(Duration::from_millis(50)),
                Err(mpsc::TryRecvError::Disconnected) => page.compare_rx = None,
            }
        }
        let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape)) && !ui.ctx().egui_wants_keyboard_input();
        let mut open = None;
        ui.horizontal(|ui| {
            close |= ui.button("⏴ All models").on_hover_text("Back to the list (Esc)").clicked();
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if page.data.is_some() && list.len() > 1 {
                    let mut pick = None;
                    egui::ComboBox::from_id_salt("compare-with")
                        .selected_text(page.compare_with.and_then(|c| list.get(c)).map_or("Compare with…".to_string(), |o| format!("Compared with {}", o.name)))
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            for (i, o) in list.iter().enumerate() {
                                if i == page.model || o.info.companion || o.files.iter().all(|f| f.not_local) {
                                    continue;
                                }
                                if ui.selectable_label(page.compare_with == Some(i), format!("{}  ·  {}", o.name, o.details())).clicked() {
                                    pick = Some(i);
                                }
                            }
                        })
                        .response
                        .on_hover_text("Compare this model with another: their facts, layers, tensors and metadata side by side");
                    if let Some(i) = pick {
                        let (tx, rx) = mpsc::channel();
                        let o = list[i].clone();
                        std::thread::spawn(move || {
                            let _ = tx.send(inspect::inspect(&o));
                        });
                        page.compare_with = Some(i);
                        page.compare_rx = Some(rx);
                        page.comparison = None;
                        page.tab = PageTab::Compare;
                    }
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(&m.name).heading()).truncate());
                });
            });
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
        let problems = page.health.iter().filter(|c| c.level == crate::health::Level::Problem).count();
        let health_label = if problems > 0 { format!("Health ({problems} ⚠)") } else { "Health".to_string() };
        let mut tabs = vec![(PageTab::Overview, "Overview")];
        if page.data.is_some() {
            tabs.push((PageTab::Health, health_label.as_str()));
        }
        if page.data.as_ref().is_some_and(|d| d.shape.kv_per_token(2.0).is_some()) && !m.info.companion {
            tabs.push((PageTab::Fit, "Will it run?"));
        }
        if tensors > 0 {
            tabs.push((PageTab::Structure, "Structure"));
            tabs.push((PageTab::Tensors, tensors_label.as_str()));
        }
        if page.has_vocab {
            tabs.push((PageTab::Tokens, "Tokens"));
        }
        if metas > 0 {
            tabs.push((PageTab::Metadata, meta_label.as_str()));
        }
        if page.compare_with.is_some() {
            tabs.push((PageTab::Compare, "Compare"));
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
            (Some(d), PageTab::Health) => {
                let d = d.clone();
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    page_health(ui, &mut page, m);
                    if !d.tensors.is_empty() {
                        ui.add_space(10.0);
                        weight_scan(ui, &mut page, m, &d);
                    }
                });
            }
            (Some(d), PageTab::Fit) => {
                let d = d.clone();
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| page_fit(ui, &mut page, m, &d));
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
            (Some(_), PageTab::Tokens) => page_tokens(ui, &mut page, m),
            (Some(d), PageTab::Compare) => {
                let d = d.clone();
                let other = page.compare_with.and_then(|c| list.get(c));
                if let Some(o) = other {
                    page_compare(ui, &mut page, m, o, &d);
                }
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

/// Health: the checks, then checking the files' checksums.
fn page_health(ui: &mut egui::Ui, page: &mut ModelPage, m: &Model) {
    use crate::health::Level;
    for c in &page.health {
        let (icon, tone) = match c.level {
            Level::Problem => ("✖", Tone::Danger),
            Level::Warn => ("⚠", Tone::Warn),
            Level::Good => ("✔", Tone::Success),
            Level::Info => ("ℹ", Tone::Neutral),
        };
        let colour = kit::tokens(ui).tone(tone);
        ui.horizontal_top(|ui| {
            ui.add_sized(vec2(18.0, 18.0), egui::Label::new(egui::RichText::new(icon).color(colour).size(14.0)));
            ui.vertical(|ui| {
                ui.strong(&c.title);
                ui.add(egui::Label::new(egui::RichText::new(&c.detail).color(ui.visuals().weak_text_color())).wrap());
            });
        });
        ui.add_space(8.0);
    }

    // Checksums.
    let named: Vec<(usize, String)> = m.files.iter().enumerate().filter(|(_, f)| !f.not_local).filter_map(|(i, f)| crate::health::checksum_of(&f.path).map(|h| (i, h))).collect();
    ui.add_space(6.0);
    ui.strong("Checksum");
    if named.is_empty() {
        ui.weak("This model's files aren't named by their checksum (as Ollama's and Hugging Face's downloads are), so there's no checksum to compare with.");
        return;
    }
    if let Some(v) = &page.verify {
        match v.rx.try_recv() {
            Ok(r) => {
                page.verified = r;
                page.verify = None;
            }
            Err(mpsc::TryRecvError::Empty) => {
                let read = v.read.load(Ordering::Relaxed);
                let mut stop = false;
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Reading… {} of {}", scan::human(read), scan::human(v.total)));
                    stop = ui.small_button("Cancel").clicked();
                });
                kit::progress(ui, read as f32 / v.total.max(1) as f32, Tone::Accent);
                if stop {
                    v.cancel.store(true, Ordering::Relaxed);
                }
                ui.ctx().request_repaint_after(Duration::from_millis(100));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => page.verify = None,
        }
    }
    if page.verified.is_empty() {
        let total: u64 = named.iter().map(|(i, _)| m.files[*i].size).sum();
        ui.label(format!(
            "The files are named by their SHA-256 checksum. Filescape can read {} and compare, to prove not a single byte has changed since download. This reads the whole model, which takes a while.",
            if named.len() == 1 { format!("the file ({})", scan::human(total)) } else { format!("all {} files ({})", named.len(), scan::human(total)) }
        ));
        ui.add_space(4.0);
        if ui.button("Verify checksum").clicked() {
            let read = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let cancel = Arc::new(AtomicBool::new(false));
            let (tx, rx) = mpsc::channel();
            let files: Vec<(usize, PathBuf, String)> = named.iter().map(|(i, h)| (*i, m.files[*i].path.clone(), h.clone())).collect();
            let (r, c) = (read.clone(), cancel.clone());
            std::thread::spawn(move || {
                let mut out = Vec::new();
                for (i, path, want) in files {
                    if c.load(Ordering::Relaxed) {
                        break;
                    }
                    // Progress counts across the files.
                    let before = r.load(Ordering::Relaxed);
                    let one = std::sync::atomic::AtomicU64::new(0);
                    let res = std::thread::scope(|s| {
                        let h = s.spawn(|| crate::actions::sha256(&path, &one, &c));
                        while !h.is_finished() {
                            r.store(before + one.load(Ordering::Relaxed), Ordering::Relaxed);
                            std::thread::sleep(Duration::from_millis(50));
                        }
                        h.join().unwrap_or_else(|_| Err(std::io::Error::other("failed")))
                    });
                    r.store(before + one.load(Ordering::Relaxed), Ordering::Relaxed);
                    if c.load(Ordering::Relaxed) {
                        break;
                    }
                    out.push((i, res.map(|got| got.eq_ignore_ascii_case(&want)).map_err(|e| e.to_string())));
                }
                let _ = tx.send(out);
            });
            let total = named.iter().map(|(i, _)| m.files[*i].size).sum();
            page.verify = Some(Verify { read, total, cancel, rx });
        }
        return;
    }
    for (i, r) in &page.verified {
        // Files named by their checksum say nothing more by name.
        let name = match m.files.get(*i).and_then(|f| f.path.file_name()).map(|n| n.to_string_lossy().into_owned()) {
            Some(n) if named.len() > 1 && crate::health::checksum_of(Path::new(&n)).is_none() => n,
            _ if named.len() > 1 => format!("Part {}", i + 1),
            _ => "The file".into(),
        };
        let t = kit::tokens(ui);
        match r {
            Ok(true) => ui.colored_label(t.tone(Tone::Success), format!("✔ {name}: the checksum matches, so it is exactly as downloaded.")),
            Ok(false) => ui.colored_label(t.tone(Tone::Danger), format!("✖ {name}: the checksum doesn't match. The file has changed or is damaged; download it again.")),
            Err(e) => ui.colored_label(t.tone(Tone::Warn), format!("⚠ {name}: couldn't be read ({e}).")),
        };
    }
    if page.verified.len() < named.len() {
        ui.weak("Stopped before every file was checked.");
    }
    if ui.small_button("Check again").clicked() {
        page.verified.clear();
    }
}

/// The computer's memory and graphics memory, found once in the background.
fn hardware(ctx: &egui::Context) -> Option<&'static crate::fit::Hardware> {
    static HW: std::sync::OnceLock<crate::fit::Hardware> = std::sync::OnceLock::new();
    static STARTED: AtomicBool = AtomicBool::new(false);
    if HW.get().is_none() && !STARTED.swap(true, Ordering::Relaxed) {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = HW.set(crate::fit::detect());
            ctx.request_repaint();
        });
    }
    HW.get()
}

/// Will it run? Memory needed at a chosen context length, against this computer.
fn page_fit(ui: &mut egui::Ui, page: &mut ModelPage, m: &Model, d: &Inspection) {
    use crate::fit::{self, Verdict};
    let Some(hw) = hardware(ui.ctx()) else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Looking at this computer's memory…");
        });
        return;
    };
    let weights = match d.tensors.iter().map(|t| t.bytes).sum::<u64>() {
        0 => m.size,
        w => w,
    };
    let contexts = fit::contexts(d.shape.context);
    if !contexts.contains(&page.context) {
        page.context = contexts.iter().copied().filter(|&c| c <= 8192).max().unwrap_or(contexts[0]);
    }

    // This computer.
    let detected = hw.usable_vram();
    let vram = if detected > 0 {
        detected
    } else {
        // As for a card found: less what the screen and other apps keep.
        let v = (page.vram_typed.max(0.0) as f64 * (1u64 << 30) as f64) as u64;
        crate::fit::Hardware { ram: 0, gpus: vec![crate::fit::Gpu { name: String::new(), vram: v }], unified: false }.usable_vram()
    };
    ui.strong("This computer");
    if hw.unified {
        ui.label(format!(
            "{} of memory, shared with the graphics chip. About {} of it can hold a model.",
            scan::human(hw.ram),
            scan::human(detected)
        ));
    } else {
        let gpus = hw.gpus.iter().map(|g| format!("{} ({})", g.name, scan::human(g.vram))).collect::<Vec<_>>().join(", ");
        if gpus.is_empty() {
            ui.label(format!("{} of memory. Filescape couldn't find how much memory the graphics card has.", scan::human(hw.ram)));
            ui.horizontal(|ui| {
                ui.label("Graphics memory:");
                ui.add(egui::DragValue::new(&mut page.vram_typed).range(0.0..=256.0).speed(0.5).suffix(" GB"));
                ui.weak("(0 if none, or if it shares main memory)");
            });
        } else {
            ui.label(format!("{} of memory, and {gpus}.", scan::human(hw.ram)));
        }
    }
    ui.add_space(10.0);

    // The choices.
    let mut idx = contexts.iter().position(|&c| c == page.context).unwrap_or(0);
    ui.horizontal(|ui| {
        ui.label("Context");
        let max = contexts.len() - 1;
        ui.spacing_mut().slider_width = 280.0;
        ui.add(egui::Slider::new(&mut idx, 0..=max).show_value(false));
        ui.strong(format!("{} tokens", fit::tokens(contexts[idx])));
        ui.weak(format!("about {} words", group(contexts[idx] * 3 / 4)));
    });
    page.context = contexts[idx];
    ui.horizontal(|ui| {
        ui.label("Cache");
        let mut q8 = page.cache_q8;
        kit::segmented(ui, &mut q8, &[(false, "Full (F16)"), (true, "Compact (Q8)")], false)
            .on_hover_text("How the model's memory of the conversation is stored. Compact halves it, with little effect on answers.");
        page.cache_q8 = q8;
    });
    ui.add_space(8.0);

    let cache_bytes = if page.cache_q8 { 1.0625 } else { 2.0 };
    let Some(n) = fit::need(d, weights, page.context, cache_bytes) else { return };
    let verdict = fit::verdict(&n, hw, vram);
    let (tone, text) = match verdict {
        Verdict::Gpu => (Tone::Success, format!("Runs fully on the graphics chip: fast. It needs about {}.", scan::human(n.total()))),
        Verdict::Split(f) => (
            Tone::Warn,
            format!("Runs, split between the graphics card and main memory (about {:.0}% on the card): slower than all on the card. It needs about {}.", f * 100.0, scan::human(n.total())),
        ),
        Verdict::Cpu => (Tone::Warn, format!("Runs in main memory on the processor: it works, but slowly. It needs about {}.", scan::human(n.total()))),
        Verdict::TooBig => (Tone::Danger, format!("Too big for this computer: it needs about {}. Try a shorter context, a compact cache, or a smaller version of the model.", scan::human(n.total()))),
    };
    kit::callout(ui, tone, text);
    ui.add_space(8.0);

    // The need as a bar, with what fits marked.
    let w = ui.available_width().min(760.0);
    let scale_max = n.total().max(vram).max(hw.usable_ram()) as f32 * 1.05;
    let (bar, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(bar, 4.0, ui.visuals().faint_bg_color);
    let mut x = bar.left();
    let parts = [
        (n.weights, Color32::from_rgb(0x4d, 0x8e, 0xf7), "Weights", "The model itself."),
        (n.cache, Color32::from_rgb(0xee, 0x8a, 0x3c), "Cache", "Its memory of the conversation, which grows with the context."),
        (n.work, Color32::from_rgb(0x8c, 0x91, 0x9b), "Working space", "Room for the calculations."),
    ];
    for (b, c, _, _) in parts {
        let pw = bar.width() * b as f32 / scale_max;
        painter.rect_filled(Rect::from_min_size(pos2(x, bar.top()), vec2(pw, bar.height())), 0.0, c);
        x += pw;
    }
    let weak = ui.visuals().weak_text_color();
    let mark = |v: u64, label: String| {
        if v == 0 {
            return;
        }
        let mx = bar.left() + bar.width() * v as f32 / scale_max;
        painter.vline(mx, bar.top() - 4.0..=bar.bottom() + 16.0, Stroke::new(1.5, ui.visuals().strong_text_color()));
        painter.text(pos2(mx + 3.0, bar.bottom() + 2.0), Align2::LEFT_TOP, label, FontId::proportional(11.5), weak);
    };
    if vram > 0 {
        mark(vram, if hw.unified { "for models".into() } else { "graphics".into() });
    }
    if !hw.unified {
        mark(hw.usable_ram(), "main memory".into());
    }
    ui.add_space(20.0);
    ui.horizontal_wrapped(|ui| {
        for (b, c, label, tip) in parts {
            swatch(ui, c, &format!("{label} {}", scan::human(b))).on_hover_text(tip);
        }
    });
    ui.add_space(12.0);

    // At other context lengths.
    ui.strong("At other context lengths");
    egui::Grid::new("fit-table").num_columns(3).spacing([18.0, 4.0]).striped(true).show(ui, |ui| {
        ui.weak("Context");
        ui.weak("Needs");
        ui.weak("On this computer");
        ui.end_row();
        for &c in contexts.iter().filter(|&&c| c >= 2048 || contexts.len() < 4) {
            let Some(n) = fit::need(d, weights, c, cache_bytes) else { continue };
            let v = match fit::verdict(&n, hw, vram) {
                Verdict::Gpu => "On the graphics chip",
                Verdict::Split(_) => "Split",
                Verdict::Cpu => "On the processor",
                Verdict::TooBig => "Too big",
            };
            let row = if c == page.context { egui::RichText::new(fit::tokens(c)).strong() } else { egui::RichText::new(fit::tokens(c)) };
            ui.label(row);
            ui.label(scan::human(n.total()));
            ui.label(v);
            ui.end_row();
        }
    });

    // Smaller versions.
    let elements: u64 = d.tensors.iter().map(|t| t.elements).sum();
    if elements > 0 {
        ui.add_space(12.0);
        ui.strong("Smaller versions of this model");
        ui.weak("Most models are also published compressed. At the same context, roughly:");
        let stored = weights as f64 * 8.0 / elements as f64;
        for (q, bpw) in [("Q8_0", 8.5), ("Q6_K", 6.56), ("Q4_K_M", 4.85), ("Q3_K_M", 3.9)] {
            if bpw >= stored - 0.2 {
                continue;
            }
            let wq = (elements as f64 * bpw / 8.0) as u64;
            if let Some(n) = fit::need(d, wq, page.context, cache_bytes) {
                let v = match fit::verdict(&n, hw, vram) {
                    Verdict::Gpu => "on the graphics chip",
                    Verdict::Split(_) => "split",
                    Verdict::Cpu => "on the processor",
                    Verdict::TooBig => "too big",
                };
                ui.label(format!("{q} ({}): about {}, {v}", precision_words(q), scan::human(n.total())));
            }
        }
    }
    ui.add_space(8.0);
    ui.weak("These are estimates for llama.cpp-based apps such as Ollama and LM Studio; each app adds a little of its own.");
}

/// The picked tensor: its facts, and a sample of its numbers as statistics and a histogram.
fn tensor_detail(ui: &mut egui::Ui, page: &mut ModelPage, m: &Model, d: &Inspection, t: usize) {
    let tt = &d.tensors[t];
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(egui::RichText::new(&tt.name).monospace().strong()).wrap());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
            if ui.small_button("✖").on_hover_text("Close").clicked() {
                page.picked = None;
            }
        });
    });
    ui.weak(format!("{} · {}", inspect::part_of(&tt.name).label(), precision_words(&tt.kind)));
    ui.add_space(6.0);
    egui::Grid::new("tensor-facts").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
        for (k, v) in [
            ("Type", tt.kind.clone()),
            ("Shape", tt.shape_text()),
            ("Values", group(tt.elements)),
            ("Size", scan::human(tt.bytes)),
            ("Starts at", format!("byte {}", group(tt.offset))),
        ] {
            ui.weak(k);
            ui.label(v);
            ui.end_row();
        }
    });
    ui.add_space(10.0);
    ui.strong("Its numbers");
    let Some(file) = m.files.get(tt.file) else { return };
    if file.not_local {
        ui.weak("This file is only in the cloud. Reading it would download it, so Filescape hasn't.");
        return;
    }
    if !crate::weights::readable(&tt.kind) {
        ui.weak(format!("Filescape can't read {} numbers yet.", tt.kind));
        return;
    }
    // Start reading when picked; take the result when it's ready.
    if page.stats.as_ref().is_none_or(|(i, _)| *i != t) && page.stats_job.as_ref().is_none_or(|(i, _)| *i != t) {
        let (tx, rx) = mpsc::channel();
        let (path, tensor, ctx) = (file.path.clone(), tt.clone(), ui.ctx().clone());
        std::thread::spawn(move || {
            let r = crate::weights::sample(&path, &tensor, 32, 64 << 10).map(|v| crate::weights::Stats::of(&v, tensor.elements, 60));
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        page.stats_job = Some((t, rx));
    }
    if let Some((i, rx)) = &page.stats_job
        && let Ok(r) = rx.try_recv()
    {
        page.stats = Some((*i, r));
        page.stats_job = None;
    }
    match &page.stats {
        Some((i, Ok(s))) if *i == t => stats_view(ui, s),
        Some((i, Err(e))) if *i == t => {
            ui.colored_label(ui.visuals().warn_fg_color, format!("Couldn't read them: {e}."));
        }
        _ => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading…");
            });
        }
    }
}

/// Statistics and a histogram of a sample of numbers.
fn stats_view(ui: &mut egui::Ui, s: &crate::weights::Stats) {
    let t = kit::tokens(ui);
    if s.sampled < s.total {
        ui.weak(format!("A sample of {} of its {} values, from across the tensor.", group(s.sampled), group(s.total)));
    } else {
        ui.weak(format!("All {} values.", group(s.total)));
    }
    if s.nan + s.inf > 0 {
        ui.colored_label(t.tone(Tone::Danger), format!("✖ {} broken values (NaN or infinite). The model will give wrong answers.", group(s.nan + s.inf)));
    }
    ui.add_space(4.0);
    let fmt = |v: f64| if v != 0.0 && (v.abs() < 1e-3 || v.abs() >= 1e4) { format!("{v:.3e}") } else { format!("{v:.4}") };
    egui::Grid::new("tensor-stats").num_columns(2).spacing([12.0, 3.0]).show(ui, |ui| {
        let zero_pct = s.zeros as f64 * 100.0 / s.sampled.max(1) as f64;
        for (k, v, tip) in [
            ("Average", fmt(s.mean), "The mean of the values."),
            ("Spread", fmt(s.std), "Standard deviation: how far values typically are from the average."),
            ("Smallest", fmt(s.min as f64), ""),
            ("Largest", fmt(s.max as f64), ""),
            ("Zeros", format!("{zero_pct:.1}%"), "Values that are exactly 0. A tensor of nothing but zeros has usually been damaged or left out."),
        ] {
            ui.weak(k).on_hover_text(tip);
            ui.label(v);
            ui.end_row();
        }
    });
    if s.bins.is_empty() {
        return;
    }
    ui.add_space(8.0);
    let w = ui.available_width().min(420.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 110.0), Sense::hover());
    let painter = ui.painter_at(rect.expand(2.0));
    painter.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    let top = *s.bins.iter().max().unwrap_or(&1) as f32;
    let bw = rect.width() / s.bins.len() as f32;
    for (i, &b) in s.bins.iter().enumerate() {
        let h = (rect.height() - 4.0) * b as f32 / top.max(1.0);
        let r = Rect::from_min_max(pos2(rect.left() + i as f32 * bw + 0.5, rect.bottom() - h), pos2(rect.left() + (i + 1) as f32 * bw - 0.5, rect.bottom()));
        painter.rect_filled(r, 0.0, t.accent);
    }
    let (lo, hi) = s.range;
    if lo < 0.0 && hi > 0.0 {
        let x = rect.left() + rect.width() * (-lo) / (hi - lo);
        painter.vline(x, rect.y_range(), Stroke::new(1.0, ui.visuals().weak_text_color()));
    }
    if let Some(p) = resp.hover_pos() {
        let i = (((p.x - rect.left()) / bw) as usize).min(s.bins.len() - 1);
        let step = (hi - lo) / s.bins.len() as f32;
        let a = lo + step * i as f32;
        resp.on_hover_text(format!("{} to {}: {} values", fmt(a as f64), fmt((a + step) as f64), group(s.bins[i] as u64)));
    }
    ui.horizontal(|ui| {
        ui.weak(fmt(lo as f64));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.weak(fmt(hi as f64)));
    });
    if s.outside > 0 {
        ui.weak(format!("{} extreme values lie outside the chart.", group(s.outside)));
    }
}

/// Checking a sample of every tensor's numbers, from Health.
fn weight_scan(ui: &mut egui::Ui, page: &mut ModelPage, m: &Model, d: &Inspection) {
    use crate::weights;
    ui.add_space(6.0);
    ui.strong("Numbers");
    if let Some(job) = &page.scan_all {
        match job.rx.try_recv() {
            Ok(f) => {
                page.scanned = Some(f);
                page.scan_all = None;
            }
            Err(mpsc::TryRecvError::Empty) => {
                let done = job.done.load(Ordering::Relaxed);
                let mut stop = false;
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Reading tensor {} of {}…", done.min(job.total), job.total));
                    stop = ui.small_button("Cancel").clicked();
                });
                kit::progress(ui, done as f32 / job.total.max(1) as f32, Tone::Accent);
                if stop {
                    job.cancel.store(true, Ordering::Relaxed);
                }
                ui.ctx().request_repaint_after(Duration::from_millis(100));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => page.scan_all = None,
        }
    }
    let readable: Vec<usize> = (0..d.tensors.len()).filter(|&i| weights::readable(&d.tensors[i].kind) && m.files.get(d.tensors[i].file).is_some_and(|f| !f.not_local)).collect();
    let Some(found) = &page.scanned else {
        if readable.is_empty() {
            ui.weak("Filescape can't read this model's numbers yet.");
            return;
        }
        ui.label("Filescape can read a sample of every tensor's numbers to look for broken values (NaN or infinite) and tensors left empty. It reads about 128 KB of each tensor, not the whole model.");
        ui.add_space(4.0);
        if ui.button("Check the numbers").clicked() {
            let done = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let cancel = Arc::new(AtomicBool::new(false));
            let (tx, rx) = mpsc::channel();
            let jobs: Vec<(usize, PathBuf, inspect::Tensor)> = readable.iter().map(|&i| (i, m.files[d.tensors[i].file].path.clone(), d.tensors[i].clone())).collect();
            let (dn, c) = (done.clone(), cancel.clone());
            std::thread::spawn(move || {
                let mut out = Vec::new();
                for (i, path, t) in jobs {
                    if c.load(Ordering::Relaxed) {
                        break;
                    }
                    let stats = weights::sample(&path, &t, 8, 16 << 10).map(|v| weights::Stats::of(&v, t.elements, 1));
                    out.push(WeightFinding { tensor: i, stats });
                    dn.fetch_add(1, Ordering::Relaxed);
                }
                let _ = tx.send(out);
            });
            page.scan_all = Some(WeightScan { done, total: readable.len() as u64, cancel, rx });
        }
        let skipped = d.tensors.len() - readable.len();
        if skipped > 0 {
            ui.weak(format!("{} tensor{} in number types Filescape can't read yet will be skipped.", group(skipped as u64), if skipped == 1 { "" } else { "s" }));
        }
        return;
    };
    let t = kit::tokens(ui);
    let broken: Vec<&WeightFinding> = found.iter().filter(|f| f.stats.as_ref().is_ok_and(|s| s.nan + s.inf > 0)).collect();
    let empty: Vec<&WeightFinding> = found.iter().filter(|f| f.stats.as_ref().is_ok_and(|s| s.sampled > 0 && s.zeros == s.sampled && s.total > 64)).collect();
    let failed: Vec<&WeightFinding> = found.iter().filter(|f| f.stats.is_err()).collect();
    let values: u64 = found.iter().filter_map(|f| f.stats.as_ref().ok()).map(|s| s.sampled).sum();
    let mut pick = None;
    let mut list = |ui: &mut egui::Ui, fs: &[&WeightFinding]| {
        for f in fs.iter().take(12) {
            if ui.link(&d.tensors[f.tensor].name).clicked() {
                pick = Some(f.tensor);
            }
        }
        if fs.len() > 12 {
            ui.weak(format!("and {} more", fs.len() - 12));
        }
    };
    if broken.is_empty() && empty.is_empty() && failed.is_empty() {
        ui.colored_label(t.tone(Tone::Success), format!("✔ No broken numbers in {} values sampled from {} tensors.", group(values), group(found.len() as u64)));
    }
    if !broken.is_empty() {
        ui.colored_label(t.tone(Tone::Danger), format!("✖ {} tensor{} with broken values (NaN or infinite). The model will give wrong answers.", broken.len(), if broken.len() == 1 { "" } else { "s" }));
        list(ui, &broken);
    }
    if !empty.is_empty() {
        ui.colored_label(t.tone(Tone::Warn), format!("⚠ In {} tensor{}, every value sampled is zero, which usually means the data was damaged or left out.", empty.len(), if empty.len() == 1 { "" } else { "s" }));
        list(ui, &empty);
    }
    if !failed.is_empty() {
        ui.colored_label(t.tone(Tone::Warn), format!("⚠ {} tensor{} couldn't be read.", failed.len(), if failed.len() == 1 { "" } else { "s" }));
        list(ui, &failed);
    }
    if found.len() < readable.len() {
        ui.weak("Stopped before every tensor was read.");
    }
    if ui.small_button("Check again").clicked() {
        page.scanned = None;
    }
    if let Some(i) = pick {
        page.picked = Some(i);
        page.filter.clear();
        page.tab = PageTab::Tensors;
    }
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
    if let Some(t) = page.picked.filter(|&t| t < d.tensors.len()) {
        egui::Panel::right("tensor-detail").default_size(320.0).min_size(260.0).resizable(true).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| tensor_detail(ui, page, m, d, t));
        });
    }
    let mut clicked = None;
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
            if page.picked == Some(page.shown[row]) {
                ui.painter().rect_filled(rect, 3.0, vis.selection.bg_fill);
            } else if resp.hovered() {
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
            if resp.clicked() {
                clicked = Some(page.shown[row]);
            }
            resp.context_menu(|ui| {
                if ui.button("Copy name").clicked() {
                    ui.ctx().copy_text(t.name.clone());
                    ui.close();
                }
            });
        }
    });
    if let Some(t) = clicked {
        page.picked = if page.picked == Some(t) { None } else { Some(t) };
    }
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
/// Tokens: what the model's vocabulary is made of, its special tokens and chat template, and
/// every token, searchable.
fn page_tokens(ui: &mut egui::Ui, page: &mut ModelPage, m: &Model) {
    if page.vocab.is_none() {
        let (tx, rx) = mpsc::channel();
        let m = m.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::vocab::read(&m, inspect::publisher_dir(&m).as_deref()));
        });
        page.vocab = Some(VocabState::Reading(rx));
    }
    if let Some(VocabState::Reading(rx)) = &page.vocab {
        match rx.try_recv() {
            Ok(Ok(v)) => {
                let s = crate::vocab::summarise(&v);
                page.vocab = Some(VocabState::Read(Arc::new(v), Arc::new(s)));
            }
            Ok(Err(e)) => page.vocab = Some(VocabState::Failed(e)),
            Err(mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(Duration::from_millis(50)),
            Err(mpsc::TryRecvError::Disconnected) => page.vocab = Some(VocabState::Failed("The vocabulary couldn't be read.".into())),
        }
    }
    let (v, s) = match &page.vocab {
        Some(VocabState::Read(v, s)) => (v.clone(), s.clone()),
        Some(VocabState::Failed(e)) => {
            kit::callout(ui, Tone::Warn, e.clone());
            return;
        }
        _ => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading the vocabulary…");
            });
            return;
        }
    };
    egui::Panel::left("vocab-summary").default_size(430.0).min_size(320.0).resizable(true).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt(("vocab-summary", page.model)).auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(ui.available_width() - 12.0);
            vocab_summary(ui, page, &v, &s);
        });
    });
    vocab_list(ui, page, &v);
}

fn tokenizer_words(v: &crate::vocab::Vocab) -> String {
    let family = match v.model.as_deref() {
        Some("gpt2") | Some("BPE") => "byte-pair encoding (BPE)",
        Some("llama") | Some("Unigram") => "SentencePiece",
        Some("bert") | Some("WordPiece") => "WordPiece",
        Some("t5") => "SentencePiece (T5)",
        Some("rwkv") => "RWKV",
        Some(other) => other,
        None => "an unnamed tokenizer",
    };
    match v.spelling {
        crate::vocab::Spelling::ByteLevel => format!("{family}, byte level: it can spell any text"),
        _ => family.to_string(),
    }
}

fn vocab_summary(ui: &mut egui::Ui, page: &mut ModelPage, v: &crate::vocab::Vocab, s: &crate::vocab::Summary) {
    use crate::vocab::{Kind, Script};
    let t = kit::tokens(ui);
    egui::Grid::new("vocab-facts").num_columns(2).spacing([14.0, 5.0]).striped(true).show(ui, |ui| {
        ui.weak("Tokens");
        ui.label(group(v.tokens.len() as u64));
        ui.end_row();
        ui.weak("Tokenizer");
        ui.add(egui::Label::new(tokenizer_words(v)).wrap());
        ui.end_row();
        if let Some(p) = &v.pre {
            ui.weak("Text splitting");
            ui.label(p).on_hover_text("How text is split into words before it's tokenized (llama.cpp's name for it)");
            ui.end_row();
        }
        if let Some(n) = v.merges {
            ui.weak("Merge rules");
            ui.label(group(n)).on_hover_text("Pairs of pieces the tokenizer joins, most common first");
            ui.end_row();
        }
        ui.weak("Start new words");
        ui.label(format!("{} tokens ({:.0}%)", group(s.word_starts as u64), s.word_starts as f64 * 100.0 / v.tokens.len().max(1) as f64))
            .on_hover_text("Tokens that begin with a space, so they start a word");
        ui.end_row();
        for (k, n) in &s.kinds {
            if *k != Kind::Normal {
                ui.weak(match k {
                    Kind::Control => "Special tokens",
                    Kind::Added => "Added tokens",
                    Kind::Byte => "Byte tokens",
                    Kind::Unused => "Unused slots",
                    _ => "Unknown token",
                });
                ui.label(group(*n as u64));
                ui.end_row();
            }
        }
        if let Some(b) = v.add_bos {
            ui.weak("Adds a start token");
            ui.label(if b { "yes" } else { "no" });
            ui.end_row();
        }
        ui.weak("Read from");
        ui.label(v.from);
        ui.end_row();
    });
    if !v.special.is_empty() {
        ui.add_space(12.0);
        ui.strong("Special tokens");
        ui.weak("Markers the model reads and writes to begin and end text and turns.");
        ui.add_space(4.0);
        egui::Grid::new("vocab-special").num_columns(3).spacing([14.0, 4.0]).show(ui, |ui| {
            for (role, id) in &v.special {
                ui.weak(*role);
                ui.add(egui::Label::new(egui::RichText::new(&v.shown[*id as usize]).monospace().size(12.0)).selectable(true));
                ui.weak(format!("#{id}"));
                ui.end_row();
            }
        });
    }
    // Languages and kinds of text.
    ui.add_space(12.0);
    ui.strong("What the tokens are");
    ui.weak("Each token by its first letter. Click one to list only those.");
    ui.add_space(4.0);
    let total = v.tokens.len().max(1) as f32;
    let top = s.scripts.iter().map(|(_, n)| *n).max().unwrap_or(1) as f32;
    let label_w = 190.0;
    for (sc, n) in &s.scripts {
        let picked = page.vocab_script == Some(*sc);
        let w = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::click());
        if picked {
            ui.painter().rect_filled(rect, 3.0, ui.visuals().selection.bg_fill);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, 3.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let y = rect.center().y;
        ui.painter().with_clip_rect(rect).text(pos2(rect.left() + 4.0, y), Align2::LEFT_CENTER, sc.label(), FontId::proportional(12.5), ui.visuals().text_color());
        let bar_left = rect.left() + label_w;
        let bar_w = (rect.right() - 60.0 - bar_left).max(10.0);
        let colour = match sc {
            Script::Partial | Script::Special | Script::Spaces => ui.visuals().weak_text_color(),
            _ => t.accent,
        };
        let bar = Rect::from_min_size(pos2(bar_left, y - 5.0), vec2((bar_w * *n as f32 / top).max(1.5), 10.0));
        ui.painter().rect_filled(bar, 2.0, colour);
        ui.painter().text(pos2(rect.right() - 4.0, y), Align2::RIGHT_CENTER, format!("{:.1}%", *n as f32 * 100.0 / total), FontId::proportional(12.0), ui.visuals().weak_text_color());
        let resp = resp.on_hover_text(format!("{}: {} tokens", sc.label(), group(*n as u64)));
        if resp.clicked() {
            page.vocab_script = if picked { None } else { Some(*sc) };
        }
    }
    if s.scripts.iter().any(|(sc, _)| *sc == Script::Partial) {
        ui.add_space(2.0);
        ui.weak("Parts of characters are pieces of letters and emoji that take several bytes; the model joins them to spell anything else.");
    }
    // Lengths.
    ui.add_space(12.0);
    ui.strong("How long the tokens are");
    ui.add_space(4.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 90.0), Sense::hover());
    let painter = ui.painter_at(rect.expand(2.0));
    painter.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    let top = *s.lengths.iter().max().unwrap_or(&1).max(&1) as f32;
    let bw = rect.width() / s.lengths.len() as f32;
    for (i, &n) in s.lengths.iter().enumerate() {
        let h = (rect.height() - 4.0) * n as f32 / top;
        let r = Rect::from_min_max(pos2(rect.left() + i as f32 * bw + 1.0, rect.bottom() - h), pos2(rect.left() + (i + 1) as f32 * bw - 1.0, rect.bottom()));
        painter.rect_filled(r, 0.0, t.accent);
    }
    if let Some(p) = resp.hover_pos() {
        let i = (((p.x - rect.left()) / bw) as usize).min(s.lengths.len() - 1);
        let len = if i == 15 { "16 or more".to_string() } else { (i + 1).to_string() };
        resp.on_hover_text(format!("{len} characters: {} tokens", group(s.lengths[i] as u64)));
    }
    ui.horizontal(|ui| {
        ui.weak("1 character");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.weak("16+"));
    });
    if !s.longest.is_empty() {
        ui.add_space(6.0);
        ui.weak("Longest");
        ui.horizontal_wrapped(|ui| {
            for &i in &s.longest {
                ui.add(egui::Label::new(egui::RichText::new(&v.shown[i as usize]).monospace().size(12.0)).selectable(true)).on_hover_text(format!("#{i}"));
            }
        });
    }
    // Chat templates.
    for (name, tpl) in &v.templates {
        ui.add_space(12.0);
        ui.strong(if name.is_empty() { "Chat template".to_string() } else { format!("Chat template: {name}") });
        let (format, has) = crate::vocab::template_traits(tpl);
        let mut words = Vec::new();
        if let Some(f) = format {
            words.push(format!("Uses the {f} format."));
        }
        if !has.is_empty() {
            let mut list = has.join(", ");
            if let Some(i) = list.rfind(", ") {
                list.replace_range(i..i + 2, " and ");
            }
            words.push(format!("Mentions {list}."));
        }
        if !words.is_empty() {
            ui.label(words.join(" "));
        }
        ui.weak("How a conversation is laid out for the model. Apps fill it in before each reply.");
        egui::CollapsingHeader::new(format!("Show the template ({} lines)", tpl.lines().count())).id_salt(("tpl", name)).show(ui, |ui| {
            let mut text = tpl.as_str();
            ui.add(egui::TextEdit::multiline(&mut text).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(4));
        });
    }
}

/// Every token, filtered by the search and the script picked.
fn vocab_list(ui: &mut egui::Ui, page: &mut ModelPage, v: &crate::vocab::Vocab) {
    ui.horizontal(|ui| {
        ui.push_id("vocab-search", |ui| {
            ui.set_max_width(320.0);
            kit::search_field(ui, &mut page.vocab_filter, "Find a token, or #id");
        });
        if let Some(sc) = page.vocab_script {
            ui.add_space(6.0);
            if ui.button(format!("{}  ×", sc.label())).on_hover_text("Show every kind").clicked() {
                page.vocab_script = None;
            }
        }
    });
    let key = (page.vocab_filter.clone(), page.vocab_script);
    if page.vocab_shown_for.as_ref() != Some(&key) {
        let f = page.vocab_filter.trim();
        let id = f.strip_prefix('#').and_then(|n| n.parse::<u32>().ok());
        // Spaces typed in the search match the marks tokens are shown with.
        let f = crate::vocab::visible_query(f);
        let lower = f.to_lowercase();
        page.vocab_shown = (0..v.tokens.len() as u32)
            .filter(|&i| {
                let i_ = i as usize;
                page.vocab_script.is_none_or(|s| v.scripts[i_] == s)
                    && match id {
                        Some(n) => i == n,
                        None => lower.is_empty() || v.shown[i_].to_lowercase().contains(&lower),
                    }
            })
            .collect();
        page.vocab_shown_for = Some(key);
    }
    ui.weak(format!("{} of {} tokens", group(page.vocab_shown.len() as u64), group(v.tokens.len() as u64)));
    ui.add_space(4.0);
    let w = ui.available_width();
    ui.spacing_mut().item_spacing.y = 0.0;
    let mono = FontId::monospace(12.5);
    let small = FontId::proportional(12.0);
    egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_H, page.vocab_shown.len(), |ui, range| {
        for row in range {
            let i = page.vocab_shown[row] as usize;
            let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::click());
            let vis = ui.visuals();
            if resp.hovered() {
                ui.painter().rect_filled(rect, 3.0, vis.widgets.hovered.weak_bg_fill);
            } else if row % 2 == 1 {
                ui.painter().rect_filled(rect, 0.0, vis.faint_bg_color);
            }
            let y = rect.center().y;
            ui.painter().text(pos2(rect.left() + 64.0, y), Align2::RIGHT_CENTER, i.to_string(), small.clone(), vis.weak_text_color());
            let text_clip = Rect::from_min_max(pos2(rect.left() + 76.0, rect.top()), pos2(rect.right() - 150.0, rect.bottom())).intersect(ui.clip_rect());
            let colour = if v.kinds[i] == crate::vocab::Kind::Normal { vis.text_color() } else { kit::tokens(ui).accent };
            // Letters the app's fonts can't draw are shown by their code points instead of boxes.
            let drawable = v.kinds[i] != crate::vocab::Kind::Normal || v.scripts[i].drawable();
            let shown = if drawable { v.shown[i].clone() } else { v.shown[i].chars().map(|c| format!("U+{:04X}", c as u32)).collect::<Vec<_>>().join(" ") };
            if drawable {
                ui.painter().with_clip_rect(text_clip).text(pos2(rect.left() + 80.0, y), Align2::LEFT_CENTER, &shown, mono.clone(), colour);
            } else {
                ui.painter().with_clip_rect(text_clip).text(pos2(rect.left() + 80.0, y), Align2::LEFT_CENTER, &shown, small.clone(), vis.weak_text_color());
            }
            let what = if v.kinds[i] == crate::vocab::Kind::Normal { v.scripts[i].short() } else { v.kinds[i].label() };
            ui.painter().text(pos2(rect.right() - 8.0, y), Align2::RIGHT_CENTER, what, small.clone(), vis.weak_text_color());
            let raw = if v.tokens[i] != v.shown[i] { format!("\nIn the file: {}", v.tokens[i]) } else { String::new() };
            let note = if drawable { "" } else { "\nThe app's fonts can't show these letters, so their code points are shown. Copy the token to see it elsewhere." };
            let resp = resp.on_hover_text(format!("#{i}  {shown}{raw}\n{}{note}", v.kinds[i].label()));
            resp.context_menu(|ui| {
                if ui.button("Copy token").clicked() {
                    ui.ctx().copy_text(v.text(i));
                    ui.close();
                }
                if ui.button("Copy id").clicked() {
                    ui.ctx().copy_text(i.to_string());
                    ui.close();
                }
            });
        }
    });
}

/// Compare: this model and another side by side.
fn page_compare(ui: &mut egui::Ui, page: &mut ModelPage, m: &Model, o: &Model, a: &Inspection) {
    use crate::modeldiff::Change;
    let Some(cmp) = page.comparison.clone() else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(format!("Reading {}…", o.name));
        });
        return;
    };
    let (b, c) = (&cmp.0, &cmp.1);
    let mut stop = false;
    egui::ScrollArea::vertical().id_salt("compare").auto_shrink(false).show(ui, |ui| {
        if let Some(why) = &b.unread {
            kit::callout(ui, Tone::Warn, format!("{} can't be compared: {why}", o.name));
            return;
        }
        let t = kit::tokens(ui);
        kit::callout(
            ui,
            match c.verdict {
                crate::modeldiff::Verdict::Different | crate::modeldiff::Verdict::NoTensors => Tone::Neutral,
                _ => Tone::Accent,
            },
            c.verdict.explain(),
        );
        ui.add_space(4.0);
        ui.weak("Compared from what the files say about themselves. The weights aren't compared.");
        ui.add_space(8.0);
        let first = egui::RichText::new(format!("1  {}", m.name)).strong();
        let second = egui::RichText::new(format!("2  {}", o.name)).strong();
        egui::Grid::new("compare-facts").num_columns(3).spacing([18.0, 5.0]).striped(true).show(ui, |ui| {
            ui.label("");
            ui.label(first.clone());
            ui.label(second.clone());
            ui.end_row();
            ui.weak("Size");
            ui.label(scan::human(m.size));
            ui.label(scan::human(o.size));
            ui.end_row();
            for r in &c.facts {
                ui.weak(r.label);
                let differs = r.differs();
                for v in [&r.a, &r.b] {
                    match v {
                        Some(v) if differs => ui.label(egui::RichText::new(v).color(t.accent)),
                        Some(v) => ui.label(v),
                        None => ui.weak("—"),
                    };
                }
                ui.end_row();
            }
        });
        let tensors = !a.tensors.is_empty() && !b.tensors.is_empty();
        // Where the space goes.
        if tensors {
            ui.add_space(14.0);
            ui.strong("Where the space goes");
            ui.add_space(4.0);
            let top = c.parts.iter().map(|(x, y)| (*x).max(*y)).max().unwrap_or(1).max(1) as f32;
            egui::Grid::new("compare-parts").num_columns(4).spacing([14.0, 4.0]).show(ui, |ui| {
                for (i, p) in inspect::Part::ALL.iter().enumerate() {
                    let (x, y) = c.parts[i];
                    if x == 0 && y == 0 {
                        continue;
                    }
                    swatch(ui, part_colour(*p), p.label());
                    for (v, n) in [(x, 1), (y, 2)] {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(140.0, 10.0), Sense::hover());
                            ui.painter().rect_filled(r, 2.0, ui.visuals().extreme_bg_color);
                            let w = r.width() * v as f32 / top;
                            ui.painter().rect_filled(Rect::from_min_size(r.min, vec2(w.max(if v > 0 { 1.5 } else { 0.0 }), r.height())), 2.0, part_colour(*p));
                            ui.label(egui::RichText::new(scan::human(v)).size(12.5)).on_hover_text(format!("In model {n}"));
                        });
                    }
                    ui.label("");
                    ui.end_row();
                }
            });
            // Number types.
            ui.add_space(14.0);
            ui.strong("How the numbers are stored");
            ui.add_space(4.0);
            let mut kinds: Vec<&str> = a.kinds.iter().map(|k| k.kind.as_str()).collect();
            for k in &b.kinds {
                if !kinds.contains(&k.kind.as_str()) {
                    kinds.push(&k.kind);
                }
            }
            egui::Grid::new("compare-kinds").num_columns(3).spacing([18.0, 4.0]).striped(true).show(ui, |ui| {
                ui.label("");
                ui.label(first.clone());
                ui.label(second.clone());
                ui.end_row();
                for k in kinds {
                    ui.label(k);
                    for d in [a, b] {
                        match d.kinds.iter().find(|x| x.kind == k) {
                            Some(x) => ui.label(format!("{} · {} tensors", scan::human(x.bytes), group(x.tensors as u64))),
                            None => ui.weak("—"),
                        };
                    }
                    ui.end_row();
                }
            });
        }
        // Metadata.
        if !c.meta.is_empty() {
            ui.add_space(14.0);
            egui::CollapsingHeader::new(egui::RichText::new(format!("Metadata that differs ({})", c.meta.len())).strong()).id_salt("compare-meta").show(ui, |ui| {
                egui::Grid::new("compare-meta-grid").num_columns(3).spacing([14.0, 4.0]).striped(true).show(ui, |ui| {
                    for (k, x, y) in &c.meta {
                        ui.add(egui::Label::new(egui::RichText::new(k).monospace().size(12.0)).selectable(true));
                        for v in [x, y] {
                            match v {
                                Some(v) => {
                                    let short: String = v.chars().take(80).collect();
                                    ui.add(egui::Label::new(short).truncate()).on_hover_text(v.chars().take(600).collect::<String>());
                                }
                                None => {
                                    ui.weak("—");
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
            });
        }
        if !tensors {
            ui.add_space(10.0);
            stop = ui.button("Stop comparing").clicked();
            return;
        }
        // Tensors.
        ui.add_space(14.0);
        ui.strong("Tensors");
        ui.weak("Matched by name. Hugging Face names are translated to GGUF's, so a GGUF download can be compared with its original.");
        ui.add_space(4.0);
        let changed = c.kind + c.shape;
        ui.weak(format!("{} the same in both, {} changed.", group(c.same as u64), group(changed as u64)));
        ui.add_space(4.0);
        let labels = [
            format!("Changed ({})", group(changed as u64)),
            format!("Only in 1 ({})", group(c.only_a as u64)),
            format!("Only in 2 ({})", group(c.only_b as u64)),
            format!("All ({})", group(c.tensors.len() as u64)),
        ];
        kit::segmented(
            ui,
            &mut page.compare_show,
            &[
                (CompareShow::Changed, labels[0].as_str()),
                (CompareShow::OnlyA, labels[1].as_str()),
                (CompareShow::OnlyB, labels[2].as_str()),
                (CompareShow::All, labels[3].as_str()),
            ],
            false,
        );
        ui.add_space(4.0);
        let rows: Vec<&crate::modeldiff::TensorRow> = c
            .tensors
            .iter()
            .filter(|r| match page.compare_show {
                CompareShow::Changed => matches!(r.change, Change::Kind | Change::Shape),
                CompareShow::OnlyA => r.change == Change::OnlyA,
                CompareShow::OnlyB => r.change == Change::OnlyB,
                CompareShow::All => true,
            })
            .collect();
        if rows.is_empty() {
            ui.weak("None.");
        }
        let side = |d: &Inspection, i: Option<usize>| i.map(|i| &d.tensors[i]).map(|t| format!("{} · {} · {}", t.kind, t.shape_text(), scan::human(t.bytes))).unwrap_or_else(|| "—".into());
        ui.spacing_mut().item_spacing.y = 0.0;
        let w = ui.available_width();
        egui::ScrollArea::vertical().id_salt("compare-tensors").max_height(420.0).min_scrolled_height((rows.len() as f32 * ROW_H).min(420.0)).auto_shrink([false, true]).show_rows(ui, ROW_H, rows.len(), |ui, range| {
            for k in range {
                let r = rows[k];
                let (rect, resp) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::hover());
                let vis = ui.visuals();
                if k % 2 == 1 {
                    ui.painter().rect_filled(rect, 0.0, vis.faint_bg_color);
                }
                let y = rect.center().y;
                let col = (w - 16.0) / 3.0;
                let clip = |x0: f32, x1: f32| Rect::from_min_max(pos2(x0, rect.top()), pos2(x1, rect.bottom())).intersect(ui.clip_rect());
                let mono = FontId::monospace(12.0);
                let plain = FontId::proportional(12.5);
                ui.painter().with_clip_rect(clip(rect.left(), rect.left() + col)).text(pos2(rect.left() + 8.0, y), Align2::LEFT_CENTER, &r.name, mono, vis.text_color());
                let colour = |changed: bool| if changed { kit::tokens(ui).accent } else { vis.text_color() };
                let ch = r.change != Change::Same;
                ui.painter().with_clip_rect(clip(rect.left() + col, rect.left() + 2.0 * col)).text(pos2(rect.left() + col + 8.0, y), Align2::LEFT_CENTER, side(a, r.a), plain.clone(), colour(ch && r.a.is_some()));
                ui.painter().with_clip_rect(clip(rect.left() + 2.0 * col, rect.right())).text(pos2(rect.left() + 2.0 * col + 8.0, y), Align2::LEFT_CENTER, side(b, r.b), plain, colour(ch && r.b.is_some()));
                let names = match (r.a, r.b) {
                    (Some(i), Some(j)) if a.tensors[i].name != b.tensors[j].name => format!("\n1: {}\n2: {}", a.tensors[i].name, b.tensors[j].name),
                    _ => String::new(),
                };
                resp.on_hover_text(format!("{} ({}){names}", r.name, r.change.label()));
            }
        });
        ui.add_space(10.0);
        stop = ui.button("Stop comparing").clicked();
    });
    if stop {
        page.compare_with = None;
        page.compare_rx = None;
        page.comparison = None;
        page.tab = PageTab::Overview;
    }
}

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
        // A folder moved to another drive (Storage) is a link now: look where it leads, keeping
        // the paths through the link, as the apps see them.
        let real = if crate::relocate::is_link(&root) { std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone()) } else { root.clone() };
        let tree = scan::scan(&real, &opts, &sp, &scan::new_live(&real));
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
