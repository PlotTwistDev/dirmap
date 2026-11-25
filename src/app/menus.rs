//! Right-click menus for the directory tree and the extension list, their keyboard shortcuts,
//! and the windows and background jobs behind them (confirmations, hashing, properties).

use crate::fsys::PathShow;
use super::*;
use crate::actions::{self, Terminal};
use std::path::Path;
use crate::transfer::{self, Choice, Clash, Step};
use std::sync::Mutex;
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
    // File manager only:
    Rename,
    CopyItems,
    CutItems,
    Paste,
    NewFolder,
    NewTab,
    /// Cloud files: remove the local copy, or keep them on this computer (`cloudops`).
    FreeUp,
    KeepHere,
    /// The "What Is This?" window for a system file or folder.
    Explain,
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
    /// Right-clicked in the file manager: folders open there rather than expanding in the list,
    /// and the file manager's own commands are offered.
    pub in_browser: bool,
    /// How many items the menu acts on (several selected in the file manager).
    pub count: usize,
    /// Something has been copied or cut, ready to paste.
    pub can_paste: bool,
    /// Cloud files the system can free up or keep are in it (see `cloudops`).
    pub cloud: bool,
    /// A system file or folder Filescape can explain, or one inside such a folder.
    pub known: bool,
}

/// A file operation running in the background.
pub(super) struct OpJob {
    pub label: String,
    rx: mpsc::Receiver<OpDone>,
    /// Pause, stop and progress, for copies and moves.
    pub ctl: Option<Arc<transfer::Ctl>>,
}

/// A copy or move asked for while another was running.
pub(super) struct Queued {
    pub sources: Vec<PathBuf>,
    pub dest: PathBuf,
    pub copy: bool,
}

struct OpDone {
    error: Option<String>,
    note: Option<String>,
    /// Fresh state of each affected path: `None` when it no longer exists.
    updates: Vec<(PathBuf, Option<Node>)>,
    /// Items to select afterwards (copies whose names were only known once made).
    select: Vec<PathBuf>,
}

/// A destructive action waiting for "are you sure?".
pub(super) struct Confirm {
    pub action: TreeAction,
    /// What it applies to (several when more than one item was selected).
    pub paths: Vec<PathBuf>,
    pub is_dir: bool,
    pub size: u64,
    pub files: u64,
    /// The size is known (folders from the Files list have no size until they're analysed).
    pub sized: bool,
}

/// The Rename box.
pub(super) struct RenameBox {
    pub path: PathBuf,
    pub text: String,
    /// Put the cursor in the box (and select the name) on the first frame.
    pub fresh: bool,
    pub error: Option<String>,
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
    pub confirm: Option<Confirm>,
    pub rename: Option<RenameBox>,
    /// Select this once the running operation has finished (a renamed item, a new folder…).
    pub then_select: Option<PathBuf>,
    /// …and open the Rename box on it (a new folder).
    pub then_rename: bool,
    hash: Option<HashJob>,
    /// Item shown in dirmap's own Properties window (where the system has none).
    pub props: Option<PathBuf>,
    type_files: Option<TypeFiles>,
    /// Last result, shown in the status bar: (text, is an error).
    pub notice: Option<(String, bool)>,
    /// The batch rename window, while it's open.
    pub batch: Option<super::batchren::BatchRename>,
    /// What can be undone, the latest last.
    pub undo: Vec<UndoEntry>,
    /// What was undone and can be redone, the latest last. Any new change clears it.
    pub redo: Vec<UndoEntry>,
    /// The running operation is an Undo or Redo: (what it records goes to `redo`, the label).
    pub undoing: Option<(bool, String)>,
    /// The steps of the running operation, as it records them.
    pub recording: Option<Arc<Mutex<Vec<Step>>>>,
    /// The latest change has just finished: the status bar offers Undo beside its note.
    pub undo_fresh: bool,
    /// A copy or move asking what to do about a name that's taken.
    pub clash: Option<ClashAsk>,
    /// Copies and moves waiting for the running one.
    pub queued: std::collections::VecDeque<Queued>,
}

/// Something that can be undone: what to call it, and the steps to play back.
pub(super) struct UndoEntry {
    pub label: String,
    pub steps: Vec<Step>,
}

/// The reply to a clash: the choice, and whether it goes for the rest of this kind too.
pub(super) type ClashReply = mpsc::Sender<(Choice, bool)>;

