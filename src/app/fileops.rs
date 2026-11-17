//! File commands in the Files workspace: copy, cut and paste, rename, delete, New, and dropping
//! items onto folders. They run through the same background operations (`menus::Ops`) as
//! Storage Analysis, so both workspaces behave the same and share one clipboard.

use super::browser::Clip;
use super::dnd::{self, DragItems, DropOp};
use super::*;
use crate::fsys::history::Location;
use crate::actions;
use crate::templates::{self, Builtin, UserTemplate};
use std::path::Path;

/// A drop to carry out after drawing: (items, into this folder, move or copy).
pub(super) type Drop = (Vec<PathBuf>, PathBuf, DropOp);

impl App {
    /// After a file operation has finished: read the open folders again, and select what was
    /// made (opening the Rename box on it for a new folder or file).
    pub(super) fn files_after_op(&mut self, made: &[PathBuf], rename: bool) {
        for p in &mut self.files.panes {
            for t in &mut p.tabs {
                t.reread();
            }
        }
        let Some(here) = self.files.here().dir().map(Path::to_path_buf) else { return };
        let names: Vec<Box<str>> = made.iter().filter(|p| p.parent() == Some(here.as_path())).filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into()).collect();
        if names.is_empty() {
            return;
        }
        let t = self.files.tab();
        t.selected = names.iter().cloned().collect();
        t.select_later = names.last().cloned();
        t.rename_later = rename;
    }

    /// Put items on dirmap's clipboard (and their paths on the system clipboard as text).
    pub(super) fn files_clip(&mut self, paths: Vec<PathBuf>, cut: bool) {
        if paths.is_empty() || (cut && paths.iter().any(|p| p.parent().is_none())) {
            return;
        }
        self.ctx_copy = Some(paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
        let keys = if cfg!(target_os = "macos") { "⌘V" } else { "Ctrl+V" };
        self.ops.notice = Some((format!("{} {}. Open a folder and press {keys} to paste.", if cut { "Cut" } else { "Copied" }, dnd::what(&paths)), false));
        self.browser.clip = Some(Clip { paths, cut });
    }

    pub(super) fn can_paste(&self) -> bool {
        // While another copy or move runs, a paste waits its turn.
        self.browser.clip.is_some()
    }

    /// Copy or move items into a folder, unless that makes no sense (into themselves).
    pub(super) fn files_transfer(&mut self, items: Vec<PathBuf>, dest: PathBuf, op: DropOp) {
        if crate::fsys::archive::split(&dest).is_some() {
            self.ops.notice = Some((files::READ_ONLY_NOTE.into(), true));
            return;
        }
        // Out of an archive is always a copy.
        let op = if items.iter().any(|p| crate::fsys::archive::inside(p)) { DropOp::Copy } else { op };
        if dnd::pointless(&items, &dest, op) {
            if items.iter().any(|s| dest.starts_with(s)) {
                self.ops.notice = Some(("Can't put a folder inside itself.".into(), true));
            }
            return;
        }
        self.transfer(items, dest, op == DropOp::Copy);
    }

    /// Ask before deleting (to the bin, or for good), saying how much is involved when known.
    pub(super) fn files_delete(&mut self, paths: Vec<PathBuf>, permanent: bool) {
        if paths.is_empty() || paths.iter().any(|p| p.parent().is_none()) {
            return;
        }
        let t = self.files.cur();
        let sizes = t.sizes.as_ref().map(|s| &s.1);
        let (mut size, mut files, mut sized, mut any_dir) = (0, 0, true, false);
        for p in &paths {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            match t.entry_named(&name) {
                Some(e) if e.is_dir => {
                    any_dir = true;
                    match sizes.and_then(|s| s.get(&e.name)) {
                        Some(&(b, f)) => {
                            size += b;
                            files += f;
                        }
                        None => sized = false,
                    }
                }
                Some(e) => {
                    size += e.size;
                    files += 1;
                }
                None => sized = false,
            }
        }
        let action = if permanent { menus::TreeAction::Delete } else { menus::TreeAction::Recycle };
        self.ops.confirm = Some(menus::Confirm { action, paths, is_dir: any_dir, size, files, sized });
    }

    /// A new folder in `dir`, then the Rename box for it.
    pub(super) fn files_new_folder(&mut self, dir: PathBuf) {
        if crate::fsys::archive::split(&dir).is_some() {
            self.ops.notice = Some((files::READ_ONLY_NOTE.into(), true));
            return;
        }
        if self.busy_note() {
            return;
        }
        self.make_new("Making a new folder".into(), move || actions::new_folder(&dir));
    }

    /// Make something new in the background, select it and open the Rename box on it. Undo
    /// sends it to the bin.
    pub(super) fn make_new(&mut self, label: String, make: impl FnOnce() -> Result<PathBuf, String> + Send + 'static) {
        self.ops.then_rename = true;
        let rec = self.ops.record();
        self.run_op_select(label, move || match make() {
            Ok(p) => {
                rec.lock().unwrap().push(crate::transfer::Step::Created(p.clone()));
                (Ok(None), vec![p.clone()], vec![p])
            }
            Err(e) => (Err(e), vec![], vec![]),
        });
    }

    /// A new file from one of dirmap's starting files, then the Rename box for it.
    pub(super) fn files_new_builtin(&mut self, dir: PathBuf, b: Builtin) {
        if self.busy_note() {
            return;
        }
        self.make_new(format!("Making {}", b.name), move || actions::new_file(&dir, b.name, b.contents.as_bytes(), b.executable));
    }

    /// A copy of one of the user's templates, then the Rename box for it.
    pub(super) fn files_new_from(&mut self, dir: PathBuf, t: UserTemplate) {
        if self.busy_note() {
            return;
        }
        let name = templates::new_name(&t.path);
        self.make_new(format!("Making {name}"), move || actions::new_from_template(&dir, &t.path, &name));
    }

    /// Say so when another operation is still running. True if one is.
    fn busy_note(&mut self) -> bool {
        if self.ops.job.is_some() {
            self.ops.notice = Some(("Another file operation is still running.".into(), true));
        }
        self.ops.job.is_some()
    }

    /// Open the templates folder in a Files tab (making it first).
    pub(super) fn open_templates_folder(&mut self) {
        let Some(dir) = templates::own_folder() else { return };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.ops.notice = Some((format!("Couldn't make {}: {e}", dir.display()), true));
            return;
        }
        self.files.new_tab(Location::Dir(dir));
        self.files.user_templates_rx = None;
    }

    /// Read the user's templates in the background (the folders could be anywhere).
    pub(super) fn load_templates(&mut self) {
        if self.files.user_templates_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(templates::user_templates(&templates::folders()));
        });
        self.files.user_templates_rx = Some(rx);
    }

    /// The New menu's items. Returns what was picked.
    pub(super) fn new_menu_items(&mut self, ui: &mut egui::Ui, dir: &Path) -> Option<NewPick> {
        let ctx = ui.ctx().clone();
        let k = |m, key| ctx.format_shortcut(&egui::KeyboardShortcut::new(m, key));
        self.load_templates();
        if let Some(rx) = &self.files.user_templates_rx
            && let Ok(t) = rx.try_recv()
        {
            self.files.user_templates = t;
        }
        let busy = self.ops.job.is_some();
        let mut out = None;
        ui.set_min_width(200.0);
        if ui.add_enabled(!busy, egui::Button::new("🗀 Folder").shortcut_text(k(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::N))).clicked() {
            out = Some(NewPick::Folder);
        }
        ui.separator();
        let (docs, scripts) = templates::builtins();
        let tip = |b: &Builtin| if b.contents.is_empty() { format!("{} (empty)", b.name) } else { format!("{}:\n\n{}", b.name, b.contents.trim_end()) };
        for b in docs {
            if ui.add_enabled(!busy, egui::Button::new(b.label)).on_hover_text(tip(b)).clicked() {
                out = Some(NewPick::Builtin(*b));
            }
        }
        ui.menu_button("Script", |ui| {
            for b in scripts {
                if ui.add_enabled(!busy, egui::Button::new(b.label)).on_hover_text(tip(b)).clicked() {
                    out = Some(NewPick::Builtin(*b));
                }
            }
        });
        ui.separator();
        let user = self.files.user_templates.clone();
        if !user.is_empty() {
            for t in user {
                let icon = if t.is_dir { "🗀 " } else { "" };
                if ui.add_enabled(!busy, egui::Button::new(format!("{icon}{}", t.label))).on_hover_text(format!("A copy of {}", t.path.display())).clicked() {
                    out = Some(NewPick::User(t));
                }
            }
            ui.separator();
        }
        if ui
            .button("Open Templates Folder")
            .on_hover_text("Put any file or folder in here (a .blend, a project folder, a document) and it appears in this menu. New makes a copy of it.")
            .clicked()
        {
            out = Some(NewPick::OpenFolder);
        }
        let _ = dir;
        out
    }

    pub(super) fn apply_new(&mut self, pick: NewPick, dir: PathBuf) {
        if crate::fsys::archive::split(&dir).is_some() {
            self.ops.notice = Some((files::READ_ONLY_NOTE.into(), true));
            return;
        }
        match pick {
            NewPick::Folder => self.files_new_folder(dir),
            NewPick::Builtin(b) => self.files_new_builtin(dir, b),
            NewPick::User(t) => self.files_new_from(dir, t),
            NewPick::OpenFolder => self.open_templates_folder(),
        }
    }

    /// A folder (row, tab, crumb, sidebar entry) that takes dropped items: an outline and the
    /// label by the pointer while something hovers it, and the drop when it's let go.
    pub(super) fn folder_drop(&mut self, ui: &egui::Ui, resp: &egui::Response, dest: &Path) -> Option<Drop> {
        let items = resp.dnd_hover_payload::<DragItems>()?;
        let op = dnd::op_for(&items.0, dest, ui.input(|i| i.modifiers), &self.drives);
        let (text, bad) = dnd::drop_text(&items.0, dest, op);
        let pal = style::pal(ui);
        ui.painter().rect_stroke(resp.rect.shrink(1.0), style::RADIUS as f32, Stroke::new(1.5, if bad { pal.weak } else { pal.accent }), egui::StrokeKind::Inside);
        dnd::pointer_label(ui.ctx(), &text, bad);
        self.files.drop_claimed = true;
        let dropped = resp.dnd_release_payload::<DragItems>()?;
        (!bad).then(|| (dropped.0.clone(), dest.to_path_buf(), op))
    }
}

/// Something picked in the New menu.
#[derive(Clone, Debug)]
pub(super) enum NewPick {
    Folder,
    Builtin(Builtin),
    User(UserTemplate),
    OpenFolder,
}
