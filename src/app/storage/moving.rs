//! Storage: where each app keeps its models, and moving one app's whole folder to another drive
//! (see `crate::relocate`). The app keeps finding its models through the link left behind.
//!
//! Moving a whole store is part of managing models in bulk (Professional), checked when it starts.

use super::*;
use crate::relocate::{self, Moved, Plan};
use crate::transfer::Ctl;
use std::path::Path;
use std::sync::Arc;

/// The apps whose folders can move. Downloads and the user's own folders are theirs to move.
const APPS: [&str; 3] = ["Ollama", "LM Studio", "Hugging Face"];

pub(in crate::app) struct Moving {
    app: &'static str,
    from: PathBuf,
    /// The folder the new one goes in, as typed or picked.
    dest: String,
    checking: Option<mpsc::Receiver<Result<Plan, String>>>,
    checked: Option<Result<Plan, String>>,
    running: Option<(Arc<Ctl>, mpsc::Receiver<Result<Moved, String>>)>,
}

impl App {
    /// The apps' folders, under the bars: where each is, and a way to move it.
    pub(super) fn storage_places(&mut self, ui: &mut egui::Ui, list: &[Model]) {
        let places: Vec<(&'static str, PathBuf)> = self.models.places.iter().filter_map(|(l, p)| APPS.iter().find(|a| **a == l.as_str()).map(|a| (*a, p.clone()))).collect();
        if places.is_empty() {
            return;
        }
        ui.add_space(8.0);
        ui.strong("Where they're kept");
        let busy = self.models.storage.removing.is_some() || self.models.storage.moving.is_some();
        let mut start = None;
        egui::Grid::new("storage-places").num_columns(3).spacing([12.0, 4.0]).show(ui, |ui| {
            for (app, p) in &places {
                ui.label(*app);
                let bytes: u64 = list.iter().filter(|m| m.files.first().is_some_and(|f| f.path.starts_with(p))).map(|m| m.size).sum();
                match std::fs::read_link(p).ok().filter(|_| relocate::is_link(p)) {
                    Some(to) => {
                        ui.add(egui::Label::new(egui::RichText::new(format!("{}  →  {}", p.shown(), to.shown())).small().monospace()));
                        ui.weak("Moved: a link leads to the new place");
                    }
                    None => {
                        ui.add(egui::Label::new(egui::RichText::new(format!("{} · {}", p.shown(), scan::human(bytes))).small().monospace()));
                        let r = ui.add_enabled(!busy, egui::Button::new("Move to another drive…")).on_hover_text(format!(
                            "Copy {app}'s models to another drive and leave a link here, so {app} keeps finding them. Professional."
                        ));
                        if r.clicked() {
                            start = Some((*app, p.clone()));
                        }
                    }
                }
                ui.end_row();
            }
        });
        if let Some((app, from)) = start
            && self.premium(crate::licence::Feature::ModelAutomation)
        {
            self.models.storage.moving = Some(Moving { app, from, dest: String::new(), checking: None, checked: None, running: None });
        }
    }

    pub(super) fn poll_moving(&mut self, ctx: &egui::Context) {
        let Some(m) = &mut self.models.storage.moving else { return };
        if let Some(rx) = &m.checking {
            match rx.try_recv() {
                Ok(r) => {
                    m.checked = Some(r);
                    m.checking = None;
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => m.checking = None,
            }
        }
        if let Some((_, rx)) = &m.running {
            match rx.try_recv() {
                Ok(r) => {
                    let (app, to) = (m.app, m.checked.as_ref().and_then(|c| c.as_ref().ok()).map(|p| p.to.clone()).unwrap_or_default());
                    self.models.storage.moving = None;
                    self.models.storage.outcome = Some(match r {
                        Ok(Moved { left: None }) => (Tone::Success, format!("Done: {app}'s models are now in {}, with a link where they were.", to.shown())),
                        Ok(Moved { left: Some((old, why)) }) => (
                            Tone::Warn,
                            format!("{app}'s models are now in {}, with a link where they were. The original folder couldn't be deleted ({why}); it's at {} and can go.", to.shown(), old.shown()),
                        ),
                        Err(e) => (Tone::Warn, e),
                    });
                    self.start_models();
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(200)),
                Err(mpsc::TryRecvError::Disconnected) => self.models.storage.moving = None,
            }
        }
    }

    /// The move's window: choose where, see what happens, and follow it.
    pub(in crate::app) fn storage_moving(&mut self, ctx: &egui::Context) {
        let Some(m) = &mut self.models.storage.moving else { return };
        let mut check = false;
        let mut pick = false;
        let mut go = false;
        let mut close = false;
        egui::Modal::new(egui::Id::new("storage-move")).show(ctx, |ui| {
            ui.set_max_width(560.0);
            let app = m.app;
            ui.heading(format!("Move {app}'s models to another drive"));
            ui.add_space(4.0);
            if let Some((ctl, _)) = &m.running {
                let (done, total) = (ctl.bytes_done.load(Ordering::Relaxed), ctl.bytes_total.load(Ordering::Relaxed).max(1));
                let frac = done as f32 / total as f32;
                let text = if frac >= 1.0 { "Checking the copy and making the link…".to_string() } else { format!("Copying… {} of {}", scan::human(done), scan::human(total)) };
                ui.add(egui::ProgressBar::new(frac.min(1.0)).desired_width(520.0).text(text));
                ui.add_space(4.0);
                ui.weak("The originals stay until the copy has been checked. Stopping puts everything back.");
                ui.add_space(8.0);
                if ui.button("■ Stop").clicked() {
                    ctl.stop.store(true, Ordering::Relaxed);
                }
                return;
            }
            ui.label(format!("{app} keeps them in {}. They're copied to a new folder on the drive you choose, checked, and a link is left here so {app} finds them as before. The originals are deleted only after that.", m.from.shown()));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Into");
                let r = ui.add(egui::TextEdit::singleline(&mut m.dest).desired_width(360.0).hint_text("A folder on another drive"));
                if r.changed() {
                    m.checked = None;
                }
                check |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                pick = ui.button("Choose…").clicked();
            });
            let dest = m.dest.trim();
            if !dest.is_empty() {
                ui.weak(format!("New folder: {}", Path::new(dest).join(relocate::folder_name(app)).shown()));
            }
            ui.add_space(6.0);
            match &m.checked {
                Some(Ok(p)) => {
                    kit::callout(ui, Tone::Neutral, format!("{} will be copied to {}.", scan::human(p.bytes), p.to.shown()));
                    match app {
                        "Ollama" => {}
                        "LM Studio" => {
                            ui.weak("Quit LM Studio first, so no model is in use while it moves.");
                        }
                        _ => {
                            ui.weak("Close programs using these models first, so none is in use while they move.");
                        }
                    }
                }
                Some(Err(e)) => kit::callout(ui, Tone::Warn, e.clone()),
                None if m.checking.is_some() => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Checking…");
                    });
                }
                None => {}
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                match &m.checked {
                    Some(Ok(_)) => go = kit::button(ui, kit::Kind::Primary, None, "Move").clicked(),
                    _ => check |= ui.add_enabled(!dest.is_empty() && m.checking.is_none(), egui::Button::new("Check")).clicked(),
                }
                close = ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if pick {
            let d = super::super::dialogs::Dialog::new().set_title("Folder on another drive");
            self.ask_path(d, super::super::dialogs::Pick::Folder, |app, p| {
                if let Some(m) = &mut app.models.storage.moving {
                    m.dest = p.to_string_lossy().to_string();
                    m.checked = None;
                    app.storage_move_check();
                }
            });
            return;
        }
        if close {
            self.models.storage.moving = None;
        } else if check {
            self.storage_move_check();
        } else if go && let Some(m) = &mut self.models.storage.moving
            && let Some(Ok(plan)) = m.checked.clone()
        {
            let ctl = Arc::new(Ctl::default());
            let (tx, rx) = mpsc::channel();
            let c = ctl.clone();
            std::thread::spawn(move || {
                let _ = tx.send(relocate::run(&plan, &c));
            });
            m.running = Some((ctl, rx));
        }
    }

    fn storage_move_check(&mut self) {
        let Some(m) = &mut self.models.storage.moving else { return };
        let (app, from, dest) = (m.app, m.from.clone(), PathBuf::from(m.dest.trim()));
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(relocate::check(app, &from, &dest));
        });
        m.checking = Some(rx);
        m.checked = None;
    }
}
