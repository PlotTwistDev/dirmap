//! Eject for external drives, removable media and disk images: from a drive's right-click menu
//! and the ⏏ button beside it in the sidebar and the Computer view.
//!
//! Before the system is asked, Filescape lets go of the drive itself: tabs on it move to the
//! Computer view, folder watching and live analysis stop, and a scan of it is stopped. When an
//! app still has files open on it, a window says which (where the system tells), with Try Again
//! and a Force Eject that asks twice.

use super::*;
use crate::drives::Drive;
use crate::eject::{self, Failure, Plan};
use crate::fsys::PathShow;
use crate::kit;
use std::path::Path;

#[derive(Default)]
pub(super) struct EjectState {
    /// Roots being ejected now (the drive, its images, the other volumes on its disk).
    busy: Vec<PathBuf>,
    jobs: Vec<EjectJob>,
    ask: Option<Ask>,
    /// Roots ejected this session that haven't come back, so nothing tries to watch them.
    gone: Vec<PathBuf>,
}

struct EjectJob {
    root: PathBuf,
    name: String,
    roots: Vec<PathBuf>,
    image: bool,
    rx: mpsc::Receiver<Result<(), Failure>>,
}

enum Ask {
    /// What else ejecting does, before it's done.
    Confirm { root: PathBuf, name: String, notes: Vec<String> },
    /// An app has files open on it.
    Busy { root: PathBuf, name: String, failure: Failure, force_armed: bool },
    /// It didn't work, or can't be done now.
    Failed { name: String, message: String },
}

/// Every root that goes when `p` is ejected.
fn roots(p: &Plan) -> Vec<PathBuf> {
    std::iter::once(&p.drive).chain(&p.siblings).chain(&p.images).map(|d| d.root.clone()).collect()
}

