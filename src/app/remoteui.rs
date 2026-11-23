//! The Remote workspace: the File Manager with its own desk (panes, tabs, split, dock), a
//! sidebar of saved sites, a quick-connect bar and the Site Manager.
//!
//! Remote is not a second file manager. It is the Files workspace's code with a different
//! desk swapped in: `use_desk` exchanges the panes and the layout settings when the user moves
//! between Files and Remote, so every view, menu, shortcut, preview, drag and file operation
//! is shared. Favourites, saved sites, marks and the rest of `FilesPrefs` belong to both.
//!
//! See `docs/design/dirmap-2.3-remote.md`.

use super::files::{FAction, FPicks, Files, FilesPrefs, Pane, PaneSave, Split, Workspace};
use super::netui::{ConnectBox, SavedSite};
use super::*;
use crate::fsys::history::Location;
use crate::fsys::remote::{self, Scheme};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Which desk is in `Files` now (the other is in `RemoteState::stash`).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(super) enum DeskKind {
    #[default]
    Files,
    Remote,
}

/// The part of the File Manager that is about layout: the panes and how they're arranged.
pub(super) struct Desk {
    panes: Vec<Pane>,
    focus: usize,
    closed: Vec<Location>,
    split: Split,
    last_split: Split,
    split_ratio: f32,
    second: PaneSave,
    preview: bool,
    preview_width: f32,
    dock: super::dock::DockPrefs,
}

/// A desk as it's remembered between runs (`FilesPrefs::remote_desk`).
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct DeskSave {
    pub first: PaneSave,
    pub second: PaneSave,
    pub two: bool,
    pub split: Split,
    pub last_split: Split,
    pub split_ratio: f32,
    pub preview: bool,
    pub preview_width: f32,
    pub dock: super::dock::DockPrefs,
    pub transfers: super::transfers::TransferPrefs,
}

/// The Remote workspace's own state, kept in `Files`.
#[derive(Default)]
pub(super) struct RemoteState {
    pub(super) active: DeskKind,
    pub(super) stash: Option<Desk>,
    /// What's typed in the quick-connect bar.
    pub(super) quick: String,
    /// The Site Manager, while it's open.
    pub(super) manager: Option<SiteManager>,
    /// The connection log window: which site.
    pub(super) log: Option<String>,
    /// The Sites section's search.
    pub(super) search: String,
    /// The pane a connection being made opens in (clicks on its questions can move the focus).
    pub(super) target: Option<usize>,
    /// Properties of an item on a server, while the window is open.
    pub(super) props: Option<RemoteProps>,
    /// Copies and moves to and from servers.
    pub(super) queue: super::transfers::Queue,
}

/// The Properties window for an item on a server: what the server says about it, and its
/// permissions to change.
pub(super) struct RemoteProps {
    path: PathBuf,
    rx: Option<mpsc::Receiver<Result<(remote::Meta, Option<String>), String>>>,
    meta: Option<remote::Meta>,
    link: Option<String>,
    error: Option<String>,
    /// The permissions as edited, and as typed in octal.
    mode: u32,
    octal: String,
    /// Apply to everything inside a folder too.
    deep: bool,
    applying: Option<mpsc::Receiver<Result<u64, String>>>,
    done: Option<Result<u64, String>>,
}

impl RemoteProps {
    fn new(path: PathBuf) -> RemoteProps {
        let (tx, rx) = mpsc::channel();
        let p = path.clone();
        std::thread::spawn(move || {
            let r = remote::meta(&p).map_err(|e| remote::describe(&e)).map(|m| {
                let link = if m.is_link { remote::link_target(&p).ok() } else { None };
                (m, link)
            });
            let _ = tx.send(r);
        });
        RemoteProps { path, rx: Some(rx), meta: None, link: None, error: None, mode: 0, octal: String::new(), deep: false, applying: None, done: None }
    }
}

/// `s` quoted for a POSIX shell.
#[cfg(any(target_os = "macos", test))]
fn sh_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "@%_+=:,./-".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Open the system's terminal running `ssh` to the server, in the folder (or a file's folder).
fn ssh_terminal(path: &Path) -> Result<(), String> {
    let caps = remote::caps(path);
    if !caps.ssh {
        return Err("A terminal opens only on SFTP (SSH) servers.".into());
    }
    // A file: its folder. (One request; this runs on a worker.)
    let dir = match remote::meta(path) {
        Ok(m) if !m.is_dir => remote::parent(path).unwrap_or_else(|| path.to_path_buf()),
        _ => path.to_path_buf(),
    };
    let args = remote::ssh_args(&dir).ok_or("That isn't an SFTP address.")?;
    launch_ssh(&args)
}

#[cfg(windows)]
fn launch_ssh(args: &[String]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x10;
    std::process::Command::new("ssh.exe").args(args).creation_flags(CREATE_NEW_CONSOLE).spawn().map(|_| ()).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "Windows' SSH client (ssh.exe) isn't installed. Add \"OpenSSH Client\" in Settings > System > Optional features.".to_string()
        } else {
            format!("Couldn't start ssh: {e}")
        }
    })
}

