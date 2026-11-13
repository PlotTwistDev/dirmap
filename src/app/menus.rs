//! Right-click menus for the directory tree and the extension list, their keyboard shortcuts,
//! and the windows and background jobs behind them (confirmations, hashing, properties).

use super::*;
use crate::actions::{self, Terminal};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64};

/// Something to do with an item of the tree.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum TreeAction {
    Toggle,
    Open,
    CopyPath,
    Reveal,
    Terminal(Terminal),
    Refresh,
    Properties,
    Hash,
    Exclude,
    Delete,
    Recycle,
    RemoveEmpty,
    Empty,
    MoveTo,
}

/// Something to do with a row of the extension list.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum ExtAction {
    Find,
    Highlight,
    Exclude,
    Copy,
}

/// Which menu items make sense for an item, and which are off right now.
pub(super) struct ItemInfo {
    pub is_dir: bool,
    pub open: bool,
    /// The scanned folder itself: it can't be deleted or moved from here.
    pub is_root: bool,
    /// A whole drive: never emptied from here.
    pub is_drive: bool,
    /// Another file operation is still running.
    pub busy: bool,
    /// Right-clicked in the Files view: folders open there rather than expanding in the list.
    pub in_browser: bool,
}

/// A file operation running in the background.
pub(super) struct OpJob {
    pub label: String,
    rx: mpsc::Receiver<OpDone>,
}

struct OpDone {
    error: Option<String>,
    note: Option<String>,
    /// Fresh state of each affected path: `None` when it no longer exists.
    updates: Vec<(PathBuf, Option<Node>)>,
}

/// A destructive action waiting for "are you sure?".
pub(super) struct Confirm {
    action: TreeAction,
    path: PathBuf,
    is_dir: bool,
    size: u64,
    files: u64,
}

pub(super) struct HashJob {
    path: PathBuf,
    size: u64,
    read: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    rx: mpsc::Receiver<Result<String, String>>,
    result: Option<Result<String, String>>,
}

/// Every file of one type, largest first.
pub(super) struct TypeFiles {
    ext: String,
    total: u64,
    files: Vec<(Vec<u32>, u64)>,
}

#[derive(Default)]
pub(super) struct Ops {
    pub job: Option<OpJob>,
    confirm: Option<Confirm>,
    hash: Option<HashJob>,
    /// Item shown in dirmap's own Properties window (where the system has none).
    props: Option<PathBuf>,
    type_files: Option<TypeFiles>,
    /// Last result, shown in the status bar: (text, is an error).
    pub notice: Option<(String, bool)>,
}

impl Ops {
    /// A window of ours is up and owns the keyboard.
    pub(super) fn modal(&self) -> bool {
        self.confirm.is_some()
    }
}

fn shortcut(ctx: &egui::Context, mods: egui::Modifiers, key: egui::Key) -> String {
    ctx.format_shortcut(&egui::KeyboardShortcut::new(mods, key))
}

/// A menu item with its shortcut on the right; true when clicked.
fn item(ui: &mut egui::Ui, enabled: bool, text: impl Into<egui::WidgetText>, keys: &str) -> bool {
    ui.add_enabled(enabled, egui::Button::new(text).shortcut_text(keys)).clicked()
}

