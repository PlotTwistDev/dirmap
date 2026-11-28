//! Shrinking (quantising) one model into a new, smaller file (see `crate::quantize`). The
//! original is never changed. Model quantisation is Professional, checked when it's opened.

use super::*;
use crate::quantize::{self, Checked, Run, Target};
use std::path::Path;

pub(in crate::app) struct Shrink {
    name: String,
    input: PathBuf,
    quant: Option<String>,
    /// For a file named by its hash: the model's name, for the new file's.
    hashed: Option<String>,
    size: u64,
    target: Target,
    /// The folder the new file goes in.
    folder: String,
    run: Option<Run>,
    /// The new file, checked, and how it compares with the original.
    done: Option<(PathBuf, Checked)>,
}

impl Shrink {
    fn out(&self) -> PathBuf {
        Path::new(self.folder.trim()).join(quantize::output_name(&self.input, self.hashed.as_deref(), self.quant.as_deref(), self.target))
    }

    /// Why it can't start, if it can't.
    fn problem(&self) -> Option<String> {
        let folder = Path::new(self.folder.trim());
        if !folder.is_dir() {
            return Some("Choose a folder for the new file.".into());
        }
        let out = self.out();
        if std::fs::symlink_metadata(&out).is_ok() {
            return Some(format!("{} already exists.", out.shown()));
        }
        let need = quantize::estimate(self.size, self.quant.as_deref(), self.target);
        if let Some(free) = crate::drives::free_at(folder)
            && free < need + (256 << 20)
        {
            return Some(format!("That drive has {} free; the new file needs about {}.", scan::human(free), scan::human(need)));
        }
        None
    }
}

/// Where the new file goes by default: beside the original, unless the original is inside an
/// app's own store (where a new file would be lost or confuse the app); then Filescape's
/// models folder.
fn default_folder(m: &Model) -> PathBuf {
    let beside = m.files.first().and_then(|f| f.path.parent()).map(Path::to_path_buf);
    match m.source {
        Source::Ollama | Source::HuggingFace => crate::getmodel::folder().filter(|f| std::fs::create_dir_all(f).is_ok()).or(beside),
        _ => beside,
    }
    .unwrap_or_default()
}

impl App {
    pub(in crate::app) fn ask_shrink(&mut self, m: &Model) {
        let Some(f) = m.files.first() else { return };
        if !self.premium(crate::licence::Feature::Quantize) {
            return;
        }
        let current = m.info.quant.as_deref().and_then(quantize::bits_of).unwrap_or(16.0);
        // The usual choice, or the biggest that still shrinks it.
        let target = quantize::TARGETS.iter().copied().find(|t| t.name == "Q4_K_M" && t.bits < current).or_else(|| quantize::TARGETS.iter().copied().find(|t| t.bits < current));
        let Some(target) = target else {
            self.models.storage.outcome = Some((Tone::Neutral, format!("{} is already as small as Filescape can make it.", m.name)));
            return;
        };
        self.models.storage.shrink = Some(Shrink {
            name: m.name.clone(),
            input: f.path.clone(),
            quant: m.info.quant.clone(),
            hashed: matches!(m.source, Source::Ollama | Source::HuggingFace).then(|| m.name.clone()),
            size: m.size,
            target,
            folder: default_folder(m).to_string_lossy().to_string(),
            run: None,
            done: None,
        });
    }