/// A running copy or move's questions about clashes.
pub(super) struct ClashAsk {
    pub rx: mpsc::Receiver<(Clash, ClashReply)>,
    pub current: Option<(Clash, ClashReply)>,
    pub all: bool,
    pub copy: bool,
}

/// How many changes Undo remembers.
pub(super) const UNDO_MAX: usize = 30;

impl Ops {
    /// A window of ours is up and owns the keyboard.
    pub(super) fn modal(&self) -> bool {
        self.confirm.is_some() || self.rename.is_some() || self.batch.is_some() || self.clash.as_ref().is_some_and(|c| c.current.is_some())
    }

    /// Start recording the steps of the operation about to run, for Undo.
    pub(super) fn record(&mut self) -> Arc<Mutex<Vec<Step>>> {
        let r = Arc::new(Mutex::new(Vec::new()));
        self.recording = Some(r.clone());
        r
    }

    /// The name of what Ctrl+Y would redo.
    pub(super) fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|u| u.label.as_str())
    }

    /// The name of what Ctrl+Z would undo.
    pub(super) fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|u| u.label.as_str())
    }
}

fn shortcut(ctx: &egui::Context, mods: egui::Modifiers, key: egui::Key) -> String {
    ctx.format_shortcut(&egui::KeyboardShortcut::new(mods, key))
}