impl App {
    /// The tree's right-click menu. Returns what was picked.
    pub(super) fn tree_menu(ui: &mut egui::Ui, info: &ItemInfo) -> Option<TreeAction> {
        use egui::{Key, Modifiers as M};
        let ctx = ui.ctx().clone();
        let k = |m, key| shortcut(&ctx, m, key);
        let mut pick = None;
        let mut set = |cond: bool, a| {
            if cond {
                pick = Some(a);
            }
        };
        ui.set_min_width(250.0);
        let can_change = !info.busy;
        if info.is_dir {
            let label = if info.in_browser { "Open Folder" } else if info.open { "Collapse" } else { "Expand" };
            let label = egui::RichText::new(label).strong();
            let keys = if info.in_browser { k(M::NONE, Key::Enter) } else { String::new() };
            set(item(ui, true, label, &keys), TreeAction::Toggle);
            ui.separator();
        }
        let open = if info.is_dir { egui::RichText::new("Open…") } else { egui::RichText::new("Open…").strong() };
        let open_keys = if info.in_browser && info.is_dir { String::new() } else { k(M::NONE, Key::Enter) };
        set(item(ui, true, open, &open_keys), TreeAction::Open);
        set(item(ui, true, "Copy Path", &k(M::COMMAND, Key::C)), TreeAction::CopyPath);
        let reveal = if cfg!(windows) { format!("Select in {}…", actions::FILE_MANAGER) } else { format!("Show in {}…", actions::FILE_MANAGER) };
        set(item(ui, true, reveal, &k(M::COMMAND, Key::E)), TreeAction::Reveal);
        for (i, (label, t)) in actions::terminals().iter().enumerate() {
            let key = if i == 0 { Key::P } else { Key::S };
            set(item(ui, true, *label, &k(M::COMMAND, key)), TreeAction::Terminal(*t));
        }
        set(item(ui, can_change, "Refresh Selected", &k(M::NONE, Key::F5)), TreeAction::Refresh);
        set(item(ui, true, "Properties", &k(M::ALT, Key::Enter)), TreeAction::Properties);
        set(item(ui, !info.is_dir, "Compute Hash (SHA-256)", &k(M::COMMAND, Key::H)), TreeAction::Hash);
        set(item(ui, !info.is_root, "Add to Exclusion Filter", ""), TreeAction::Exclude);
        ui.separator();
        let deletable = can_change && !info.is_root;
        set(item(ui, deletable, "Delete Permanently…", &k(M::SHIFT, Key::Delete)), TreeAction::Delete);
        set(item(ui, deletable, format!("Delete (to {})…", actions::BIN), &k(M::NONE, Key::Delete)), TreeAction::Recycle);
        if info.is_dir {
            set(item(ui, can_change, "Remove Empty Folders", ""), TreeAction::RemoveEmpty);
            set(item(ui, can_change && !info.is_drive, "Empty Folder…", ""), TreeAction::Empty);
        }
        set(item(ui, deletable, "Move to…", &k(M::COMMAND, Key::M)), TreeAction::MoveTo);
        pick
    }

    /// The extension list's right-click menu.
    pub(super) fn ext_menu(ui: &mut egui::Ui, ext: &str, highlighted: bool) -> Option<ExtAction> {
        let mut pick = None;
        ui.set_min_width(220.0);
        let name = if ext.is_empty() { "files without an extension".to_string() } else { format!(".{ext} files") };
        if ui.button(egui::RichText::new(format!("🔍 Find all {name}")).strong()).clicked() {
            pick = Some(ExtAction::Find);
        }
        if ui.button(if highlighted { "Clear Highlight" } else { "Highlight in Map" }).clicked() {
            pick = Some(ExtAction::Highlight);
        }
        ui.separator();
        if ui.add_enabled(!ext.is_empty(), egui::Button::new("Copy Extension")).clicked() {
            pick = Some(ExtAction::Copy);
        }
        let ex = ui
            .add_enabled(!ext.is_empty(), egui::Button::new("Add to Exclusion Filter"))
            .on_hover_text(format!("Skip *.{ext} files from the next scan on"));
        if ex.clicked() {
            pick = Some(ExtAction::Exclude);
        }
        pick
    }

    pub(super) fn item_info(&self, path: &[u32]) -> Option<ItemInfo> {
        // Paths from the tree panel and the maps are in the tree on screen (matches only, or all).
        let tree = self.shown_tree()?;
        let n = tree.at(path)?;
        Some(ItemInfo {
            is_dir: n.is_dir,
            open: self.expanded.contains(path),
            is_root: path.is_empty(),
            is_drive: path.is_empty() && self.scanned_path.parent().is_none(),
            busy: self.ops.job.is_some(),
            in_browser: false,
        })
    }

