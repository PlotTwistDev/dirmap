//! The AI Models workspace (Ctrl/⌘+4): the language and image models on this computer, found
//! where Ollama, LM Studio and Hugging Face keep them, in Downloads, in folders the user adds and
//! in the Storage Analysis scan. Grouped by model, with copies and unused Ollama downloads
//! marked (see `crate::models`).

use super::files::Workspace;
use super::*;
use crate::fsys::PathShow;
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
        self.models = ModelsState { job: Some(job), list: None, chosen: None, places };
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
        if chosen.is_some() {
            let frame = egui::Frame::new().fill(pal.chrome).inner_margin(egui::Margin::same(10));
            let r = egui::Panel::right("model-details").default_size(340.0).min_size(240.0).resizable(true).frame(frame).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.model_details(ui));
            });
            self.help_regions.push((r.response.rect, topics::AI_MODELS));
        }
        let frame = egui::Frame::new().fill(pal.chrome).inner_margin(egui::Margin::same(10));
        let r = egui::CentralPanel::default().frame(frame).show(ui, |ui| self.models_main(ui));
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
                    open = Some(i);
                }
                resp.context_menu(|ui| {
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
        if ui.button("Show in Files").clicked() {
            open = m.files.first().map(|f| f.path.clone());
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