/// A menu item with its shortcut on the right; true when clicked.
fn item(ui: &mut egui::Ui, enabled: bool, text: impl Into<egui::WidgetText>, keys: &str) -> bool {
    ui.add_enabled(enabled, egui::Button::new(super::style::mi(text)).shortcut_text(keys)).clicked()
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
        if info.in_browser {
            let many = if info.count > 1 { format!(" {} Items", info.count) } else { String::new() };
            set(item(ui, true, format!("Copy{many}"), &k(M::COMMAND, Key::C)), TreeAction::CopyItems);
            set(item(ui, can_change && !info.is_root, format!("Cut{many}"), &k(M::COMMAND, Key::X)), TreeAction::CutItems);
            if info.is_dir {
                set(item(ui, can_change && info.can_paste, "Paste Into Folder", ""), TreeAction::Paste);
                set(item(ui, true, "Open in New Tab", &k(M::COMMAND, Key::T)), TreeAction::NewTab);
            }
            set(item(ui, can_change && !info.is_root && info.count == 1, "Rename…", "F2"), TreeAction::Rename);
            set(item(ui, can_change, "New Folder", &k(M::COMMAND | M::SHIFT, Key::N)), TreeAction::NewFolder);
            ui.separator();
        }
        set(item(ui, true, "Copy Path", &k(M::COMMAND | M::SHIFT, Key::C)), TreeAction::CopyPath);
        let reveal = if cfg!(windows) { format!("Select in {}…", actions::FILE_MANAGER) } else { format!("Show in {}…", actions::FILE_MANAGER) };
        set(item(ui, true, reveal, &k(M::COMMAND, Key::E)), TreeAction::Reveal);
        for (i, (label, t)) in actions::terminals().iter().enumerate() {
            let key = if i == 0 { Key::P } else { Key::S };
            set(item(ui, true, *label, &k(M::COMMAND, key)), TreeAction::Terminal(*t));
        }
        set(item(ui, can_change, "Refresh Selected", &k(M::NONE, Key::F5)), TreeAction::Refresh);
        set(item(ui, true, "Properties", &k(M::ALT, Key::Enter)), TreeAction::Properties);
        if info.known && info.count <= 1 {
            set(item(ui, true, "What Is This?", ""), TreeAction::Explain);
        }
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
        if info.cloud {
            ui.separator();
            set(item(ui, can_change, crate::cloudops::Action::FreeUp.label(), ""), TreeAction::FreeUp);
            set(item(ui, can_change, crate::cloudops::Action::Keep.label(), ""), TreeAction::KeepHere);
        }
        pick
    }

    /// The extension list's right-click menu.
    pub(super) fn ext_menu(ui: &mut egui::Ui, ext: &str, highlighted: bool) -> Option<ExtAction> {
        let mut pick = None;
        ui.set_min_width(220.0);
        let name = if ext.is_empty() { "files without an extension".to_string() } else { format!(".{ext} files") };
        if ui.add(egui::Button::new(crate::app::style::mi(egui::RichText::new(format!("Find All {name}")).strong()))).clicked() {
            pick = Some(ExtAction::Find);
        }
        if ui.add(egui::Button::new(crate::app::style::mi(if highlighted { "Clear Highlight" } else { "Highlight in Map" }))).clicked() {
            pick = Some(ExtAction::Highlight);
        }
        ui.separator();
        if ui.add_enabled(!ext.is_empty(), egui::Button::new(crate::app::style::mi("Copy Extension"))).clicked() {
            pick = Some(ExtAction::Copy);
        }
        let ex = ui
            .add_enabled(!ext.is_empty(), egui::Button::new(crate::app::style::mi("Add to Exclusion Filter")))
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
            count: 1,
            can_paste: false,
            cloud: super::cloud::manageable(n),
            known: crate::known::about(&self.full_path(path)).is_some(),
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
        // In the file manager, Enter opens a folder there, like Explorer.
        if self.browser_has_keys() && info.is_dir && ctx.input_mut(|i| i.consume_key(M::NONE, Key::Enter)) {
            self.browse_into(sel);
            return;
        }
        let action = ctx.input_mut(|i| {
            // Ctrl+C, Ctrl+X and Ctrl+V reach egui as Copy, Cut and Paste events rather than key
            // presses (and Paste only when the clipboard holds text). With Shift, Ctrl+C copies
            // the path; on Windows Shift+Delete arrives as Cut.
            let copy = i.events.iter().any(|e| matches!(e, egui::Event::Copy)) || i.consume_key(M::CTRL, Key::Insert);
            let cut = i.events.iter().any(|e| matches!(e, egui::Event::Cut));
            if cut {
                return Some(if i.modifiers.shift && !i.modifiers.command { TreeAction::Delete } else { TreeAction::CutItems });
            }
            if copy && i.modifiers.shift {
                return Some(TreeAction::CopyPath);
            }
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
                return Some(TreeAction::CopyItems);
            }
            if i.consume_key(M::NONE, Key::F2) {
                return Some(TreeAction::Rename);
            }
            keys.into_iter().find(|&(m, k, _)| i.consume_key(m, k)).map(|(_, _, a)| a)
        });
        let Some(a) = action else { return };
        // The same rules as the menu, which greys these out.
        let allowed = match a {
            TreeAction::Delete | TreeAction::Recycle | TreeAction::MoveTo | TreeAction::CutItems | TreeAction::Rename => !info.busy && !info.is_root,
            TreeAction::Refresh => !info.busy,
            TreeAction::Hash => !info.is_dir,
            _ => true,
        };
        if allowed {
            self.browser_action(a, sel);
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
                self.ctx_copy = Some(disk.shown());
                self.ops.notice = Some((format!("Copied {}", disk.display()), false));
                Ok(())
            }
            TreeAction::Reveal => actions::reveal(&disk),
            TreeAction::Terminal(t) => actions::terminal(&disk, t),
            TreeAction::Explain => {
                self.explain_path(disk.clone());
                Ok(())
            }
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
            TreeAction::FreeUp | TreeAction::KeepHere => {
                self.cloud_manage(cloud_action(a), vec![(disk, n)]);
                Ok(())
            }
            TreeAction::Exclude => {
                let (list, entry) = if n.is_dir {
                    (&mut self.settings.exclude_folders, disk.shown())
                } else {
                    (&mut self.settings.exclude_files, n.name.to_string())
                };
                add_line(list, &entry);
                self.ops.notice = Some((format!("Added {entry} to the exclusion filter. Rescan to apply it."), false));
                Ok(())
            }
            TreeAction::Delete | TreeAction::Recycle | TreeAction::Empty => {
                self.ops.confirm = Some(Confirm { action: a, paths: vec![disk], is_dir: n.is_dir, size: n.size, files: n.files, sized: true });
                Ok(())
            }
            TreeAction::Refresh if path.is_empty() => {
                self.path_input = self.scanned_path.shown();
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
            TreeAction::Rename | TreeAction::CopyItems | TreeAction::CutItems | TreeAction::Paste | TreeAction::NewFolder | TreeAction::NewTab => {
                self.items_action(a, vec![path]);
                Ok(())
            }
            TreeAction::MoveTo => {
                let start = disk.parent().map(Path::to_path_buf).unwrap_or_default();
                if let Some(dest) = rfd::FileDialog::new().set_title("Move to folder").set_directory(start).pick_folder() {
                    self.transfer(vec![disk], dest, false);
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
                if self.highlight_ext.as_ref() != Some(&ext) && !self.premium(crate::licence::Feature::Filters) {
                    return;
                }
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
    pub(super) fn run_op(&mut self, label: String, work: impl FnOnce() -> (Result<Option<String>, String>, Vec<PathBuf>) + Send + 'static) {
        self.run_op_select(label, move || {
            let (r, affected) = work();
            (r, affected, Vec::new())
        });
    }

    /// `run_op` for work that also names an item to select when it's done.
    pub(super) fn run_op_select(
        &mut self,
        label: String,
        work: impl FnOnce() -> (Result<Option<String>, String>, Vec<PathBuf>, Vec<PathBuf>) + Send + 'static,
    ) {
        if self.ops.job.is_some() {
            return;
        }
        let opts = self.options_for(self.scanned_with.as_ref().unwrap_or(&self.settings));
        // Only what's inside the analysis is read again for the tree; the Files lists re-read
        // their folders themselves.
        let root = self.tree.is_some().then(|| self.scanned_path.clone());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (r, affected, select) = work();
            let inside = |p: &PathBuf| root.as_ref().is_some_and(|r| p.starts_with(r));
            let updates = affected.into_iter().filter(inside).map(|p| {
                let n = reread(&p, &opts);
                (p, n)
            });
            let updates = updates.collect();
            let (error, note) = match r {
                Ok(note) => (None, note),
                Err(e) => (Some(e), None),
            };
            let _ = tx.send(OpDone { error, note, updates, select });
        });
        self.ops.notice = None;
        self.ops.undo_fresh = false;
        self.ops.job = Some(OpJob { label, rx, ctl: None });
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
                    self.ops.clash = None;
                    if let Some(rec) = self.ops.recording.take() {
                        let steps = std::mem::take(&mut *rec.lock().unwrap());
                        self.marks_follow(&steps);
                        let undoing = self.ops.undoing.take();
                        if let Some((to_redo, label)) = undoing {
                            // An Undo's steps can be redone, and a Redo's undone again.
                            if !steps.is_empty() {
                                let stack = if to_redo { &mut self.ops.redo } else { &mut self.ops.undo };
                                stack.push(UndoEntry { label, steps });
                            }
                        } else if !steps.is_empty() {
                            self.ops.redo.clear();
                            self.ops.undo.push(UndoEntry { label: transfer::describe(&steps), steps });
                            if self.ops.undo.len() > UNDO_MAX {
                                self.ops.undo.remove(0);
                            }
                            self.ops.undo_fresh = true;
                        }
                    }
                    self.apply_updates(done.updates);
                    self.ops.notice = done.error.map(|e| (e, true)).or(done.note.map(|n| (n, false)));
                    // A renamed item, a new folder or pasted items get selected.
                    let mut then = done.select;
                    then.extend(self.ops.then_select.take());
                    let rename = std::mem::take(&mut self.ops.then_rename);
                    let in_files = self.files.prefs.workspace != files::Workspace::Analysis;
                    if let Some(p) = then.last().cloned()
                        && let Some(tp) = self.tree_path_of(&p)
                    {
                        self.select(tp);
                        if rename && !in_files {
                            self.start_rename(p);
                        }
                    }
                    self.files_after_op(&then, rename && in_files);
                    self.browser_pick_later(then);
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.ops.job = None;
                    self.ops.clash = None;
                    self.ops.recording = None;
                }
            }
        }
        // The next copy or move in the queue.
        if self.ops.job.is_none()
            && let Some(q) = self.ops.queued.pop_front()
        {
            let notice = self.ops.notice.clone();
            self.transfer(q.sources, q.dest, q.copy);
            // Keep the last one's result on show until this one finishes.
            if self.ops.notice.is_none() {
                self.ops.notice = notice;
            }
        }
        self.clash_window(ctx);
        self.confirm_window(ctx);
        self.rename_window(ctx);
        self.batch_rename_window(ctx);
        self.hash_window(ctx);
        self.props_window(ctx);
        self.type_files_window(ctx);
    }

    /// The running copy or move in the status bar: how far it's got, Pause or Resume, Stop, and
    /// what's waiting. Laid out right to left.
    pub(super) fn transfer_strip(&mut self, ui: &mut egui::Ui) {
        let Some(job) = &self.ops.job else { return };
        let Some(ctl) = job.ctl.clone() else { return };
        let label = job.label.clone();
        let paused = ctl.paused.load(Ordering::Relaxed);
        let stopping = ctl.stop.load(Ordering::Relaxed);
        if !self.ops.queued.is_empty() {
            let n = self.ops.queued.len();
            ui.menu_button(format!("+{n} waiting"), |ui| {
                for (k, q) in self.ops.queued.iter().enumerate() {
                    let what = if q.sources.len() == 1 { q.sources[0].file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default() } else { format!("{} items", q.sources.len()) };
                    ui.label(format!("{}. {} {what} to {}", k + 1, if q.copy { "Copy" } else { "Move" }, q.dest.display()));
                }
                ui.separator();
                if ui.button("Cancel the Waiting Ones").clicked() {
                    self.ops.queued.clear();
                    ui.close();
                }
            })
            .response
            .on_hover_text("Copies and moves run one at a time, in the order you asked for them");
        }
        if ui.add_enabled(!stopping, egui::Button::new("Stop").small()).on_hover_text("Stop after the file being copied; what's done so far stays (Undo removes it)").clicked() {
            ctl.stop.store(true, Ordering::Relaxed);
            ctl.paused.store(false, Ordering::Relaxed);
        }
        if ui.add_enabled(!stopping, egui::Button::new(if paused { "Resume" } else { "Pause" }).small()).clicked() {
            ctl.paused.store(!paused, Ordering::Relaxed);
        }
        let (bt, bd) = (ctl.bytes_total.load(Ordering::Relaxed), ctl.bytes_done.load(Ordering::Relaxed));
        let (it, id) = (ctl.items_total.load(Ordering::Relaxed), ctl.items_done.load(Ordering::Relaxed));
        let frac = if bt > 0 { bd as f32 / bt as f32 } else if it > 0 { id as f32 / it as f32 } else { 0.0 };
        let text = if ctl.counting.load(Ordering::Relaxed) {
            "Counting…".to_string()
        } else if bt > 0 {
            format!("{} of {}", scan::human(bd.min(bt)), scan::human(bt))
        } else {
            format!("{} of {}", id.min(it), it)
        };
        let text = if stopping { "Stopping…".to_string() } else if paused { format!("Paused · {text}") } else { text };
        ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)).desired_width(170.0).text(egui::RichText::new(text).size(11.0)));
        let current = ctl.current.lock().map(|c| c.clone()).unwrap_or_default();
        let tip = if current.is_empty() { label.clone() } else { format!("{label}\nNow: {current}") };
        ui.label(egui::RichText::new(&label).size(12.0)).on_hover_text(tip);
        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }

    /// Undo the last change (Ctrl+Z), in the background.
    pub(super) fn undo_last(&mut self) {
        self.undo_or_redo(false);
    }

    pub(super) fn redo_last(&mut self) {
        self.undo_or_redo(true);
    }

    fn undo_or_redo(&mut self, redo: bool) {
        if self.ops.job.is_some() {
            self.ops.notice = Some(("Another file operation is still running.".into(), true));
            return;
        }
        let stack = if redo { &mut self.ops.redo } else { &mut self.ops.undo };
        let Some(e) = stack.pop() else {
            self.ops.notice = Some((if redo { "There's nothing to redo." } else { "There's nothing to undo." }.into(), false));
            return;
        };
        let label = e.label;
        let steps = e.steps;
        let keep = label.clone();
        let rec = self.ops.record();
        let verb = if redo { "Redo" } else { "Undo" };
        self.run_op(format!("{verb}ing {label}"), move || {
            let (errors, touched, inverse) = transfer::undo(&steps);
            *rec.lock().unwrap() = inverse;
            let done = if redo { "Redid" } else { "Undid" };
            let r = if errors.is_empty() { Ok(Some(format!("{done} {label}."))) } else { Err(format!("{verb} {label}: {}", errors.join(" "))) };
            (r, touched)
        });
        self.ops.undoing = Some((!redo, keep));
    }

    /// Asks what to do when a copy or move finds a name taken. The copy waits meanwhile.
    pub(super) fn clash_window(&mut self, ctx: &egui::Context) {
        let Some(c) = &mut self.ops.clash else { return };
        if c.current.is_none() {
            match c.rx.try_recv() {
                Ok(x) => c.current = Some(x),
                Err(_) => return,
            }
        }
        let Some((clash, _)) = &c.current else { return };
        let name = clash.dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let folder = clash.dest.parent().map(|p| p.shown()).unwrap_or_default();
        let both_dirs = clash.src_dir && clash.dest_dir;
        let what = |d: bool| if d { "folder" } else { "file" };
        let mut pick = None;
        egui::Modal::new(egui::Id::new("clash")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(if both_dirs { "This folder is already there" } else { "Replace or skip?" });
            ui.add_space(6.0);
            ui.label(format!("{folder} already has a {} called {name}.", what(clash.dest_dir)));
            if let Some(from) = clash.src.parent() {
                ui.weak(format!("{} from {}", if c.copy { "Copying" } else { "Moving" }, from.display()));
            }
            ui.add_space(6.0);
            let when = |t: Option<std::time::SystemTime>| {
                t.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| fmt_time(d.as_secs() as i64)).unwrap_or_default()
            };
            egui::Grid::new("clash-grid").num_columns(3).spacing([14.0, 4.0]).show(ui, |ui| {
                let newer = |a: Option<std::time::SystemTime>, b: Option<std::time::SystemTime>| matches!((a, b), (Some(a), Some(b)) if a > b);
                ui.weak(if c.copy { "Copying" } else { "Moving" });
                ui.label(if clash.src_dir { "Folder".to_string() } else { scan::human(clash.src_size) });
                ui.label(format!("{}{}", when(clash.src_modified), if newer(clash.src_modified, clash.dest_modified) { "  (newer)" } else { "" }));
                ui.end_row();
                ui.weak("Already there");
                ui.label(if clash.dest_dir { "Folder".to_string() } else { scan::human(clash.dest_size) });
                ui.label(format!("{}{}", when(clash.dest_modified), if newer(clash.dest_modified, clash.src_modified) { "  (newer)" } else { "" }));
                ui.end_row();
            });
            ui.add_space(8.0);
            let bin = actions::BIN;
            ui.horizontal_wrapped(|ui| {
                if both_dirs && ui.button("Merge").on_hover_text("Put the contents together, asking about any files with the same name").clicked() {
                    pick = Some(Choice::Merge);
                }
                if ui.button("Replace").on_hover_text(format!("The {} that's there goes to the {bin}, and this one takes its place", what(clash.dest_dir))).clicked() {
                    pick = Some(Choice::Replace);
                }
                if ui.button("Keep Both").on_hover_text("This one gets a new name, like \"photo 2.jpg\"").clicked() {
                    pick = Some(Choice::KeepBoth);
                }
                if ui.button("Skip").on_hover_text("Leave this one where it is").clicked() {
                    pick = Some(Choice::Skip);
                }
                if ui.button("Stop").on_hover_text(format!("{} nothing more; what's done so far stays (Undo puts it back)", if c.copy { "Copy" } else { "Move" })).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    pick = Some(Choice::Stop);
                }
            });
            ui.add_space(4.0);
            ui.checkbox(&mut c.all, format!("Do the same for other {}s that clash", if both_dirs { "folder" } else { "file" }));
        });
        if let Some(choice) = pick {
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            });
            let (_, reply) = c.current.take().unwrap();
            let _ = reply.send((choice, c.all && choice != Choice::Stop));
            c.all = false;
        }
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        let Some(c) = &self.ops.confirm else { return };
        let many = c.paths.len() > 1;
        let what = if many { format!("{} items", c.paths.len()) } else if c.is_dir { "folder".to_string() } else { "file".to_string() };
        let (title, verb, warn) = match c.action {
            TreeAction::Delete => ("Delete permanently", "Delete", format!("This {what} will be deleted for good. It won't go to the {}.", actions::BIN)),
            TreeAction::Recycle => (if cfg!(windows) { "Delete to Recycle Bin" } else { "Move to Trash" }, "Delete", format!("This {what} will be moved to the {}.", actions::BIN)),
            _ => ("Empty folder", "Empty Folder", format!("Everything inside this folder will be deleted for good. It won't go to the {}.", actions::BIN)),
        };
        let warn = if many { warn.replacen("This ", "These ", 1) } else { warn };
        let amount = if !c.sized {
            String::new()
        } else if c.is_dir || many {
            format!("{} in {}", scan::human(c.size), files_text(c.files))
        } else {
            scan::human(c.size)
        };
        let mut go = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("confirm-delete")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading(title);
            ui.add_space(6.0);
            for p in c.paths.iter().take(8) {
                ui.label(egui::RichText::new(p.shown()).strong());
            }
            if c.paths.len() > 8 {
                ui.label(format!("and {} more", c.paths.len() - 8));
            }
            if !amount.is_empty() {
                ui.label(&amount);
            }
            ui.add_space(6.0);
            ui.colored_label(if c.action == TreeAction::Recycle { ui.visuals().text_color() } else { ui.visuals().warn_fg_color }, warn);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                go = ui.button(verb).clicked();
                cancel = ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if cancel || go {
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            });
        }
        if cancel {
            self.ops.confirm = None;
        } else if go {
            let c = self.ops.confirm.take().unwrap();
            let name = match &c.paths[..] {
                [one] => one.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                many => format!("{} items", many.len()),
            };
            let paths = c.paths;
            match c.action {
                TreeAction::Delete => self.run_op(format!("Deleting {name}"), move || {
                    let r = paths.iter().try_for_each(|p| actions::delete(p)).map(|_| None).map_err(|e| format!("Couldn't delete everything: {e}"));
                    (r, paths)
                }),
                TreeAction::Recycle => {
                    let rec = self.ops.record();
                    self.run_op(format!("Moving {name} to the {}", actions::BIN), move || {
                        // Each item goes to the bin, noting where it went so Undo can put it back.
                        let mut steps = Vec::new();
                        let mut r = Ok(None);
                        for p in &paths {
                            match actions::trash_keep(p) {
                                Ok(receipt) => steps.push(Step::Trashed { original: p.clone(), receipt }),
                                Err(e) => {
                                    r = Err(e);
                                    break;
                                }
                            }
                        }
                        *rec.lock().unwrap() = steps;
                        (r, paths)
                    })
                }
                _ => self.run_op(format!("Emptying {name}"), move || {
                    let r = paths.iter().try_for_each(|p| actions::empty_folder(p)).map(|_| None).map_err(|e| format!("Couldn't delete everything: {e}"));
                    (r, paths)
                }),
            }
        }
    }

    /// The Rename box: Enter renames, Escape cancels.
    fn rename_window(&mut self, ctx: &egui::Context) {
        let Some(r) = &mut self.ops.rename else { return };
        let mut go = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("rename")).show(ctx, |ui| {
            ui.set_min_width(380.0);
            ui.heading("Rename");
            ui.add_space(6.0);
            ui.weak(r.path.parent().map(|p| p.shown()).unwrap_or_default());
            let resp = ui.add(egui::TextEdit::singleline(&mut r.text).desired_width(360.0).id(egui::Id::new("rename-box")));
            if r.fresh {
                r.fresh = false;
                resp.request_focus();
                // Select the name without its extension, as Finder and Explorer do.
                let is_dir = r.path.is_dir();
                let stem = match r.text.rfind('.') {
                    Some(i) if i > 0 && !is_dir => i,
                    _ => r.text.len(),
                };
                let chars = r.text[..stem].chars().count();
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(chars))));
                    state.store(ui.ctx(), resp.id);
                }
            }
            go = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if let Some(e) = &r.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                go |= ui.button("Rename").clicked();
                cancel = ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        // The Enter or Escape that closed the box mustn't also reach the list behind it.
        if cancel || go {
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            });
        }
        if cancel {
            self.ops.rename = None;
        } else if go {
            let r = self.ops.rename.as_ref().unwrap();
            let (path, text) = (r.path.clone(), r.text.trim().to_string());
            if path.file_name().is_some_and(|n| n.to_string_lossy() == text) {
                self.ops.rename = None;
                return;
            }
            // Check what we can before going to disk, so mistakes are shown in the box.
            if text.is_empty() || text.contains('/') || (cfg!(windows) && text.contains(['\\', ':', '*', '?', '"', '<', '>', '|'])) {
                self.ops.rename.as_mut().unwrap().error = Some(if cfg!(windows) { "A name can't be empty or contain \\ / : * ? \" < > |".into() } else { "A name can't be empty or contain /.".into() });
                return;
            }
            self.ops.rename = None;
            let dest = path.with_file_name(&text);
            self.ops.then_select = Some(dest.clone());
            let rec = self.ops.record();
            self.run_op(format!("Renaming {}", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()), move || {
                match actions::rename(&path, &text) {
                    Ok(new) => {
                        rec.lock().unwrap().push(Step::Moved { from: path.clone(), to: new.clone() });
                        (Ok(None), vec![path, new])
                    }
                    Err(e) => (Err(e), vec![]),
                }
            });
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
            ui.label(h.path.shown());
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
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.shown());
                ui.label("Name");
                ui.strong(name);
                ui.end_row();
                ui.label("Location");
                ui.label(path.parent().map(|p| p.shown()).unwrap_or_default());
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
                        .text(pos2(rect.left() + 4.0, cy), Align2::LEFT_CENTER, rel.shown(), font.clone(), tc);
                    ui.painter().text(pos2(rect.right() - 4.0, cy), Align2::RIGHT_CENTER, scan::human(*size), font, tc);
                    if resp.on_hover_text(root.join(&rel).shown()).clicked() {
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
    pub(super) fn apply_updates(&mut self, updates: Vec<(PathBuf, Option<Node>)>) {
        if updates.is_empty() || self.tree.is_none() {
            return;
        }
        let root = self.scanned_path.clone();
        self.rework_tree(false, |tree| {
            for (disk, new) in updates {
                let Ok(rel) = disk.strip_prefix(&root) else { continue };
                let names: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                match new {
                    _ if !names.is_empty() => {
                        replace_at(tree, &names, new);
                    }
                    Some(mut n) => {
                        n.name = tree.name.clone();
                        *tree = n;
                    }
                    None => {
                        tree.children.clear();
                        resum(tree);
                    }
                }
            }
        });
    }

    /// Change the tree with `f`, keeping the user's place: zoom, selection and open folders are
    /// remembered by their paths on disk across it. With `same_items` (only sizes and order
    /// changed, as when switching the size shown), duplicate results are kept too.
    pub(super) fn rework_tree(&mut self, same_items: bool, f: impl FnOnce(&mut Node)) {
        if self.tree.is_none() {
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
        f(&mut tree);
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
        let style = self.colors_style;
        for (i, e) in self.exts.iter().enumerate() {
            self.colors.entry(e.ext.clone()).or_insert_with(|| ext_color(&e.ext, i, style));
        }
        self.rows = None;
        self.filter_stats = None;
        self.hits = None;
        self.hist = None;
        self.sun = None;
        if same_items && self.dup_job.is_none() {
            // Same files, new positions: follow each duplicate to where it is now.
            let mut groups = std::mem::take(&mut self.dup_groups);
            for g in &mut groups {
                for c in &mut g.files {
                    if let Some(p) = self.tree_path_of(&c.path) {
                        c.tree_path = p;
                    }
                }
            }
            self.dup_set = Arc::new(groups.iter().flat_map(|g| g.files.iter().map(|f| f.tree_path.clone())).collect());
            self.dup_groups = groups;
        } else {
            // Duplicate results point at old tree positions.
            if let Some(j) = self.dup_job.take() {
                j.progress.cancel.store(true, Ordering::Relaxed);
            }
            self.dup_groups.clear();
            self.dup_rows.clear();
            self.dup_set = Arc::new(HashSet::new());
            self.dupes_only = false;
            self.dup_done = false;
        }
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
    if crate::fsys::remote::is_remote(path) {
        let m = crate::fsys::remote::meta(path).ok()?;
        return Some(if m.is_dir && !m.is_link {
            scan::scan_remote(path, &Progress::default())
        } else {
            Node::plain(scan::root_name(path), m.len, m.modified)
        });
    }
    let md = std::fs::symlink_metadata(path).ok()?;
    if md.is_dir() {
        Some(scan::scan(path, opts, &Progress::default(), &scan::new_live(path)))
    } else if md.is_file() {
        let modified = md.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
        let (alloc, mut flags) = scan::on_disk(&md, || path.to_path_buf(), scan::cluster_of(path));
        flags |= crate::cloud::root_of(&crate::cloud::known(), path).map_or(0, |r| r.bits());
        Some(Node::file(scan::root_name(path), md.len(), alloc, flags, modified, opts.metric))
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
    scan::sort_children(&mut n.children);
    n.resum();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> Node {
        Node { name: name.into(), size, logical: size, alloc: size, flags: 0, files: 1, modified: size as i64, is_dir: false, children: Vec::new() }
    }

    fn dir(name: &str, mut children: Vec<Node>) -> Node {
        let mut n = Node { name: name.into(), size: 0, logical: 0, alloc: 0, flags: 0, files: 0, modified: 0, is_dir: true, children: Vec::new() };
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

/// The cloud command a menu action stands for.
pub(super) fn cloud_action(a: TreeAction) -> crate::cloudops::Action {
    if a == TreeAction::FreeUp { crate::cloudops::Action::FreeUp } else { crate::cloudops::Action::Keep }
}