    /// Tree shortcuts act on the selection, from anywhere except a text box.
    pub(super) fn tree_shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers as M};
        if self.tree.is_none() || self.job.is_some() || self.ops.modal() || ctx.text_edit_focused() {
            return;
        }
        let Some(sel) = self.selected.clone() else { return };
        let Some(info) = self.item_info(&sel) else { return };
        // In the Files view, Enter opens a folder there, like Explorer.
        if self.map_kind == MapKind::Browser && info.is_dir && ctx.input_mut(|i| i.consume_key(M::NONE, Key::Enter)) {
            self.browse_into(sel);
            return;
        }
        let action = ctx.input_mut(|i| {
            // Ctrl+C reaches egui as a Copy event rather than a key press.
            let copy = i.events.iter().any(|e| matches!(e, egui::Event::Copy)) || i.consume_key(M::CTRL, Key::Insert);
            let terms = actions::terminals();
            let mut keys = vec![
                (M::SHIFT, Key::Delete, TreeAction::Delete),
                (M::NONE, Key::Delete, TreeAction::Recycle),
                (M::ALT, Key::Enter, TreeAction::Properties),
                (M::NONE, Key::Enter, TreeAction::Open),
                (M::COMMAND, Key::E, TreeAction::Reveal),
                (M::NONE, Key::F5, TreeAction::Refresh),
                (M::COMMAND, Key::H, TreeAction::Hash),
                (M::COMMAND, Key::M, TreeAction::MoveTo),
                (M::COMMAND, Key::P, TreeAction::Terminal(terms[0].1)),
            ];
            if let Some(t) = terms.get(1) {
                keys.push((M::COMMAND, Key::S, TreeAction::Terminal(t.1)));
            }
            if copy {
                return Some(TreeAction::CopyPath);
            }
            keys.into_iter().find(|&(m, k, _)| i.consume_key(m, k)).map(|(_, _, a)| a)
        });
        let Some(a) = action else { return };
        // The same rules as the menu, which greys these out.
        let allowed = match a {
            TreeAction::Delete | TreeAction::Recycle | TreeAction::MoveTo => !info.busy && !info.is_root,
            TreeAction::Refresh => !info.busy,
            TreeAction::Hash => !info.is_dir,
            _ => true,
        };
        if allowed {
            self.tree_action(a, sel);
        }
    }

    pub(super) fn tree_action(&mut self, a: TreeAction, path: Vec<u32>) {
        let Some(tree) = self.shown_tree() else { return };
        let Some(n) = tree.at(&path) else { return };
        let disk = self.full_path(&path);
        self.selected = Some(path.clone());
        let result = match a {
            TreeAction::Toggle => {
                if !self.expanded.remove(&path[..]) {
                    self.expanded.insert(path.into());
                }
                self.rows = None;
                Ok(())
            }
            TreeAction::Open => actions::open(&disk),
            TreeAction::CopyPath => {
                self.ctx_copy = Some(disk.display().to_string());
                self.ops.notice = Some((format!("Copied {}", disk.display()), false));
                Ok(())
            }
            TreeAction::Reveal => actions::reveal(&disk),
            TreeAction::Terminal(t) => actions::terminal(&disk, t),
            TreeAction::Properties => {
                if !actions::shell_properties(&disk) {
                    self.ops.props = Some(disk);
                }
                Ok(())
            }
            TreeAction::Hash => {
                self.start_hash(disk, n.size);
                Ok(())
            }
            TreeAction::Exclude => {
                let (list, entry) = if n.is_dir {
                    (&mut self.settings.exclude_folders, disk.display().to_string())
                } else {
                    (&mut self.settings.exclude_files, n.name.to_string())
                };
                add_line(list, &entry);
                self.ops.notice = Some((format!("Added {entry} to the exclusion filter. Rescan to apply it."), false));
                Ok(())
            }
            TreeAction::Delete | TreeAction::Recycle | TreeAction::Empty => {
                self.ops.confirm = Some(Confirm { action: a, path: disk, is_dir: n.is_dir, size: n.size, files: n.files });
                Ok(())
            }
            TreeAction::Refresh if path.is_empty() => {
                self.path_input = self.scanned_path.display().to_string();
                self.start_scan();
                Ok(())
            }
            TreeAction::Refresh => {
                self.run_op(format!("Refreshing {}", n.name), move || (Ok(None), vec![disk]));
                Ok(())
            }
            TreeAction::RemoveEmpty => {
                self.run_op(format!("Removing empty folders in {}", n.name), move || {
                    let r = actions::remove_empty_folders(&disk).map(|c| match c {
                        0 => Some("No empty folders found.".to_string()),
                        1 => Some("Removed 1 empty folder.".to_string()),
                        c => Some(format!("Removed {} empty folders.", group(c))),
                    });
                    (r.map_err(|e| format!("Couldn't remove every empty folder: {e}")), vec![disk])
                });
                Ok(())
            }
            TreeAction::MoveTo => {
                let start = disk.parent().map(Path::to_path_buf).unwrap_or_default();
                if let Some(dest) = rfd::FileDialog::new().set_title("Move to folder").set_directory(start).pick_folder() {
                    let root = self.scanned_path.clone();
                    self.run_op(format!("Moving {}", n.name), move || match actions::move_into(&disk, &dest) {
                        Ok(to) => {
                            let mut affected = vec![disk];
                            if to.starts_with(&root) {
                                affected.push(to.clone());
                            }
                            (Ok(Some(format!("Moved to {}", to.display()))), affected)
                        }
                        Err(e) => (Err(e), vec![disk]),
                    });
                }
                Ok(())
            }
        };
        if let Err(e) = result {
            self.ops.notice = Some((e, true));
        }
    }

    pub(super) fn ext_action(&mut self, a: ExtAction, ext: String) {
        match a {
            ExtAction::Highlight => {
                self.highlight_ext = if self.highlight_ext.as_ref() == Some(&ext) { None } else { Some(ext) };
            }
            ExtAction::Copy => {
                self.ctx_copy = Some(format!(".{ext}"));
            }
            ExtAction::Exclude => {
                let glob = format!("*.{ext}");
                add_line(&mut self.settings.exclude_files, &glob);
                self.ops.notice = Some((format!("Added {glob} to the exclusion filter. Rescan to apply it."), false));
            }
            ExtAction::Find => {
                let Some(tree) = self.tree.clone() else { return };
                let mut files = Vec::new();
                fn walk(n: &Node, ext: &str, path: &mut Vec<u32>, out: &mut Vec<(Vec<u32>, u64)>) {
                    for (i, c) in n.children.iter().enumerate() {
                        path.push(i as u32);
                        if c.is_dir {
                            walk(c, ext, path, out);
                        } else if c.ext().eq_ignore_ascii_case(ext) {
                            out.push((path.clone(), c.size));
                        }
                        path.pop();
                    }
                }
                walk(&tree, &ext, &mut Vec::new(), &mut files);
                files.sort_by_key(|f| std::cmp::Reverse(f.1));
                let total = files.iter().map(|f| f.1).sum();
                self.ops.type_files = Some(TypeFiles { ext, total, files });
            }
        }
    }

    /// Run a file operation on a background thread. `work` returns its outcome (an optional
    /// note on success) and the paths it touched, which are then re-read from disk.
    fn run_op(&mut self, label: String, work: impl FnOnce() -> (Result<Option<String>, String>, Vec<PathBuf>) + Send + 'static) {
        if self.ops.job.is_some() {
            return;
        }
        let opts = self.options_for(self.scanned_with.as_ref().unwrap_or(&self.settings));
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (r, affected) = work();
            let updates = affected.into_iter().map(|p| {
                let n = reread(&p, &opts);
                (p, n)
            });
            let updates = updates.collect();
            let (error, note) = match r {
                Ok(note) => (None, note),
                Err(e) => (Some(e), None),
            };
            let _ = tx.send(OpDone { error, note, updates });
        });
        self.ops.notice = None;
        self.ops.job = Some(OpJob { label, rx });
    }

    fn start_hash(&mut self, path: PathBuf, size: u64) {
        if let Some(h) = &self.ops.hash {
            h.cancel.store(true, Ordering::Relaxed);
        }
        let (read, cancel) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicBool::new(false)));
        let (tx, rx) = mpsc::channel();
        let (p, r, c) = (path.clone(), read.clone(), cancel.clone());
        std::thread::spawn(move || {
            let _ = tx.send(actions::sha256(&p, &r, &c).map_err(|e| e.to_string()));
        });
        self.ops.hash = Some(HashJob { path, size, read, cancel, rx, result: None });
    }

    /// Poll background work and show the menus' windows. Call once per frame.
    pub(super) fn ops_ui(&mut self, ctx: &egui::Context) {
        if let Some(text) = self.ctx_copy.take() {
            ctx.copy_text(text);
        }
        if let Some(job) = &self.ops.job {
            match job.rx.try_recv() {
                Ok(done) => {
                    self.ops.job = None;
                    self.apply_updates(done.updates);
                    self.ops.notice = done.error.map(|e| (e, true)).or(done.note.map(|n| (n, false)));
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => self.ops.job = None,
            }
        }
        self.confirm_window(ctx);
        self.hash_window(ctx);
        self.props_window(ctx);
        self.type_files_window(ctx);
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        let Some(c) = &self.ops.confirm else { return };
        let what = if c.is_dir { "folder" } else { "file" };
        let (title, verb, warn) = match c.action {
            TreeAction::Delete => ("Delete permanently", "Delete", format!("This {what} will be deleted for good. It won't go to the {}.", actions::BIN)),
            TreeAction::Recycle => (if cfg!(windows) { "Delete to Recycle Bin" } else { "Move to Trash" }, "Delete", format!("This {what} will be moved to the {}.", actions::BIN)),
            _ => ("Empty folder", "Empty Folder", format!("Everything inside this folder will be deleted for good. It won't go to the {}.", actions::BIN)),
        };
        let amount = if c.is_dir { format!("{} in {}", scan::human(c.size), files_text(c.files)) } else { scan::human(c.size) };
        let mut go = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("confirm-delete")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading(title);
            ui.add_space(6.0);
            ui.label(egui::RichText::new(c.path.display().to_string()).strong());
            ui.label(&amount);
            ui.add_space(6.0);
            ui.colored_label(if c.action == TreeAction::Recycle { ui.visuals().text_color() } else { ui.visuals().warn_fg_color }, warn);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                go = ui.button(verb).clicked();
                cancel = ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if cancel {
            self.ops.confirm = None;
        } else if go {
            let c = self.ops.confirm.take().unwrap();
            let name = c.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let p = c.path;
            match c.action {
                TreeAction::Delete => self.run_op(format!("Deleting {name}"), move || {
                    (actions::delete(&p).map(|_| None).map_err(|e| format!("Couldn't delete everything: {e}")), vec![p])
                }),
                TreeAction::Recycle => self.run_op(format!("Moving {name} to the {}", actions::BIN), move || (actions::trash(&p).map(|_| None), vec![p])),
                _ => self.run_op(format!("Emptying {name}"), move || {
                    (actions::empty_folder(&p).map(|_| None).map_err(|e| format!("Couldn't delete everything: {e}")), vec![p])
                }),
            }
        }
    }

    fn hash_window(&mut self, ctx: &egui::Context) {
        let Some(h) = &mut self.ops.hash else { return };
        if h.result.is_none() {
            if let Ok(r) = h.rx.try_recv() {
                h.result = Some(r);
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        let mut open = true;
        let mut copy = None;
        egui::Window::new("SHA-256").open(&mut open).pivot(Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(h.path.display().to_string());
            ui.add_space(4.0);
            match &h.result {
                None => {
                    let done = h.read.load(Ordering::Relaxed) as f32 / h.size.max(1) as f32;
                    ui.add(egui::ProgressBar::new(done).desired_width(420.0).show_percentage());
                }
                Some(Ok(hex)) => {
                    ui.horizontal(|ui| {
                        ui.monospace(hex);
                        if ui.button("Copy").clicked() {
                            copy = Some(hex.clone());
                        }
                    });
                }
                Some(Err(e)) => {
                    ui.colored_label(ui.visuals().error_fg_color, format!("Couldn't read the file: {e}"));
                }
            }
        });
        if let Some(t) = copy {
            self.ctx_copy = Some(t);
        }
        if !open && let Some(h) = self.ops.hash.take() {
            h.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn props_window(&mut self, ctx: &egui::Context) {
        let Some(path) = self.ops.props.clone() else { return };
        let node = self.tree_path_of(&path).and_then(|p| self.shown_tree()?.at(&p).map(|n| (n.size, n.files, n.modified, n.is_dir)));
        let md = std::fs::symlink_metadata(&path).ok();
        let mut open = true;
        egui::Window::new("Properties").open(&mut open).pivot(Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).collapsible(false).resizable(false).show(ctx, |ui| {
            egui::Grid::new("props").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string());
                ui.label("Name");
                ui.strong(name);
                ui.end_row();
                ui.label("Location");
                ui.label(path.parent().map(|p| p.display().to_string()).unwrap_or_default());
                ui.end_row();
                if let Some((size, files, modified, is_dir)) = node {
                    ui.label("Type");
                    ui.label(if is_dir { "Folder".to_string() } else { categories::of(&path.to_string_lossy()).label().to_string() });
                    ui.end_row();
                    ui.label("Size");
                    ui.label(format!("{} ({} bytes)", scan::human(size), group(size)));
                    ui.end_row();
                    if is_dir {
                        ui.label("Contains");
                        ui.label(files_text(files));
                        ui.end_row();
                    }
                    if modified > 0 {
                        ui.label(if is_dir { "Newest change" } else { "Modified" });
                        ui.label(fmt_time(modified));
                        ui.end_row();
                    }
                }
                if let Some(md) = &md {
                    let secs = |t: std::io::Result<std::time::SystemTime>| {
                        t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64)
                    };
                    if let Some(c) = secs(md.created()) {
                        ui.label("Created");
                        ui.label(fmt_time(c));
                        ui.end_row();
                    }
                    if let Some(a) = secs(md.accessed()) {
                        ui.label("Accessed");
                        ui.label(fmt_time(a));
                        ui.end_row();
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        ui.label("Permissions");
                        ui.monospace(format!("{:o}", md.permissions().mode() & 0o7777));
                        ui.end_row();
                    }
                    ui.label("Read-only");
                    ui.label(if md.permissions().readonly() { "Yes" } else { "No" });
                    ui.end_row();
                } else {
                    ui.label("On disk");
                    ui.colored_label(ui.visuals().warn_fg_color, "No longer there");
                    ui.end_row();
                }
            });
        });
        if !open {
            self.ops.props = None;
        }
    }

    fn type_files_window(&mut self, ctx: &egui::Context) {
        let Some(tf) = &self.ops.type_files else { return };
        let mut open = true;
        let mut pick = None;
        let title = if tf.ext.is_empty() { "Files without an extension".to_string() } else { format!(".{} files", tf.ext) };
        let root = self.scanned_path.clone();
        let tree = self.tree.clone();
        egui::Window::new(title).id(egui::Id::new("type-files")).open(&mut open).pivot(Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).default_size([520.0, 420.0]).show(ctx, |ui| {
            ui.label(format!("{} · {}. Click one to select it.", files_text(tf.files.len() as u64), scan::human(tf.total)));
            ui.separator();
            let Some(tree) = tree else { return };
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_H, tf.files.len(), |ui, range| {
                for (path, size) in &tf.files[range] {
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                    let vis = ui.visuals().clone();
                    // These paths are in the whole scan; the selection is in the tree on screen.
                    let selected = self.selected.as_deref().is_some_and(|s| self.full_of(s) == path[..]);
                    if selected {
                        ui.painter().rect_filled(rect, 2.0, vis.selection.bg_fill);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 2.0, vis.widgets.hovered.weak_bg_fill);
                    }
                    let tc = if selected { vis.selection.stroke.color } else { vis.text_color() };
                    let mut rel = PathBuf::new();
                    let mut n: &Node = &tree;
                    for &i in path {
                        let Some(c) = n.children.get(i as usize) else { break };
                        n = c;
                        rel.push(&*n.name);
                    }
                    let font = FontId::proportional(13.0);
                    let cy = rect.center().y;
                    let clip = Rect::from_min_max(rect.min, pos2(rect.right() - 90.0, rect.bottom()));
                    ui.painter()
                        .with_clip_rect(clip.intersect(ui.clip_rect()))
                        .text(pos2(rect.left() + 4.0, cy), Align2::LEFT_CENTER, rel.display().to_string(), font.clone(), tc);
                    ui.painter().text(pos2(rect.right() - 4.0, cy), Align2::RIGHT_CENTER, scan::human(*size), font, tc);
                    if resp.on_hover_text(root.join(&rel).display().to_string()).clicked() {
                        pick = Some(path.clone());
                    }
                }
            });
        });
        if let Some(p) = pick {
            self.select_full(p);
        }
        if !open {
            self.ops.type_files = None;
        }
    }

    /// Where a path on disk is in the tree, if it's there.
    pub(super) fn tree_path_of(&self, disk: &Path) -> Option<Vec<u32>> {
        let rel = disk.strip_prefix(&self.scanned_path).ok()?;
        let tree = self.shown_tree()?;
        let mut n: &Node = &tree;
        let mut out = Vec::new();
        for c in rel.components() {
            let name = c.as_os_str().to_string_lossy();
            let i = n.children.iter().position(|k| *k.name == *name)?;
            out.push(i as u32);
            n = &n.children[i];
        }
        Some(out)
    }

    /// Put fresh copies of changed items into the tree (or take them out), keeping the
    /// selection, the zoomed folder and the open folders where they were.
    fn apply_updates(&mut self, updates: Vec<(PathBuf, Option<Node>)>) {
        if updates.is_empty() || self.tree.is_none() {
            return;
        }
        // Work in whole-scan positions; "Show matches only" is rebuilt from the new tree.
        self.drop_filtered_view();
        // Tree positions change when sizes do, so remember things by their paths on disk.
        let selected = self.selected.as_ref().map(|p| self.full_path(p));
        let zoom = self.full_path(&self.zoom);
        let expanded: Vec<PathBuf> = self.expanded.iter().map(|p| self.full_path(p)).collect();

        self.map.clear();
        let old = self.tree.take().unwrap();
        let mut tree = Arc::try_unwrap(old).unwrap_or_else(|shared| (*shared).clone());
        for (disk, new) in updates {
            let Ok(rel) = disk.strip_prefix(&self.scanned_path) else { continue };
            let names: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            match new {
                _ if !names.is_empty() => {
                    replace_at(&mut tree, &names, new);
                }
                Some(mut n) => {
                    n.name = tree.name.clone();
                    tree = n;
                }
                None => {
                    tree.children.clear();
                    resum(&mut tree);
                }
            }
        }
        self.tree = Some(Arc::new(tree));

        // Back to tree positions; anything gone falls back to its nearest surviving folder.
        let nearest = |app: &Self, p: &Path| p.ancestors().find_map(|a| app.tree_path_of(a)).unwrap_or_default();
        self.selected = selected.map(|p| nearest(self, &p));
        self.zoom = nearest(self, &zoom);
        self.expanded = expanded.iter().filter_map(|p| self.tree_path_of(p)).map(Into::into).collect();
        self.expanded.insert(Box::new([]));
        if let Some(sel) = self.selected.clone() {
            for k in 0..sel.len() {
                self.expanded.insert(sel[..k].into());
            }
        }

        let tree = self.tree.clone().unwrap();
        self.exts = scan::ext_stats(&tree);
        for (i, e) in self.exts.iter().enumerate() {
            self.colors.entry(e.ext.clone()).or_insert_with(|| ext_color(&e.ext, i));
        }
        self.rows = None;
        self.filter_stats = None;
        self.hits = None;
        self.hist = None;
        self.sun = None;
        // Duplicate results point at old tree positions.
        if let Some(j) = self.dup_job.take() {
            j.progress.cancel.store(true, Ordering::Relaxed);
        }
        self.dup_groups.clear();
        self.dup_rows.clear();
        self.dup_set = Arc::new(HashSet::new());
        self.dupes_only = false;
        self.dup_done = false;
        self.compare();
        self.paint_gen += 1;
    }
}