#[cfg(target_os = "macos")]
fn launch_ssh(args: &[String]) -> Result<(), String> {
    let line = std::iter::once("ssh".to_string()).chain(args.iter().map(|a| sh_quote(a))).collect::<Vec<_>>().join(" ");
    let script = format!("tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell", line.replace('\\', "\\\\").replace('"', "\\\""));
    std::process::Command::new("osascript").arg("-e").arg(script).spawn().map(|_| ()).map_err(|e| format!("Couldn't open Terminal: {e}"))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn launch_ssh(args: &[String]) -> Result<(), String> {
    let mut tried: Vec<(String, Vec<&str>)> = Vec::new();
    if let Ok(t) = std::env::var("TERMINAL") {
        tried.push((t, vec!["-e"]));
    }
    for (t, run) in [
        ("x-terminal-emulator", vec!["-e"]),
        ("gnome-terminal", vec!["--"]),
        ("konsole", vec!["-e"]),
        ("xfce4-terminal", vec!["-x"]),
        ("kitty", vec![]),
        ("alacritty", vec!["-e"]),
        ("wezterm", vec!["start", "--"]),
        ("foot", vec![]),
        ("xterm", vec!["-e"]),
    ] {
        tried.push((t.to_string(), run));
    }
    for (t, run) in &tried {
        if std::process::Command::new(t).args(run).arg("ssh").args(args).spawn().is_ok() {
            return Ok(());
        }
    }
    Err("Couldn't find a terminal to open (set $TERMINAL to choose one).".into())
}

/// Permission bits as `rwxr-xr-x`.
fn rwx(mode: u32) -> String {
    let mut s = String::new();
    for shift in [6, 3, 0] {
        let b = (mode >> shift) & 7;
        s.push(if b & 4 != 0 { 'r' } else { '-' });
        s.push(if b & 2 != 0 { 'w' } else { '-' });
        s.push(if b & 1 != 0 { 'x' } else { '-' });
    }
    s
}

impl Desk {
    fn from_save(s: &DeskSave, fallback: &FilesPrefs) -> Desk {
        let first = if s.first.tabs.is_empty() { None } else { Some(&s.first) };
        let mut panes = Vec::new();
        match first {
            Some(f) => panes.push(Pane::new(&f.tabs, f.active, f.sort, f.reversed, f.view)),
            None => {
                // First time: this computer on the left, the most recently used site on the right.
                let left = fallback.tabs.get(fallback.active).cloned().filter(|l| l.dir().is_none_or(|d| !remote::is_remote(d)));
                let left = left.or_else(|| crate::fsys::places::home().map(Location::Dir)).unwrap_or(Location::Computer);
                panes.push(Pane::new(&[left], 0, fallback.sort, fallback.reversed, fallback.view));
            }
        }
        let two = s.two || first.is_none();
        if two {
            let right = if s.second.tabs.is_empty() {
                let recent = fallback.sites.iter().filter(|s| s.path().is_some()).max_by_key(|s| s.last_used);
                vec![recent.and_then(SavedSite::path).map(Location::Dir).unwrap_or(Location::Computer)]
            } else {
                s.second.tabs.clone()
            };
            panes.push(Pane::new(&right, s.second.active, s.second.sort, s.second.reversed, s.second.view));
        }
        let split = if two { if s.split == Split::Single { Split::Vertical } else { s.split } } else { Split::Single };
        Desk {
            panes,
            focus: if two { 1 } else { 0 },
            closed: Vec::new(),
            split,
            last_split: if s.last_split == Split::Single { Split::Vertical } else { s.last_split },
            split_ratio: if s.split_ratio > 0.05 && s.split_ratio < 0.95 { s.split_ratio } else { 0.5 },
            second: s.second.clone(),
            preview: s.preview,
            preview_width: if s.preview_width > 50.0 { s.preview_width } else { fallback.preview_width },
            dock: if first.is_none() { DockClosed::closed(&fallback.dock) } else { s.dock.clone() },
        }
    }

    fn to_save(&self) -> DeskSave {
        DeskSave {
            first: self.panes[0].save(),
            second: self.panes.get(1).map(Pane::save).unwrap_or_else(|| self.second.clone()),
            two: self.panes.len() > 1,
            split: self.split,
            last_split: self.last_split,
            split_ratio: self.split_ratio,
            preview: self.preview,
            preview_width: self.preview_width,
            dock: self.dock.clone(),
            transfers: Default::default(),
        }
    }
}

/// The dock starts closed in Remote.
struct DockClosed;
impl DockClosed {
    fn closed(d: &super::dock::DockPrefs) -> super::dock::DockPrefs {
        let mut d = d.clone();
        d.open = false;
        d
    }
}

/// Exchange the desk in `files` with `d`.
fn swap(files: &mut Files, d: &mut Desk) {
    std::mem::swap(&mut files.panes, &mut d.panes);
    std::mem::swap(&mut files.focus, &mut d.focus);
    std::mem::swap(&mut files.closed, &mut d.closed);
    let p = &mut files.prefs;
    std::mem::swap(&mut p.split, &mut d.split);
    std::mem::swap(&mut p.last_split, &mut d.last_split);
    std::mem::swap(&mut p.split_ratio, &mut d.split_ratio);
    std::mem::swap(&mut p.second, &mut d.second);
    std::mem::swap(&mut p.preview, &mut d.preview);
    std::mem::swap(&mut p.preview_width, &mut d.preview_width);
    std::mem::swap(&mut p.dock, &mut d.dock);
    files.focus = files.focus.min(files.panes.len().saturating_sub(1));
    files.path_edit = None;
    files.drag_hover_tab = None;
    for pane in &mut files.panes {
        pane.seen = None;
    }
}

/// Put the desk for `ws` in place (Storage Analysis keeps whichever was there). Cheap when
/// it's already there, so it's called every frame.
pub(super) fn use_desk(files: &mut Files, ws: Workspace) {
    let want = match ws {
        Workspace::Files => DeskKind::Files,
        Workspace::Remote => DeskKind::Remote,
        Workspace::Analysis => return,
    };
    if files.rws.active == want {
        return;
    }
    let mut other = match files.rws.stash.take() {
        Some(d) => d,
        None => Desk::from_save(&files.prefs.remote_desk, &files.prefs),
    };
    swap(files, &mut other);
    files.rws.stash = Some(other);
    files.rws.active = want;
}

/// Finish `Files::to_save`: the Files desk goes in the usual fields and the Remote desk in
/// `remote_desk`, whichever is on screen.
pub(super) fn finish_save(files: &Files, p: &mut FilesPrefs) {
    let transfers = files.rws.queue.prefs.clone().unwrap_or_else(|| p.remote_desk.transfers.clone());
    finish_desk(files, p);
    p.remote_desk.transfers = transfers;
}

fn finish_desk(files: &Files, p: &mut FilesPrefs) {
    let Some(stash) = &files.rws.stash else { return };
    match files.rws.active {
        DeskKind::Files => p.remote_desk = stash.to_save(),
        DeskKind::Remote => {
            // `p` was made from the Remote desk: move it to `remote_desk`, and put the Files
            // desk back where Files reads it.
            p.remote_desk = DeskSave {
                first: PaneSave { tabs: p.tabs.clone(), active: p.active, sort: p.sort, reversed: p.reversed, view: p.view },
                second: p.second.clone(),
                two: files.panes.len() > 1,
                split: p.split,
                last_split: p.last_split,
                split_ratio: p.split_ratio,
                preview: p.preview,
                preview_width: p.preview_width,
                dock: p.dock.clone(),
                transfers: Default::default(),
            };
            let f = stash.to_save();
            (p.tabs, p.active, p.sort, p.reversed, p.view) = (f.first.tabs, f.first.active, f.first.sort, f.first.reversed, f.first.view);
            p.second = f.second;
            (p.split, p.last_split, p.split_ratio) = (if f.two { f.split } else { Split::Single }, f.last_split, f.split_ratio);
            (p.preview, p.preview_width, p.dock) = (f.preview, f.preview_width, f.dock);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The Site Manager.

/// A site being edited, or a new one.
pub(super) struct SiteManager {
    /// The saved site the form shows (None: a new one, not saved yet).
    sel: Option<usize>,
    /// The address fields (shared with the Connect to Server box).
    form: ConnectBox,
    group: String,
    notes: String,
    favourite: bool,
    local_dir: String,
    search: String,
    /// A test running, and the last result.
    test: Option<mpsc::Receiver<Result<String, String>>>,
    tested: Option<Result<String, String>>,
    /// Delete asked for: confirm first.
    confirm_delete: bool,
    /// A group being renamed: (old, new).
    rename_group: Option<(String, String)>,
    error: Option<String>,
}

impl SiteManager {
    fn new(sel: Option<usize>, sites: &[SavedSite]) -> SiteManager {
        let mut m = SiteManager {
            sel: None,
            form: ConnectBox { kind: Some(Scheme::Sftp), save: true, ..Default::default() },
            group: String::new(),
            notes: String::new(),
            favourite: false,
            local_dir: String::new(),
            search: String::new(),
            test: None,
            tested: None,
            confirm_delete: false,
            rename_group: None,
            error: None,
        };
        if let Some(i) = sel.filter(|&i| i < sites.len()) {
            m.load(i, &sites[i]);
        }
        m
    }

    fn load(&mut self, i: usize, s: &SavedSite) {
        self.sel = Some(i);
        self.form = ConnectBox::editing(i, s);
        self.group = s.group.clone();
        self.notes = s.notes.clone();
        self.favourite = s.favourite;
        self.local_dir = s.local_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        self.tested = None;
        self.test = None;
        self.confirm_delete = false;
        self.error = None;
    }

    fn blank(&mut self, group: String) {
        *self = SiteManager { group, search: std::mem::take(&mut self.search), ..SiteManager::new(None, &[]) };
        self.form.fresh = true;
    }

    /// The form as a saved site, keeping what the form doesn't show from `old`.
    fn site(&mut self, old: Option<&SavedSite>) -> Result<SavedSite, String> {
        let path = self.form.url()?;
        let mut s = old.cloned().unwrap_or_default();
        s.url = remote::show(&path).unwrap_or_default();
        s.name = self.form.name.trim().to_string();
        s.key = Some(self.form.key.trim()).filter(|k| !k.is_empty()).map(PathBuf::from);
        s.group = self.group.trim().trim_matches('/').to_string();
        s.notes = self.notes.clone();
        s.favourite = self.favourite;
        s.local_dir = Some(self.local_dir.trim()).filter(|d| !d.is_empty()).map(PathBuf::from);
        Ok(s)
    }
}

/// Groups in the order shown: favourites first (as ""), then by name; "" also holds sites in no
/// group.
/// Most recent server folders kept in the Remote sidebar.
const RECENT_SERVERS: usize = 8;

/// Note a folder just opened: a server folder goes to the front of `list`, replacing that
/// server's previous one, so each server shows once with where you were last.
pub(super) fn note_recent(list: &mut Vec<PathBuf>, p: &Path) {
    let Some(url) = remote::site_url(p) else { return };
    list.retain(|q| remote::site_url(q).as_deref() != Some(url.as_str()));
    list.insert(0, p.to_path_buf());
    list.truncate(RECENT_SERVERS);
}

/// "shots on render01": the folder first, since the sidebar cuts long names short.
fn recent_label(site: &remote::Site, p: &str) -> String {
    let name = p.trim_end_matches('/').rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("/");
    format!("{name} on {}", site.host)
}

fn grouped(sites: &[SavedSite], search: &str) -> Vec<(String, Vec<usize>)> {
    let q = search.trim().to_lowercase();
    let mut groups: std::collections::BTreeMap<String, Vec<usize>> = Default::default();
    for (i, s) in sites.iter().enumerate() {
        if !q.is_empty() {
            let hay = format!("{} {} {} {}", s.name, s.url, s.group, s.notes).to_lowercase();
            if !q.split_whitespace().all(|w| hay.contains(w)) {
                continue;
            }
        }
        groups.entry(s.group.clone()).or_default().push(i);
    }
    let mut out: Vec<(String, Vec<usize>)> = groups.into_iter().collect();
    for (_, v) in &mut out {
        v.sort_by(|&a, &b| sites[b].favourite.cmp(&sites[a].favourite).then_with(|| sites[a].shown().to_lowercase().cmp(&sites[b].shown().to_lowercase())));
    }
    out
}

/// A padlock for a site: shut for encrypted protocols.
fn lock(scheme: Option<Scheme>) -> &'static str {
    match scheme {
        Some(s) if s.insecure() => "🔓",
        Some(_) => "🔒",
        None => "🖧",
    }
}

fn scheme_of(p: &Path) -> Option<Scheme> {
    remote::split(p).map(|(s, _)| s.scheme)
}

/// Seconds since 1970, now.
fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl App {
    /// Keep the right desk on screen for the workspace. Every frame, before the Files UI.
    pub(super) fn remote_frame(&mut self, ctx: &egui::Context) {
        let ws = self.files.prefs.workspace;
        use_desk(&mut self.files, ws);
        self.site_manager_window(ctx);
        self.remote_log_window(ctx);
        self.remote_props_window(ctx);
    }

    /// Commands that work differently for items on a server. True when handled here.
    pub(super) fn remote_action(&mut self, a: FAction, paths: &[PathBuf]) -> bool {
        let here = self.files.here().dir().map(Path::to_path_buf);
        let first = paths.first().cloned().or(here);
        match a {
            FAction::Reload => {
                self.files.refresh();
                true
            }
            FAction::SshTerminal => {
                match first.filter(|p| remote::is_remote(p)) {
                    Some(p) => {
                        if let Some((s, _)) = remote::split(&p) {
                            self.ops.notice = Some((format!("Opening an SSH terminal on {}…", s.short()), false));
                        }
                        self.files.spawn(move || ssh_terminal(&p));
                    }
                    None => self.ops.notice = Some(("An SSH terminal opens on a folder on an SFTP server.".into(), true)),
                }
                true
            }
            FAction::Properties if first.as_deref().is_some_and(remote::is_remote) => {
                self.files.rws.props = Some(RemoteProps::new(first.unwrap()));
                true
            }
            FAction::Reveal | FAction::Terminal(_) | FAction::SystemMenu(_) | FAction::CopyUnc if first.as_deref().is_some_and(remote::is_remote) => {
                let ssh = first.as_deref().is_some_and(|p| remote::caps(p).ssh);
                self.ops.notice = Some((if ssh { "That's on a server. Use Open SSH Terminal Here (Ctrl+Alt+T) for a terminal there." } else { "That's on a server, so Explorer and the system's commands don't apply." }.into(), true));
                true
            }
            _ => false,
        }
    }

    /// The Remote workspace's own keys. F5 and F6 copy and move to the other pane (with two
    /// panes), Ctrl+Alt+T opens an SSH terminal. Returns the command to run on the selection.
    pub(super) fn remote_keys(&mut self, ctx: &egui::Context) -> Option<FAction> {
        use egui::{Key, Modifiers as M};
        if self.files.prefs.workspace != Workspace::Remote || self.files.rws.manager.is_some() {
            return None;
        }
        let two = self.files.panes.len() > 1;
        ctx.input_mut(|i| {
            if i.consume_key(M::CTRL | M::ALT, Key::T) {
                Some(FAction::SshTerminal)
            } else if two && i.consume_key(M::NONE, Key::F5) {
                Some(FAction::CopyToOther)
            } else if two && i.consume_key(M::NONE, Key::F6) {
                Some(FAction::MoveToOther)
            } else {
                None
            }
        })
    }

    /// Properties of an item on a server, with its permissions.
    fn remote_props_window(&mut self, ctx: &egui::Context) {
        let Some(pr) = self.files.rws.props.as_mut() else { return };
        if let Some(rx) = &pr.rx {
            match rx.try_recv() {
                Ok(Ok((m, link))) => {
                    pr.mode = m.mode.unwrap_or(0);
                    pr.octal = format!("{:o}", pr.mode);
                    pr.meta = Some(m);
                    pr.link = link;
                    pr.rx = None;
                }
                Ok(Err(e)) => {
                    pr.error = Some(e);
                    pr.rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => pr.rx = None,
            }
        }
        if let Some(rx) = &pr.applying {
            match rx.try_recv() {
                Ok(r) => {
                    if r.is_ok()
                        && let Some(m) = pr.meta.as_mut()
                    {
                        m.mode = Some(pr.mode);
                    }
                    pr.done = Some(r);
                    pr.applying = None;
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => pr.applying = None,
            }
        }
        let caps = remote::caps(&pr.path);
        let mut open = true;
        egui::Window::new("Properties").id(egui::Id::new("remote-props")).open(&mut open).collapsible(false).resizable(false).pivot(Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).show(ctx, |ui| {
            let (site, rpath) = remote::split(&pr.path).unwrap_or_else(|| (remote::Site { scheme: Scheme::Sftp, user: String::new(), host: String::new(), port: 0 }, String::new()));
            egui::Grid::new("rprops").num_columns(2).spacing([16.0, 5.0]).show(ui, |ui| {
                ui.label("Name");
                ui.strong(rpath.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("/"));
                ui.end_row();
                ui.label("Server");
                ui.label(format!("{} {} ({})", lock(Some(site.scheme)), site.short(), site.scheme.title()));
                ui.end_row();
                ui.label("Folder");
                ui.label(rpath.rsplit_once('/').map(|(d, _)| if d.is_empty() { "/" } else { d }).unwrap_or("/"));
                ui.end_row();
                if let Some(e) = &pr.error {
                    ui.label("");
                    ui.colored_label(ui.visuals().error_fg_color, e);
                    ui.end_row();
                }
                let Some(m) = pr.meta else {
                    if pr.error.is_none() {
                        ui.label("");
                        ui.spinner();
                        ui.end_row();
                    }
                    return;
                };
                ui.label("Type");
                ui.label(if m.is_link && m.is_dir { "Link to a folder" } else if m.is_link { "Link" } else if m.is_dir { "Folder" } else { "File" });
                ui.end_row();
                if let Some(t) = &pr.link {
                    ui.label("Points to");
                    ui.monospace(t);
                    ui.end_row();
                }
                if !m.is_dir {
                    ui.label("Size");
                    ui.label(format!("{} ({} bytes)", scan::human(m.len), m.len));
                    ui.end_row();
                }
                if let Some(t) = m.modified_time() {
                    let t: chrono::DateTime<chrono::Local> = t.into();
                    ui.label("Modified");
                    ui.label(t.format("%Y-%m-%d %H:%M:%S").to_string());
                    ui.end_row();
                }
                if m.uid.is_some() || m.gid.is_some() {
                    ui.label("Owner, group");
                    ui.label(format!("{} : {}", m.uid.map(|u| u.to_string()).unwrap_or("?".into()), m.gid.map(|g| g.to_string()).unwrap_or("?".into())));
                    ui.end_row();
                }
                ui.label("Permissions");
                if m.mode.is_none() && !caps.chmod {
                    ui.weak("The server doesn't say.");
                    ui.end_row();
                    return;
                }
                ui.vertical(|ui| {
                    let busy = pr.applying.is_some();
                    ui.add_enabled_ui(caps.chmod && !busy, |ui| {
                        egui::Grid::new("rwx").num_columns(4).spacing([10.0, 2.0]).show(ui, |ui| {
                            ui.label("");
                            for h in ["Read", "Write", "Run"] {
                                ui.weak(h);
                            }
                            ui.end_row();
                            for (who, shift) in [("Owner", 6), ("Group", 3), ("Others", 0)] {
                                ui.label(who);
                                for bit in [4u32, 2, 1] {
                                    let mask = bit << shift;
                                    let mut on = pr.mode & mask != 0;
                                    if ui.checkbox(&mut on, "").changed() {
                                        pr.mode = if on { pr.mode | mask } else { pr.mode & !mask };
                                        pr.octal = format!("{:o}", pr.mode);
                                    }
                                }
                                ui.end_row();
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Octal");
                            if ui.add(egui::TextEdit::singleline(&mut pr.octal).desired_width(48.0).char_limit(4)).changed()
                                && let Ok(v) = u32::from_str_radix(pr.octal.trim(), 8)
                            {
                                pr.mode = v & 0o7777;
                            }
                            ui.monospace(rwx(pr.mode));
                        });
                        if m.is_dir {
                            ui.checkbox(&mut pr.deep, "Also everything inside").on_hover_text("Files get these permissions; folders also get Run wherever they have Read, so they can still be opened");
                        }
                    });
                    if !caps.chmod {
                        ui.weak("This kind of server can't change permissions.");
                    }
                    let changed = Some(pr.mode) != m.mode || pr.deep;
                    ui.horizontal(|ui| {
                        if busy {
                            ui.spinner();
                            ui.label("Changing…");
                        } else if caps.chmod && ui.add_enabled(changed, egui::Button::new("Apply")).clicked() {
                            let (tx, rx) = mpsc::channel();
                            let (path, mode, deep) = (pr.path.clone(), pr.mode, pr.deep);
                            std::thread::spawn(move || {
                                let dir_mode = mode | ((mode & 0o444) >> 2);
                                let _ = tx.send(remote::chmod(&path, mode, deep.then_some(dir_mode)).map_err(|e| remote::describe(&e)));
                            });
                            pr.applying = Some(rx);
                            pr.done = None;
                        }
                    });
                    match &pr.done {
                        Some(Ok(n)) if *n > 1 => {
                            ui.colored_label(style::pal(ui).accent, format!("✔ Changed {n} items."));
                        }
                        Some(Ok(_)) => {
                            ui.colored_label(style::pal(ui).accent, "✔ Changed.");
                        }
                        Some(Err(e)) => {
                            ui.colored_label(ui.visuals().error_fg_color, e);
                        }
                        None => {}
                    }
                });
                ui.end_row();
            });
        });
        if !open {
            self.files.rws.props = None;
        }
    }

    /// Open a saved site in the Remote workspace: the site in the right-hand pane (connecting
    /// first), and its local folder, if it has one, on the left.
    pub(super) fn open_site(&mut self, i: usize, new_tab: bool) {
        let Some(s) = self.files.prefs.sites.get_mut(i) else { return };
        s.last_used = now();
        let s = s.clone();
        let Some(path) = s.path() else { return };
        remote::set_options(&s.url, s.options());
        if self.files.prefs.workspace != Workspace::Remote {
            self.set_workspace(Workspace::Remote);
            use_desk(&mut self.files, Workspace::Remote);
        }
        let target = self.remote_pane();
        if let Some(local) = s.local_dir.clone()
            && self.files.panes.len() > 1
        {
            self.files.go_in(1 - target, Location::Dir(local));
        }
        self.files.set_focus(target);
        if new_tab {
            self.files.new_tab(Location::Dir(path.clone()));
        }
        self.files.rws.target = Some(target);
        self.open_server(path);
    }

    /// The pane a server opens in: the focused one if it's on a server or the only one,
    /// otherwise the other one (so local stays beside remote).
    fn remote_pane(&self) -> usize {
        let f = &self.files;
        if f.panes.len() < 2 {
            return 0;
        }
        let on_server = |i: usize| f.panes[i].tab().here().dir().is_some_and(remote::is_remote);
        if on_server(f.focus) {
            f.focus
        } else if on_server(1 - f.focus) {
            1 - f.focus
        } else {
            1
        }
    }

    /// Connect to what's typed in the quick-connect bar.
    fn quick_connect(&mut self) {
        let text = self.files.rws.quick.trim().to_string();
        if text.is_empty() {
            return;
        }
        if crate::netdrives::parse_smb(&text).is_some() {
            // A network share goes through the system, in the Connect box.
            self.files.connect = Some(ConnectBox { target: text, remember: true, save: false, fresh: true, ..Default::default() });
            return;
        }
        let path = remote::parse(&text).or_else(|| {
            // A bare host ("render01", "rich@render01:2222/mnt") is SFTP.
            (!text.contains("://") && !text.contains(['\\', ' '])).then(|| remote::parse(&format!("sftp://{text}"))).flatten()
        });
        match path {
            Some(p) => {
                // A password typed in the address has been taken out of it: don't leave it showing.
                if text.contains('@') {
                    self.files.rws.quick = remote::show(&p).unwrap_or_default();
                }
                let target = self.remote_pane();
                self.files.set_focus(target);
                self.files.rws.target = Some(target);
                self.open_server(p);
            }
            None => self.ops.notice = Some((format!("\"{text}\" isn't an address dirmap can connect to. Try sftp://user@host/folder, ftp://host, https://host/dav or \\\\server\\share."), true)),
        }
    }

    /// The Remote workspace's part of the toolbar: quick connect, Site Manager, and the focused
    /// pane's connection.
    pub(super) fn remote_toolbar(&mut self, ui: &mut egui::Ui) {
        if self.files.prefs.workspace != Workspace::Remote {
            return;
        }
        let pal = style::pal(ui);
        style::tool_group(ui, |ui| {
            if ui.add(egui::Button::new("🗄 Sites")).on_hover_text("Site Manager: saved servers, groups and sign-in").clicked() {
                let sel = self.files.here().dir().and_then(|d| {
                    let url = remote::site_url(d)?;
                    self.files.prefs.sites.iter().position(|s| s.path().and_then(|p| remote::site_url(&p)).as_deref() == Some(url.as_str()))
                });
                self.files.rws.manager = Some(SiteManager::new(sel, &self.files.prefs.sites));
            }
        });
        let narrow = style::narrow(ui);
        let r = ui.add(
            egui::TextEdit::singleline(&mut self.files.rws.quick)
                .hint_text("Quick connect: sftp://user@host/folder")
                .desired_width(if narrow { 140.0 } else { 220.0 }),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        r.on_hover_text("Type a server's address and press Enter. A bare name (render01) is SFTP. FTP and http WebDAV aren't encrypted and dirmap asks before using them.");
        if enter || ui.add_enabled(!self.files.rws.quick.trim().is_empty(), egui::Button::new("Connect")).clicked() {
            self.quick_connect();
        }
        // The focused pane's connection.
        let Some(dir) = self.files.here().dir().map(Path::to_path_buf) else { return };
        let Some((site, _)) = remote::split(&dir) else {
            ui.label(egui::RichText::new("🖳 This computer").color(pal.weak)).on_hover_text("The focused pane shows a folder on this computer");
            return;
        };
        let url = site.url();
        let on = remote::connected().contains(&url);
        let ms = remote::latency(&url).map(|d| format!(" · {} ms", d.as_millis())).unwrap_or_default();
        let text = format!("{} {}{ms}", lock(Some(site.scheme)), site.host);
        let colour = if site.scheme.insecure() { ui.visuals().warn_fg_color } else if on { pal.text } else { pal.weak };
        let tip = format!(
            "{}\n{}\n{}{}\nClick for the connection's log and Disconnect",
            site.url(),
            site.scheme.title(),
            if on { "Connected" } else { "Not connected (it connects when used)" },
            if site.scheme.insecure() { "\nNot encrypted: the password and files can be read on the network" } else { "" }
        );
        ui.menu_button(egui::RichText::new(text).color(colour), |ui| {
            if ui.add(egui::Button::new(style::mi("Connection Log…"))).clicked() {
                self.files.rws.log = Some(url.clone());
                ui.close();
            }
            if remote::caps(&dir).ssh
                && ui.add(egui::Button::new(style::mi("Open SSH Terminal Here…")).shortcut_text(ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::CTRL | egui::Modifiers::ALT, egui::Key::T)))).clicked()
            {
                self.files_action(FAction::SshTerminal, vec![dir.clone()]);
                ui.close();
            }
            if ui.add_enabled(on, egui::Button::new(style::mi("Disconnect"))).clicked() {
                remote::disconnect(&dir);
                ui.close();
            }
            if !self.files.prefs.sites.iter().any(|s| s.path().and_then(|p| remote::site_url(&p)).as_deref() == Some(url.as_str()))
                && ui.add(egui::Button::new(style::mi("Save as Site…"))).clicked()
            {
                let mut m = SiteManager::new(None, &self.files.prefs.sites);
                m.form = ConnectBox::editing(usize::MAX, &SavedSite { url: remote::show(&dir).unwrap_or_default(), ..Default::default() });
                m.form.editing = None;
                m.form.fresh = true;
                self.files.rws.manager = Some(m);
                ui.close();
            }
        })
        .response
        .on_hover_text(tip);
    }

    /// The Remote workspace's sidebar sections: Sites (with search and groups) and Sessions.
    pub(super) fn remote_sidebar(&mut self, ui: &mut egui::Ui, here: &Location, picks: &mut FPicks) {
        if self.files.prefs.workspace != Workspace::Remote {
            return;
        }
        let pal = style::pal(ui);
        let sites = self.files.prefs.sites.clone();
        let connected = remote::connected();
        let here_site = here.dir().and_then(remote::site_url);
        let (mut open, mut open_tab, mut edit, mut fav) = (None, None, None, None);
        let mut new_site = false;
        files::side_section(ui, "rm-sites", "Sites", true, |ui| {
            if sites.len() > 6 {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.add(egui::TextEdit::singleline(&mut self.files.rws.search).hint_text("🔍 Find a site").desired_width(ui.available_width() - 10.0));
                });
            }
            let searching = !self.files.rws.search.trim().is_empty();
            let groups = grouped(&sites, &self.files.rws.search);
            let favs: Vec<usize> = groups.iter().flat_map(|(_, v)| v.iter().copied()).filter(|&i| sites[i].favourite).collect();
            let mut row = |ui: &mut egui::Ui, i: usize, indent: f32| {
                let s = &sites[i];
                let Some(path) = s.path() else { return };
                let url = remote::site_url(&path).unwrap_or_default();
                let on = connected.contains(&url);
                let at = here_site.as_deref() == Some(url.as_str());
                let resp = ui
                    .horizontal(|ui| {
                        ui.add_space(indent);
                        files::side_row(ui, lock(scheme_of(&path)), &s.shown(), at, 24.0)
                    })
                    .inner;
                let resp = resp.on_hover_text(format!(
                    "{}\n{}{}{}",
                    s.url,
                    if on { "Connected" } else { "Click to connect" },
                    if scheme_of(&path).is_some_and(Scheme::insecure) { "\nNot encrypted" } else { "" },
                    if s.notes.trim().is_empty() { String::new() } else { format!("\n\n{}", s.notes.trim()) }
                ));
                if on {
                    ui.painter().circle_filled(resp.rect.right_center() - vec2(10.0, 0.0), 3.0, pal.accent);
                }
                if resp.clicked() {
                    open = Some(i);
                }
                if resp.middle_clicked() {
                    open_tab = Some(i);
                }
                if let Some(d) = self.folder_drop(ui, &resp, &path) {
                    picks.drop = Some(d);
                }
                resp.context_menu(|ui| {
                    if ui.add(egui::Button::new(style::mi("Open in New Tab"))).clicked() {
                        open_tab = Some(i);
                        ui.close();
                    }
                    if ui.add(egui::Button::new(style::mi(if s.favourite { "Remove from Favourites" } else { "Add to Favourites" }))).clicked() {
                        fav = Some(i);
                        ui.close();
                    }
                    if ui.add(egui::Button::new(style::mi("Edit…"))).clicked() {
                        edit = Some(i);
                        ui.close();
                    }
                    if on && ui.add(egui::Button::new(style::mi("Disconnect"))).clicked() {
                        remote::disconnect(&path);
                        ui.close();
                    }
                });
            };
            if !favs.is_empty() && !searching {
                for &i in &favs {
                    row(ui, i, 0.0);
                }
                ui.add_space(4.0);
            }
            for (g, members) in &groups {
                let members: Vec<usize> = members.iter().copied().filter(|&i| !sites[i].favourite || searching).collect();
                if members.is_empty() {
                    continue;
                }
                if g.is_empty() {
                    for &i in &members {
                        row(ui, i, 0.0);
                    }
                } else {
                    let depth = g.matches('/').count() as f32;
                    let id = ui.id().with(("rm-group", g));
                    let mut open_g = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(true);
                    let title = g.rsplit('/').next().unwrap_or(g);
                    let resp = ui
                        .horizontal(|ui| {
                            ui.add_space(depth * 10.0);
                            files::side_row(ui, if open_g { "🗁" } else { "🗀" }, &format!("{title}  ({})", members.len()), false, 22.0)
                        })
                        .inner;
                    if resp.clicked() {
                        open_g = !open_g;
                        ui.data_mut(|d| d.insert_temp(id, open_g));
                    }
                    if open_g {
                        for &i in &members {
                            row(ui, i, depth * 10.0 + 12.0);
                        }
                    }
                }
            }
            if sites.is_empty() {
                ui.label(egui::RichText::new("No saved sites yet").italics().color(pal.weak));
            }
            ui.add_space(2.0);
            if ui.add(egui::Button::new(egui::RichText::new("✚ New Site…").small().color(pal.weak)).frame_when_inactive(false)).clicked() {
                new_site = true;
            }
        });
        // Connected now, saved or not.
        let unsaved: Vec<String> = connected
            .iter()
            .filter(|u| !sites.iter().any(|s| s.path().and_then(|p| remote::site_url(&p)).as_deref() == Some(u.as_str())))
            .cloned()
            .collect();
        if !unsaved.is_empty() {
            files::side_section(ui, "rm-sessions", "Other Connections", true, |ui| {
                for u in &unsaved {
                    let Some(path) = remote::parse(u) else { continue };
                    let resp = files::side_row(ui, lock(scheme_of(&path)), u.split("://").nth(1).unwrap_or(u), here_site.as_deref() == Some(u.as_str()), 24.0).on_hover_text(format!("{u}\nConnected (not saved)"));
                    ui.painter().circle_filled(resp.rect.right_center() - vec2(10.0, 0.0), 3.0, pal.accent);
                    if resp.clicked() {
                        let target = self.remote_pane();
                        self.files.set_focus(target);
                        self.files.rws.target = Some(target);
                        self.open_server(path.clone());
                    }
                    resp.context_menu(|ui| {
                        if ui.add(egui::Button::new(style::mi("Disconnect"))).clicked() {
                            remote::disconnect(&path);
                            ui.close();
                        }
                        if ui.add(egui::Button::new(style::mi("Connection Log…"))).clicked() {
                            self.files.rws.log = Some(u.clone());
                            ui.close();
                        }
                    });
                }
            });
        }
        // Where you were lately, saved or not: click to go back there.
        let recent = self.files.prefs.recent_servers.clone();
        let (mut go, mut go_tab, mut drop_recent) = (None, None, None);
        if !recent.is_empty() {
            files::side_section(ui, "rm-recent", "Recent Servers", true, |ui| {
                for (k, p) in recent.iter().enumerate() {
                    let Some((site, rp)) = remote::split(p) else { continue };
                    let at = here.dir() == Some(p.as_path());
                    let resp = files::side_row(ui, lock(Some(site.scheme)), &recent_label(&site, &rp), at, 24.0)
                        .on_hover_text(format!("{}{}", site.url(), rp));
                    if resp.clicked() {
                        go = Some(k);
                    }
                    if resp.middle_clicked() {
                        go_tab = Some(k);
                    }
                    resp.context_menu(|ui| {
                        if ui.add(egui::Button::new(style::mi("Open in New Tab"))).clicked() {
                            go_tab = Some(k);
                            ui.close();
                        }
                        if ui.add(egui::Button::new(style::mi("Remove from Recent"))).clicked() {
                            drop_recent = Some(k);
                            ui.close();
                        }
                    });
                }
            });
        }
        if let Some(k) = go.or(go_tab)
            && let Some(p) = recent.get(k)
        {
            let target = self.remote_pane();
            self.files.set_focus(target);
            if go_tab.is_some() {
                self.files.new_tab(Location::Dir(p.clone()));
            }
            self.files.rws.target = Some(target);
            self.open_server(p.clone());
        }
        if let Some(k) = drop_recent {
            self.files.prefs.recent_servers.retain(|q| Some(q) != recent.get(k));
        }
        if let Some(i) = open {
            self.open_site(i, false);
        }
        if let Some(i) = open_tab {
            self.open_site(i, true);
        }
        if let Some(i) = edit {
            self.files.rws.manager = Some(SiteManager::new(Some(i), &self.files.prefs.sites));
        }
        if let Some(i) = fav
            && let Some(s) = self.files.prefs.sites.get_mut(i)
        {
            s.favourite = !s.favourite;
        }
        if new_site {
            let mut m = SiteManager::new(None, &self.files.prefs.sites);
            m.form.fresh = true;
            self.files.rws.manager = Some(m);
        }
    }

    /// The Site Manager window. Call once per frame.
    fn site_manager_window(&mut self, ctx: &egui::Context) {
        let Some(m) = self.files.rws.manager.as_mut() else { return };
        if let Some(rx) = &m.test {
            match rx.try_recv() {
                Ok(r) => {
                    m.tested = Some(r);
                    m.test = None;
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => m.test = None,
            }
        }
        let sites = &mut self.files.prefs.sites;
        let mut open = true;
        let mut connect = None;
        let mut close = false;
        let all_groups: Vec<String> = {
            let mut g: Vec<String> = sites.iter().map(|s| s.group.clone()).filter(|g| !g.is_empty()).collect();
            g.sort_by_key(|s| s.to_lowercase());
            g.dedup();
            g
        };
        egui::Window::new("Site Manager").id(egui::Id::new("site-manager")).open(&mut open).collapsible(false).resizable(true).default_size([760.0, 470.0]).show(ctx, |ui| {
            let pal = style::pal(ui);
            ui.horizontal_top(|ui| {
                // The list.
                ui.vertical(|ui| {
                    ui.set_width(250.0);
                    ui.add(egui::TextEdit::singleline(&mut m.search).hint_text("🔍 Find").desired_width(250.0));
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().id_salt("sm-list").max_height(360.0).auto_shrink([false, false]).show(ui, |ui| {
                        let groups = grouped(sites, &m.search);
                        let mut pick = None;
                        let mut rename = None;
                        for (g, members) in &groups {
                            if !g.is_empty() {
                                let r = ui.label(egui::RichText::new(format!("🗁 {g}")).strong().color(pal.weak));
                                r.context_menu(|ui| {
                                    if ui.button("Rename Group…").clicked() {
                                        rename = Some(g.clone());
                                        ui.close();
                                    }
                                });
                            }
                            for &i in members {
                                let s = &sites[i];
                                let label = format!("{}{} {}", if g.is_empty() { "" } else { "    " }, lock(s.path().as_deref().and_then(scheme_of)), s.shown());
                                let star = if s.favourite { " ★" } else { "" };
                                if ui.selectable_label(m.sel == Some(i), format!("{label}{star}")).on_hover_text(&s.url).clicked() {
                                    pick = Some(i);
                                }
                            }
                        }
                        if groups.is_empty() {
                            ui.weak(if sites.is_empty() { "No saved sites. Fill in the form and press Save." } else { "Nothing matches." });
                        }
                        if let Some(i) = pick {
                            m.load(i, &sites[i]);
                        }
                        if let Some(g) = rename {
                            m.rename_group = Some((g.clone(), g));
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("New Site").clicked() {
                            let g = m.group.clone();
                            m.blank(g);
                        }
                        if ui.add_enabled(m.sel.is_some(), egui::Button::new("Duplicate")).clicked()
                            && let Some(i) = m.sel
                        {
                            let mut copy = sites[i].clone();
                            copy.name = format!("{} (copy)", sites[i].shown());
                            copy.favourite = false;
                            copy.last_used = 0;
                            sites.insert(i + 1, copy);
                            m.load(i + 1, &sites[i + 1]);
                        }
                        if ui.add_enabled(m.sel.is_some(), egui::Button::new("Delete")).clicked() {
                            m.confirm_delete = true;
                        }
                        ui.menu_button("Import/Export", |ui| {
                            use super::siteimport as si;
                            let mut got: Option<Result<si::Found, String>> = None;
                            let fz = si::filezilla_file();
                            if ui.add_enabled(fz.is_some(), egui::Button::new("Import from FileZilla")).on_disabled_hover_text("FileZilla's sites weren't found on this computer; use From a File…").clicked()
                                && let Some(f) = &fz
                            {
                                got = Some(std::fs::read_to_string(f).map_err(|e| e.to_string()).and_then(|t| si::from_filezilla(&t)));
                                ui.close();
                            }
                            if ui.button("Import from WinSCP").clicked() {
                                got = Some(si::winscp_here());
                                ui.close();
                            }
                            if ui.button("Import from a File…").on_hover_text("A FileZilla sitemanager.xml, or a WinSCP.ini (WinSCP: Tools > Export/Backup Configuration)").clicked() {
                                if let Some(f) = rfd::FileDialog::new().add_filter("Site files", &["xml", "ini"]).pick_file() {
                                    got = Some(si::from_file(&f));
                                }
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("Export for FileZilla…").on_hover_text("All sites, without passwords, as a sitemanager.xml that FileZilla, WinSCP and Cyberduck can import").clicked() {
                                let (xml, left_out) = si::to_filezilla(&sites);
                                if let Some(f) = rfd::FileDialog::new().set_file_name("dirmap-sites.xml").add_filter("FileZilla site file", &["xml"]).save_file() {
                                    match std::fs::write(&f, xml) {
                                        Ok(()) => {
                                            let extra = if left_out.is_empty() { String::new() } else { format!(" Left out (FileZilla has no WebDAV): {}.", left_out.join(", ")) };
                                            m.tested = Some(Ok(format!("Exported {} sites to {}, without passwords.{extra}", sites.len() - left_out.len(), f.display())));
                                            m.error = None;
                                        }
                                        Err(e) => m.error = Some(format!("Couldn't write {}: {e}", f.display())),
                                    }
                                }
                                ui.close();
                            }
                            if let Some(r) = got {
                                match r {
                                    Ok(found) => {
                                        let n = si::merge(sites, found.sites);
                                        let mut msg = format!("Imported {n} site{}. Passwords aren't brought across: you're asked on the first connection, and Remember keeps them in the keychain.", if n == 1 { "" } else { "s" });
                                        if !found.skipped.is_empty() {
                                            msg.push_str(&format!(" Left out: {}.", found.skipped.join("; ")));
                                        }
                                        m.tested = Some(Ok(msg));
                                        m.error = None;
                                    }
                                    Err(e) => m.error = Some(e),
                                }
                            }
                        });
                    });
                });
                ui.separator();
                // The form.
                ui.vertical(|ui| {
                    let f = &mut m.form;
                    let kind_name = |k: Scheme| k.title();
                    egui::Grid::new("sm-form").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                        ui.label("Name");
                        ui.add(egui::TextEdit::singleline(&mut f.name).hint_text("Render farm head").desired_width(300.0));
                        ui.end_row();
                        ui.label("Group");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut m.group).hint_text("(none)  e.g. Render Farm, Clients/Acme").desired_width(250.0));
                            egui::ComboBox::from_id_salt("sm-groups").width(30.0).selected_text("").show_ui(ui, |ui| {
                                for g in &all_groups {
                                    if ui.selectable_label(m.group == *g, g).clicked() {
                                        m.group = g.clone();
                                    }
                                }
                                if ui.selectable_label(m.group.is_empty(), "(none)").clicked() {
                                    m.group.clear();
                                }
                            });
                        });
                        ui.end_row();
                        ui.label("Protocol");
                        let mut kind = f.kind.unwrap_or(Scheme::Sftp);
                        egui::ComboBox::from_id_salt("sm-kind").width(300.0).selected_text(kind_name(kind)).show_ui(ui, |ui| {
                            for k in Scheme::ALL {
                                ui.selectable_value(&mut kind, k, kind_name(k));
                            }
                        });
                        f.kind = Some(kind);
                        ui.end_row();
                        ui.label("Server");
                        ui.horizontal(|ui| {
                            let r = ui.add(egui::TextEdit::singleline(&mut f.host).hint_text("render01.studio.lan").desired_width(220.0));
                            if std::mem::take(&mut f.fresh) {
                                r.request_focus();
                            }
                            ui.label("Port");
                            ui.add(egui::TextEdit::singleline(&mut f.port).hint_text(kind.default_port().to_string()).desired_width(48.0));
                        });
                        ui.end_row();
                        ui.label("User name");
                        ui.add(egui::TextEdit::singleline(&mut f.user).hint_text(if kind == Scheme::Sftp { remote::local_user() } else { "anonymous".into() }).desired_width(300.0));
                        ui.end_row();
                        if kind == Scheme::Sftp {
                            ui.label("Sign in with");
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut f.key).hint_text("SSH agent, then ~/.ssh keys, then password").desired_width(220.0));
                                if ui.button("Key…").on_hover_text("Choose a private key file").clicked() {
                                    let start = crate::fsys::places::home().map(|h| h.join(".ssh"));
                                    let mut d = rfd::FileDialog::new().set_title("Choose an SSH key file");
                                    if let Some(s) = start.filter(|s| s.is_dir()) {
                                        d = d.set_directory(s);
                                    }
                                    if let Some(k) = d.pick_file() {
                                        f.key = k.display().to_string();
                                    }
                                }
                            });
                            ui.end_row();
                        }
                        ui.label("Remote folder");
                        ui.add(egui::TextEdit::singleline(&mut f.folder).hint_text(if kind == Scheme::Sftp { "(your home folder)" } else { "/" }).desired_width(300.0));
                        ui.end_row();
                        ui.label("Local folder");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut m.local_dir).hint_text("(optional) shown beside it").desired_width(220.0));
                            if ui.button("Choose…").clicked()
                                && let Some(d) = rfd::FileDialog::new().set_title("Local folder to show beside this site").pick_folder()
                            {
                                m.local_dir = d.display().to_string();
                            }
                        });
                        ui.end_row();
                        ui.label("Notes");
                        ui.add(egui::TextEdit::multiline(&mut m.notes).desired_rows(2).desired_width(300.0));
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut m.favourite, "★ Favourite (listed first, and in the Files sidebar)");
                        ui.end_row();
                    });
                    ui.add_space(4.0);
                    if kind_is_insecure(f) {
                        ui.colored_label(ui.visuals().warn_fg_color, "Not encrypted: the password and files can be read on the network. dirmap asks before connecting.");
                    }
                    ui.weak(format!("Passwords are never saved in dirmap's files; when you tick Remember they go to {}.", super::netui::keychain_name()));
                    if let Some(r) = &m.tested {
                        match r {
                            Ok(t) => ui.colored_label(pal.accent, format!("✔ {t}")),
                            Err(e) => ui.colored_label(ui.visuals().error_fg_color, format!("✖ {e}")),
                        };
                    }
                    if let Some(e) = &m.error {
                        ui.colored_label(ui.visuals().error_fg_color, e);
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let ready = !m.form.host.trim().is_empty();
                        if ui.add_enabled(ready, egui::Button::new("Save")).clicked() {
                            match m.site(m.sel.and_then(|i| sites.get(i))) {
                                Ok(s) => {
                                    remote::set_options(&s.url, s.options());
                                    match m.sel {
                                        Some(i) if i < sites.len() => sites[i] = s,
                                        _ => {
                                            sites.push(s);
                                            m.sel = Some(sites.len() - 1);
                                        }
                                    }
                                    m.error = None;
                                }
                                Err(e) => m.error = Some(e),
                            }
                        }
                        if m.test.is_some() {
                            ui.spinner();
                            ui.label("Testing…");
                        } else if ui.add_enabled(ready, egui::Button::new("Test")).on_hover_text("Connect, list the folder and disconnect, without saving").clicked() {
                            match m.site(m.sel.and_then(|i| sites.get(i))) {
                                Ok(s) => {
                                    remote::set_options(&s.url, s.options());
                                    let (tx, rx) = mpsc::channel();
                                    let path = s.path();
                                    std::thread::spawn(move || {
                                        let _ = tx.send(path.ok_or_else(|| "That isn't an address.".to_string()).and_then(|p| remote::test(&p)));
                                    });
                                    m.test = Some(rx);
                                    m.tested = None;
                                    m.error = None;
                                }
                                Err(e) => m.error = Some(e),
                            }
                        }
                        if ui.add_enabled(ready, egui::Button::new("Connect")).on_hover_text("Save it and open it in the Remote workspace").clicked() {
                            match m.site(m.sel.and_then(|i| sites.get(i))) {
                                Ok(s) => {
                                    let i = match m.sel {
                                        Some(i) if i < sites.len() => {
                                            sites[i] = s;
                                            i
                                        }
                                        _ => {
                                            sites.push(s);
                                            sites.len() - 1
                                        }
                                    };
                                    connect = Some(i);
                                    close = true;
                                }
                                Err(e) => m.error = Some(e),
                            }
                        }
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
                });
            });
        });
        // Confirmations over the window.
        if m.confirm_delete
            && let Some(i) = m.sel.filter(|&i| i < sites.len())
        {
            let mut answer = None;
            egui::Modal::new(egui::Id::new("sm-delete")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.heading("Delete this site?");
                ui.label(format!("{} ({}) is removed from the Site Manager. Files on the server aren't touched. A password kept in the keychain for it is removed too.", sites[i].shown(), sites[i].url));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        answer = Some(false);
                    }
                });
            });
            if let Some(yes) = answer {
                m.confirm_delete = false;
                if yes {
                    let gone = sites.remove(i);
                    let url = gone.url.clone();
                    if !sites.iter().any(|s| s.url == url) {
                        std::thread::spawn(move || remote::forget_password(&url));
                    }
                    let g = gone.group;
                    m.blank(g);
                }
            }
        }
        if let Some((old, new)) = m.rename_group.as_mut() {
            let mut answer = None;
            egui::Modal::new(egui::Id::new("sm-rename-group")).show(ctx, |ui| {
                ui.set_width(340.0);
                ui.heading("Rename group");
                let r = ui.add(egui::TextEdit::singleline(new).desired_width(f32::INFINITY));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Rename").clicked() || enter {
                        answer = Some(true);
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        answer = Some(false);
                    }
                });
            });
            if let Some(yes) = answer {
                if yes {
                    let (old, new) = (old.clone(), new.trim().trim_matches('/').to_string());
                    for s in sites.iter_mut() {
                        if s.group == old {
                            s.group = new.clone();
                        } else if let Some(rest) = s.group.strip_prefix(&format!("{old}/")) {
                            s.group = if new.is_empty() { rest.to_string() } else { format!("{new}/{rest}") };
                        }
                    }
                    if m.group == old {
                        m.group = new;
                    }
                }
                m.rename_group = None;
            }
        }
        if !open || close {
            self.files.rws.manager = None;
        }
        if let Some(i) = connect {
            self.open_site(i, false);
        }
    }

    /// A site's connection log.
    fn remote_log_window(&mut self, ctx: &egui::Context) {
        let Some(url) = self.files.rws.log.clone() else { return };
        let mut open = true;
        egui::Window::new(format!("Connection log: {url}")).id(egui::Id::new("remote-log")).open(&mut open).default_size([620.0, 340.0]).show(ctx, |ui| {
            let lines = remote::log_of(&url);
            if let Some(l) = remote::latency(&url) {
                ui.weak(format!("Round trip: about {} ms", l.as_millis()));
            }
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                if lines.is_empty() {
                    ui.weak("Nothing yet.");
                }
                for l in &lines {
                    let t: chrono::DateTime<chrono::Local> = l.at.into();
                    let text = egui::RichText::new(format!("{}  {}", t.format("%H:%M:%S"), l.text)).monospace().size(11.5);
                    if l.error {
                        ui.colored_label(ui.visuals().error_fg_color, text);
                    } else {
                        ui.label(text);
                    }
                }
            });
            ui.weak("Passwords, passphrases and keys are never written here.");
        });
        if !open {
            self.files.rws.log = None;
        }
    }
}

