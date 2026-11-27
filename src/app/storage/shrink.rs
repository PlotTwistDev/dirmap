//! Shrinking (quantising) one model into a new, smaller file (see `crate::quantize`). The
//! original is never changed. Model quantisation is Professional, checked when it's opened.

use super::*;
use crate::quantize::{self, Run, Target};
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
        });
    }

    pub(in crate::app) fn storage_shrink(&mut self, ctx: &egui::Context) {
        let Some(s) = &mut self.models.storage.shrink else { return };
        // Finished?
        if let Some(r) = &s.run {
            let result = r.result.lock().unwrap().take();
            if let Some(result) = result {
                let out = r.out.clone();
                self.models.storage.outcome = Some(match result {
                    Ok(()) => {
                        let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
                        (Tone::Success, format!("Done: {} ({}). The original is unchanged.", out.shown(), scan::human(size)))
                    }
                    Err(e) => (Tone::Warn, e),
                });
                self.models.storage.shrink = None;
                self.start_models();
                return;
            }
            ctx.request_repaint_after(Duration::from_millis(200));
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
            match quantize::start(&s.input, &s.out(), s.target, threads) {
                Ok(r) => s.run = Some(r),
                Err(e) => {
                    self.models.storage.outcome = Some((Tone::Warn, e));
                    self.models.storage.shrink = None;
                }
            }
        }
    }
}