fn files_text(n: u64) -> String {
    if n == 1 { "1 file".into() } else { format!("{} files", group(n)) }
}

/// Add a line to a one-per-line settings list, unless it's already there.
fn add_line(list: &mut String, line: &str) {
    if list.lines().any(|l| l.trim() == line) {
        return;
    }
    if !list.is_empty() && !list.ends_with('\n') {
        list.push('\n');
    }
    list.push_str(line);
}

/// Read one item from disk again, as the scan would see it. `None` if it's gone.
fn reread(path: &Path, opts: &Options) -> Option<Node> {
    let md = std::fs::symlink_metadata(path).ok()?;
    if md.is_dir() {
        Some(scan::scan(path, opts, &Progress::default(), &scan::new_live(path)))
    } else if md.is_file() {
        let modified = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
        Some(Node { name: scan::root_name(path), size: md.len(), files: 1, modified, is_dir: false, children: Vec::new() })
    } else {
        None
    }
}

/// Replace, add or remove (`new` = None) the item at `names` below `n`, then fix up the totals
/// and order of every folder on the way.
fn replace_at(n: &mut Node, names: &[String], new: Option<Node>) {
    let Some((first, rest)) = names.split_first() else { return };
    let i = n.children.iter().position(|c| *c.name == **first);
    if rest.is_empty() {
        match (i, new) {
            (Some(i), Some(mut c)) => {
                c.name = n.children[i].name.clone();
                n.children[i] = c;
            }
            (Some(i), None) => {
                n.children.remove(i);
            }
            (None, Some(c)) => n.children.push(c),
            (None, None) => return,
        }
    } else {
        match i {
            Some(i) if n.children[i].is_dir => replace_at(&mut n.children[i], rest, new),
            _ => return,
        }
    }
    resum(n);
}

