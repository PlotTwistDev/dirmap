//! The Remote workspace: the File Manager with its own desk (panes, tabs, split, dock), a
//! sidebar of saved sites, a quick-connect bar and the Site Manager.
//!
//! Remote is not a second file manager. It is the Files workspace's code with a different
//! desk swapped in: `use_desk` exchanges the panes and the layout settings when the user moves
//! between Files and Remote, so every view, menu, shortcut, preview, drag and file operation
//! is shared. Favourites, saved sites, marks and the rest of `FilesPrefs` belong to both.
//!
//! See `docs/design/dirmap-2.3-remote.md`.

use super::files::{FPicks, Files, FilesPrefs, Pane, PaneSave, Split, Workspace};
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
        }
    }
}

/// The dock starts closed in Remote (it gets the Transfers tab in phase 3).
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
    fn old_saved_sites_still_load() {
        let old = r#"(name: "Render", url: "sftp://rich@render01/mnt", key: None)"#;
        let s: SavedSite = ron::from_str(old).unwrap();
        assert_eq!(s.name, "Render");
        assert!(s.group.is_empty() && !s.favourite && !s.allow_insecure);
    }
}