fn list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl App {
    /// The drive at `root`, if Eject applies to it.
    pub(super) fn ejectable(&self, root: &Path) -> Option<&Drive> {
        self.drives.iter().find(|d| d.root == root).filter(|d| eject::ejectable(d))
    }

    /// Being ejected now, or ejected and not plugged in again.
    pub(super) fn ejecting(&self, path: &Path) -> bool {
        self.eject.busy.iter().chain(&self.eject.gone).any(|r| path.starts_with(r))
    }

    /// A drive's root is being ejected right now (for a spinner in place of ⏏).
    pub(super) fn ejecting_now(&self, root: &Path) -> bool {
        self.eject.busy.iter().any(|r| r == root)
    }

    /// Eject the drive at `root`: straight away, or after saying what else it does.
    pub(super) fn ask_eject(&mut self, root: &Path) {
        let Some(d) = self.ejectable(root).cloned() else { return };
        if self.ejecting_now(root) {
            return;
        }
        let name = eject::name_of(&d);
        if let Some(why) = eject::protected(&d) {
            self.eject.ask = Some(Ask::Failed { name, message: why.into() });
            return;
        }
        let plan = eject::plan(&d, &self.drives);
        let going = roots(&plan);
        let on = |p: &Path| going.iter().any(|r| p.starts_with(r));
        // A copy to or from it must finish (or be stopped) first: ejecting would break it.
        if let Some(j) = self.files.rws.queue.jobs.iter().find(|j| !j.finished() && (on(&j.dest) || j.sources.iter().any(|s| on(s)))) {
            self.eject.ask = Some(Ask::Failed {
                name,
                message: format!("A transfer still uses it: {}. Wait for it to finish, or stop it under Transfers, then eject.", j.label()),
            });
            return;
        }
        let mut notes = Vec::new();
        if !plan.images.is_empty() {
            let names: Vec<String> = plan.images.iter().map(eject::name_of).collect();
            notes.push(format!("{} {} stored on it, so {} ejected first.", list(&names), if names.len() == 1 { "is a disk image" } else { "are disk images" }, if names.len() == 1 { "it is" } else { "they are" }));
        }
        if !plan.siblings.is_empty() {
            let names: Vec<String> = plan.siblings.iter().map(eject::name_of).collect();
            notes.push(format!("{} {} on the same disk, so {} ejected too.", list(&names), if names.len() == 1 { "is" } else { "are" }, if names.len() == 1 { "it is" } else { "they are" }));
        }
        if let Some(job) = &self.job
            && on(&job.root)
        {
            notes.push(format!("A scan of {} is running. Ejecting stops it.", job.root.shown()));
        }
        if let Some(op) = &self.ops.job {
            notes.push(format!("A file operation is running ({}). If it uses this drive, let it finish first.", op.label));
        }
        if notes.is_empty() {
            self.start_eject(plan, false);
        } else {
            self.eject.ask = Some(Ask::Confirm { root: root.to_path_buf(), name, notes });
        }
    }

    /// Let go of the drive here, then ask the system to eject it on a worker thread.
    fn start_eject(&mut self, plan: Plan, force: bool) {
        let going = roots(&plan);
        if let Some(job) = &self.job
            && going.iter().any(|r| job.root.starts_with(r))
        {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
        for r in &going {
            self.files.leave(r);
        }
        if going.iter().any(|r| self.scanned_path.starts_with(r)) {
            self.live = None;
        }
        let name = eject::name_of(&plan.drive);
        let image = plan.drive.info.kind == drives::Kind::Image;
        let root = plan.drive.root.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            // A moment for the folder watchers and previews to let go of it.
            std::thread::sleep(Duration::from_millis(400));
            let mut r = eject::eject(&plan, force);
            if matches!(r, Err(Failure::Busy { .. })) && !force {
                // The system's own indexer or a thumbnail reader often lets go a moment later.
                std::thread::sleep(Duration::from_millis(1200));
                r = eject::eject(&plan, false);
            }
            let _ = tx.send(r);
        });
        self.eject.busy.extend(going.iter().cloned());
        self.eject.jobs.push(EjectJob { root, name: name.clone(), roots: going, image, rx });
        self.ops.notice = Some((format!("Ejecting {name}…"), false));
    }

    pub(super) fn eject_poll(&mut self, ctx: &egui::Context) {
        if !self.eject.gone.is_empty() && self.drives_rx.is_none() {
            let drives = &self.drives;
            self.eject.gone.retain(|g| !drives.iter().any(|d| &d.root == g));
        }
        if self.eject.jobs.is_empty() {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        let mut done = Vec::new();
        self.eject.jobs.retain(|j| match j.rx.try_recv() {
            Ok(r) => {
                done.push((j.root.clone(), j.name.clone(), j.roots.clone(), j.image, r));
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => {
                done.push((j.root.clone(), j.name.clone(), j.roots.clone(), j.image, Err(Failure::Other("Ejecting stopped unexpectedly.".into()))));
                false
            }
        });
        for (root, name, roots, image, r) in done {
            self.eject.busy.retain(|b| !roots.contains(b));
            match r {
                Ok(()) => {
                    self.eject.gone.extend(roots);
                    self.ops.notice = Some((if image { format!("Ejected {name}.") } else { format!("Ejected {name}. It's safe to unplug.") }, false));
                }
                Err(failure @ Failure::Busy { .. }) => {
                    self.ops.notice = None;
                    self.eject.ask = Some(Ask::Busy { root, name, failure, force_armed: false });
                }
                Err(Failure::Other(message)) => {
                    self.ops.notice = None;
                    self.eject.ask = Some(Ask::Failed { name, message });
                }
            }
            self.refresh_drives();
        }
    }

    /// The question ejecting asks, if any.
    pub(super) fn eject_window(&mut self, ctx: &egui::Context) {
        let Some(ask) = &mut self.eject.ask else { return };
        enum Do {
            Nothing,
            Close,
            Eject(PathBuf, bool),
        }
        let mut act = Do::Nothing;
        egui::Modal::new(egui::Id::new("eject-ask")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            match ask {
                Ask::Confirm { root, name, notes } => {
                    ui.heading(format!("Eject {name}?"));
                    ui.add_space(4.0);
                    for n in notes.iter() {
                        ui.add(egui::Label::new(n.as_str()).wrap());
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if kit::button(ui, kit::Kind::Primary, None, "Eject").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            act = Do::Eject(root.clone(), false);
                        }
                        if ui.button("Cancel").clicked() {
                            act = Do::Close;
                        }
                    });
                }
                Ask::Busy { root, name, failure, force_armed } => {
                    ui.heading(format!("{name} is in use"));
                    ui.add_space(4.0);
                    let (holders, detail) = match failure {
                        Failure::Busy { holders, detail } => (holders.clone(), detail.clone()),
                        Failure::Other(s) => (Vec::new(), s.clone()),
                    };
                    if !detail.is_empty() {
                        ui.add(egui::Label::new(detail).wrap());
                    }
                    if holders.is_empty() {
                        ui.add(egui::Label::new("It can't be ejected while files on it are open. Close any documents, windows or terminals using it, then try again.").wrap());
                    } else {
                        ui.add(egui::Label::new("It can't be ejected while these apps have files open on it. Close them (or just the files from it), then try again:").wrap());
                        ui.add_space(2.0);
                        for h in &holders {
                            let line = if h.name == "Filescape" {
                                "•  Filescape (a preview, a copy or a folder still open here)".to_string()
                            } else if h.pid != 0 {
                                format!("•  {} (process {})", h.name, h.pid)
                            } else {
                                format!("•  {}", h.name)
                            };
                            ui.label(egui::RichText::new(line).strong());
                        }
                    }
                    if *force_armed {
                        ui.add_space(6.0);
                        ui.add(egui::Label::new(egui::RichText::new("Force Eject closes its files under the apps using them. Anything they haven't finished saving to it can be lost or damaged.").color(ui.visuals().error_fg_color)).wrap());
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if kit::button(ui, kit::Kind::Primary, None, "Try Again").clicked() {
                            act = Do::Eject(root.clone(), false);
                        }
                        let force = if *force_armed { "Force Eject Anyway" } else { "Force Eject…" };
                        if kit::button(ui, kit::Kind::Danger, None, force).on_hover_text("Unmount it even though files are open, as the system's own Force Eject does").clicked() {
                            if *force_armed {
                                act = Do::Eject(root.clone(), true);
                            } else {
                                *force_armed = true;
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            act = Do::Close;
                        }
                    });
                }
                Ask::Failed { name, message } => {
                    ui.heading(format!("Couldn't eject {name}"));
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(message.as_str()).wrap());
                    ui.add_space(8.0);
                    if ui.button("OK").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        act = Do::Close;
                    }
                }
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                act = Do::Close;
            }
        });
        match act {
            Do::Nothing => {}
            Do::Close => {
                self.eject.ask = None;
                ctx.input_mut(|i| {
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                });
            }
            Do::Eject(root, force) => {
                self.eject.ask = None;
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                match self.ejectable(&root).cloned() {
                    Some(d) => {
                        let plan = eject::plan(&d, &self.drives);
                        self.start_eject(plan, force);
                    }
                    // Gone already (unplugged meanwhile).
                    None => self.refresh_drives(),
                }
            }
        }
    }

    /// The ⏏ button at the right of a drive's row (`row`): a spinner while it's being ejected.
    /// True when clicked.
    pub(super) fn eject_button(&self, ui: &egui::Ui, row: Rect, root: &Path, cy: f32) -> bool {
        let r = Rect::from_center_size(pos2(row.right() - 14.0, cy), vec2(20.0, 20.0));
        if self.ejecting_now(root) {
            egui::Spinner::new().size(12.0).paint_at(ui, r.shrink(4.0));
            return false;
        }
        let resp = ui.interact(r, ui.id().with(("eject", root)), Sense::click());
        let pal = style::pal(ui);
        if resp.hovered() {
            ui.painter().rect_filled(r, 4.0, pal.hover);
        }
        ui.painter().text(r.center(), Align2::CENTER_CENTER, "⏏", FontId::proportional(13.0), if resp.hovered() { pal.strong } else { pal.weak });
        resp.on_hover_text("Eject").clicked()
    }
}