fn kind_is_insecure(f: &ConnectBox) -> bool {
    f.kind.is_some_and(Scheme::insecure)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(name: &str, group: &str, fav: bool) -> SavedSite {
        SavedSite { name: name.into(), url: format!("sftp://u@{name}"), group: group.into(), favourite: fav, ..Default::default() }
    }

    #[test]
    fn groups_sort_and_search() {
        let s = vec![site("b", "Farm", false), site("a", "Farm", false), site("nas", "", true), site("c", "Clients/Acme", false)];
        let g = grouped(&s, "");
        let names: Vec<(String, Vec<String>)> = g.iter().map(|(g, v)| (g.clone(), v.iter().map(|&i| s[i].name.clone()).collect())).collect();
        assert_eq!(names, vec![("".into(), vec!["nas".into()]), ("Clients/Acme".into(), vec!["c".into()]), ("Farm".into(), vec!["a".into(), "b".into()])]);
        let g = grouped(&s, "farm b");
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].1, vec![0]);
    }

    #[test]
    fn shell_quoting_and_ssh_command() {
        assert_eq!(sh_quote("render01"), "render01");
        assert_eq!(sh_quote("it's here"), "'it'\\''s here'");
        assert_eq!(rwx(0o750), "rwxr-x---");
        let a = remote::ssh_args(Path::new("sftp:/rich@render01:2222/mnt/my shots/it's")).unwrap();
        assert_eq!(a[..4], ["-t", "-p", "2222", "rich@render01"]);
        assert_eq!(a[4], "cd '/mnt/my shots/it'\\''s' && exec \"${SHELL:-/bin/sh}\" -l");
        assert!(remote::ssh_args(Path::new("ftp:/rich@nas/x")).is_none());
    }

    #[test]
    fn old_saved_sites_still_load() {
        let old = r#"(name: "Render", url: "sftp://rich@render01/mnt", key: None)"#;
        let s: SavedSite = ron::from_str(old).unwrap();
        assert_eq!(s.name, "Render");
        assert!(s.group.is_empty() && !s.favourite && !s.allow_insecure);
    }
}