    pub(in crate::app) fn storage_shrink(&mut self, ctx: &egui::Context) {
        let Some(s) = &mut self.models.storage.shrink else { return };
        // Finished?
        if let Some(r) = &s.run {
            let result = r.result.lock().unwrap().take();
            if let Some(result) = result {
                let out = r.out.clone();
                s.run = None;
                match result {
                    Ok(c) => s.done = Some((out, c)),
                    Err(e) => {
                        self.models.storage.outcome = Some((Tone::Warn, e));
                        self.models.storage.shrink = None;
                        return;
                    }
                }
                // The new file joins the list while the result is on screen.
                self.start_models();
                return;
            }
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        if s.done.is_some() {
            self.shrink_result(ctx);
            return;
        }
        let mut go = false;
        let mut pick = false;
        let mut close = false;
        egui::Modal::new(egui::Id::new("storage-shrink")).show(ctx, |ui| {
            ui.set_max_width(560.0);
            ui.heading("Optimise model size");
            ui.weak(format!("Make a smaller copy of {}. The original is kept.", s.name));
            ui.add_space(4.0);
            if let Some(r) = &s.run {
                let (done, total) = (r.done.load(Ordering::Relaxed), r.total.load(Ordering::Relaxed));
                let text = if total == 0 { "Reading the model…".to_string() } else { format!("Shrinking… part {done} of {total}") };
                ui.add(egui::ProgressBar::new(if total == 0 { 0.0 } else { done as f32 / total as f32 }).desired_width(520.0).text(text));
                ui.add_space(4.0);
                ui.weak(format!("Writing {}. The original is only read.", r.out.shown()));
                ui.add_space(8.0);
                if ui.button("■ Stop").clicked() {
                    r.stop.store(true, Ordering::Relaxed);
                }
                return;
            }
            let now = s.quant.clone().unwrap_or_else(|| "unknown precision".into());
            ui.label(format!(
                "Makes a new, smaller file with less precise weights: it loads faster and needs less memory, and writes slightly worse. The original ({now}, {}) is kept as it is.",
                scan::human(s.size)
            ));
            if quantize::already_shrunk(s.quant.as_deref()) {
                ui.add_space(4.0);
                kit::callout(ui, Tone::Warn, "This model is already shrunk. Shrinking it again loses more than shrinking the original would; use the original if you have it.");
            }
            ui.add_space(6.0);
            let current = s.quant.as_deref().and_then(quantize::bits_of).unwrap_or(16.0);
            egui::Grid::new("shrink-targets").num_columns(3).spacing([12.0, 3.0]).show(ui, |ui| {
                for t in quantize::TARGETS {
                    let smaller = t.bits < current;
                    ui.add_enabled_ui(smaller, |ui| ui.radio_value(&mut s.target, t, t.name));
                    ui.label(format!("about {}", scan::human(quantize::estimate(s.size, s.quant.as_deref(), t))));
                    ui.weak(if smaller { t.about } else { "Not smaller than it is now" });
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Into");
                ui.add(egui::TextEdit::singleline(&mut s.folder).desired_width(380.0));
                pick = ui.button("Choose…").clicked();
            });
            ui.weak(format!("New file: {}", s.out().shown()));
            let problem = s.problem();
            if let Some(p) = &problem {
                ui.add_space(4.0);
                kit::callout(ui, Tone::Warn, p.clone());
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                go = ui.add_enabled_ui(problem.is_none(), |ui| kit::button(ui, kit::Kind::Primary, None, &format!("Shrink to {}", s.target.name))).inner.clicked();
                close = ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if pick {
            let d = super::super::dialogs::Dialog::new().set_title("Folder for the smaller model");
            self.ask_path(d, super::super::dialogs::Pick::Folder, |app, p| {
                if let Some(s) = &mut app.models.storage.shrink {
                    s.folder = p.to_string_lossy().to_string();
                }
            });
        } else if close {
            self.models.storage.shrink = None;
        } else if go {
            let threads = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4);
            match quantize::start(&s.input, s.size, &s.out(), s.target, threads) {
                Ok(r) => s.run = Some(r),
                Err(e) => {
                    self.models.storage.outcome = Some((Tone::Warn, e));
                    self.models.storage.shrink = None;
                }
            }
        }
    }

    /// The new file's check and the original beside it.
    fn shrink_result(&mut self, ctx: &egui::Context) {
        let Some(s) = &self.models.storage.shrink else { return };
        let Some((out, c)) = &s.done else { return };
        let (out, input) = (out.clone(), s.input.clone());
        let (mut close, mut show, mut compare) = (false, false, false);
        egui::Modal::new(egui::Id::new("storage-shrink")).show(ctx, |ui| {
            ui.set_max_width(560.0);
            ui.heading("Optimise model size");
            kit::callout(ui, Tone::Success, format!("Done, and checked: {}", out.shown()));
            ui.add_space(6.0);
            let precision = |p: &Option<String>| p.clone().unwrap_or_else(|| "not stated".into());
            let bits = |b: Option<f64>| b.map_or("—".to_string(), |b| format!("{b:.2}"));
            let count = |n: usize| if n == 0 { "—".to_string() } else { crate::inspect::group(n as u64) };
            let params = |n: u64| if n == 0 { "—".to_string() } else { crate::inspect::group(n) };
            egui::Grid::new("shrink-result").num_columns(3).spacing([18.0, 4.0]).striped(true).show(ui, |ui| {
                ui.strong("");
                ui.strong("Original");
                ui.strong("New file");
                ui.end_row();
                ui.label("Size on disk");
                ui.label(scan::human(c.before.bytes));
                ui.label(scan::human(c.after.bytes));
                ui.end_row();
                ui.label("Precision");
                ui.label(precision(&c.before.precision));
                ui.label(precision(&c.after.precision));
                ui.end_row();
                ui.label("Bits per weight").on_hover_text("The file's size in bits over its weights: measured from the files, not estimated.");
                ui.label(bits(c.before.bits()));
                ui.label(bits(c.after.bits()));
                ui.end_row();
                ui.label("Weights");
                ui.label(params(c.before.params));
                ui.label(params(c.after.params));
                ui.end_row();
                ui.label("Tensors");
                ui.label(count(c.before.tensors));
                ui.label(count(c.after.tensors));
                ui.end_row();
            });
            ui.add_space(4.0);
            if c.before.bytes > c.after.bytes {
                let saved = c.before.bytes - c.after.bytes;
                ui.label(format!("{} smaller ({:.0}% of the original).", scan::human(saved), c.after.bytes as f64 * 100.0 / c.before.bytes.max(1) as f64));
            }
            if c.changed > 0 {
                ui.weak(format!("{} of {} tensors now use fewer bits; the rest, such as normalisation weights, stay precise on purpose.", c.changed, c.after.tensors));
            }
            ui.add_space(6.0);
            ui.strong("Checked");
            let ok = kit::tokens(ui).success;
            for line in &c.checks {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                    crate::kit::icons::paint(ui.painter(), r, crate::kit::icons::Icon::Check, ok);
                    ui.label(line);
                });
            }
            ui.weak("These checks say the file is complete and well formed. How well it writes is worth trying in Ask AI or the Lab's Compare page.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                compare = ui
                    .add_enabled(self.models.list.is_some(), egui::Button::new("Compare with the original"))
                    .on_hover_text("Open both side by side in the model's page: every tensor, before and after")
                    .clicked();
                show = ui.button("Show in Files").clicked();
                close = kit::button(ui, kit::Kind::Primary, None, "Done").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if compare {
            self.models.storage.shrink = None;
            self.compare_files(&input, &out);
        } else if show {
            self.models.storage.shrink = None;
            self.show_model_file(&out);
        } else if close {
            self.models.storage.shrink = None;
        }
    }
}