/// Recompute a folder's totals from its children and sort them as the scan does.
fn resum(n: &mut Node) {
    n.children.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    n.size = n.children.iter().map(|c| c.size).sum();
    n.files = n.children.iter().map(|c| c.files).sum();
    n.modified = n.children.iter().map(|c| c.modified).max().unwrap_or(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> Node {
        Node { name: name.into(), size, files: 1, modified: size as i64, is_dir: false, children: Vec::new() }
    }

    fn dir(name: &str, mut children: Vec<Node>) -> Node {
        let mut n = Node { name: name.into(), size: 0, files: 0, modified: 0, is_dir: true, children: Vec::new() };
        n.children.append(&mut children);
        resum(&mut n);
        n
    }

    #[test]
    fn replace_keeps_totals_and_order() {
        let mut root = dir("root", vec![dir("a", vec![file("x", 10), file("y", 5)]), file("b", 12)]);
        assert_eq!((root.size, root.files), (27, 3));
        assert_eq!(&*root.children[0].name, "a");

        // Removing a file shrinks its folder below its sibling, which moves ahead.
        replace_at(&mut root, &["a".into(), "x".into()], None);
        assert_eq!((root.size, root.files), (17, 2));
        assert_eq!(&*root.children[0].name, "b");
        assert_eq!(root.children[1].size, 5);

        // Adding and replacing.
        replace_at(&mut root, &["a".into(), "z".into()], Some(file("z", 100)));
        assert_eq!(&*root.children[0].name, "a");
        assert_eq!(root.size, 117);
        replace_at(&mut root, &["b".into()], Some(file("b", 1)));
        assert_eq!((root.size, root.modified), (106, 100));

        // A path that isn't there changes nothing.
        replace_at(&mut root, &["nope".into(), "q".into()], None);
        assert_eq!(root.size, 106);
    }

    #[test]
    fn add_line_once() {
        let mut s = String::from("C:\\Windows");
        add_line(&mut s, "D:\\x");
        add_line(&mut s, "D:\\x");
        assert_eq!(s, "C:\\Windows\nD:\\x");
    }
}
