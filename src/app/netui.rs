//! The sidebar's Network & Remote section and the Connect to Server box.
//!
//! It lists network drives and mounts (with their state), the server locations the user saved,
//! and connections the system remembers (Windows mapped drives, GVFS mounts on Linux). Connecting
//! and disconnecting go through the operating system on a worker thread, so a dead server shows
//! an error instead of freezing the window, and dirmap never sees a password for those.
//!
//! It also keeps the user's SFTP, FTP and WebDAV servers (dirmap's own connections, in
//! `fsys::remote`), and shows the questions those connections ask: a new server's identity, a
//! password, a key's passphrase.

use crate::fsys::PathShow;
use super::files::FPicks;
use super::*;
use crate::fsys::history::Location;
use crate::fsys::remote::{self, Answer, Ask, Scheme};
use crate::netdrives;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A server the user saved under Network & Remote. No password is kept here: that is in the
/// system keychain, if the user asked for it to be remembered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub(super) struct SavedSite {
    /// What the sidebar shows ("Render farm").
    pub name: String,
    /// Where it opens: `sftp://rich@render01/mnt/shots`.
    pub url: String,
    /// An SSH key file to sign in with.
    pub key: Option<PathBuf>,
    /// The Site Manager's folder for it: "Render Farm", or "Clients/Acme" for a subfolder.
    pub group: String,
    /// Shown first, and in the Files sidebar too.
    pub favourite: bool,
    /// The user's own notes ("NAS in the machine room, admin is Sam").
    pub notes: String,
    /// The user agreed to connect although the protocol isn't encrypted (FTP, http WebDAV).
    pub allow_insecure: bool,
    /// The local folder to show beside it in the Remote workspace.
    pub local_dir: Option<PathBuf>,
    /// When it was last opened (seconds since 1970), for Recent.
    pub last_used: i64,
}

impl SavedSite {
    pub fn path(&self) -> Option<PathBuf> {
        remote::parse(&self.url)
    }

    /// Connections to this site use its key file and insecure choice.
    pub fn options(&self) -> remote::Options {
        remote::Options { key: self.key.clone(), allow_insecure: self.allow_insecure }
    }

    pub(super) fn shown(&self) -> String {
        if !self.name.trim().is_empty() {
            return self.name.clone();
        }
        self.path().and_then(|p| remote::split(&p)).map(|(s, _)| s.short()).unwrap_or_else(|| self.url.clone())
    }
}

/// A question from a connection, waiting for the user.
pub(super) struct AskBox {
    ask: Ask,
    reply: mpsc::Sender<Answer>,
    text: String,
    remember: bool,
    fresh: bool,
}

/// Where the questions arrive.
pub(super) type AskRx = mpsc::Receiver<(Ask, mpsc::Sender<Answer>)>;

/// Route connections' questions to this window. Call once.
pub(super) fn install_asker(ctx: &egui::Context) -> AskRx {
    let (tx, rx) = mpsc::channel::<(Ask, mpsc::Sender<Answer>)>();
    let tx = std::sync::Mutex::new(tx);
    let ctx = ctx.clone();
    remote::set_asker(move |q| {
        let (rtx, rrx) = mpsc::channel();
        if tx.lock().unwrap().send((q, rtx)).is_err() {
            return Answer::Cancel;
        }
        ctx.request_repaint();
        // Nobody answering for a long time counts as Cancel, so a worker isn't stuck for ever.
        rrx.recv_timeout(Duration::from_secs(15 * 60)).unwrap_or(Answer::Cancel)
    });
    rx
}

/// The Connect to Server box.
#[derive(Default)]
pub(super) struct ConnectBox {
    /// None: a network share through the system (SMB). Otherwise one of dirmap's own.
    pub kind: Option<Scheme>,
    pub host: String,
    pub port: String,
    pub user: String,
    pub folder: String,
    pub key: String,
    pub name: String,
    /// The saved server being edited.
    pub editing: Option<usize>,
    pub target: String,
    /// Windows: the drive letter ("" for the first free one) and whether to reconnect at sign-in.
    pub letter: String,
    pub remember: bool,
    /// Also add it to the saved servers.
    pub save: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub fresh: bool,
    /// The server to save once connected.
    pub pending: Option<SavedSite>,
}

impl ConnectBox {
    /// The box filled in from a saved server, to change it.
    pub(super) fn editing(i: usize, s: &SavedSite) -> ConnectBox {
        let mut c = ConnectBox { editing: Some(i), save: true, fresh: true, name: s.name.clone(), key: s.key.as_ref().map(|k| k.display().to_string()).unwrap_or_default(), ..Default::default() };
        if let Some((site, folder)) = s.path().and_then(|p| remote::split(&p)) {
            c.kind = Some(site.scheme);
            c.host = site.host;
            c.user = site.user;
            c.port = if site.port == site.scheme.default_port() { String::new() } else { site.port.to_string() };
            c.folder = if folder == "/" { String::new() } else { folder };
        }
        c
    }

    /// The address the fields make, or why they don't make one.
    pub(super) fn url(&mut self) -> Result<PathBuf, String> {
        let kind = self.kind.ok_or("")?;
        // A whole address pasted into the server field fills in the rest.
        if self.host.contains("://")
            && let Some((site, folder)) = remote::parse(&self.host).and_then(|p| remote::split(&p))
        {
            self.kind = Some(site.scheme);
            self.host = site.host;
            self.user = site.user;
            self.port = if site.port == site.scheme.default_port() { String::new() } else { site.port.to_string() };
            self.folder = if folder == "/" { String::new() } else { folder };
            return self.url();
        }
        let host = self.host.trim();
        if host.is_empty() {
            return Err("Type the server's name or address.".into());
        }
        let port = match self.port.trim() {
            "" => kind.default_port(),
            p => p.parse::<u16>().map_err(|_| "The port is a number from 1 to 65535.".to_string())?,
        };
        let user = self.user.trim();
        let user = if user.is_empty() { String::new() } else { format!("{user}@") };
        let folder = self.folder.trim().trim_start_matches('/');
        remote::parse(&format!("{}://{user}{host}:{port}/{folder}", kind.prefix())).ok_or_else(|| "That isn't a server address Filescape can read.".into())
    }
}

impl App {
    /// Read the system's network locations in the background.
    pub(super) fn net_refresh(&mut self) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(netdrives::list());
        });
        self.files.net_rx = Some(rx);
    }

    pub(super) fn net_poll(&mut self, ctx: &egui::Context) {
        if self.files.net_rx.is_none() && self.files.net.is_empty() && !self.files.net_asked {
            self.files.net_asked = true;
            self.net_refresh();
        }
        if let Some(rx) = &self.files.net_rx {
            match rx.try_recv() {
                Ok(n) => {
                    self.files.net = n;
                    self.files.net_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
                Err(mpsc::TryRecvError::Disconnected) => self.files.net_rx = None,
            }
        }
        let mut done = Vec::new();
        self.files.net_jobs.retain(|rx| match rx.try_recv() {
            Ok(r) => {
                done.push(r);
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => false,
        });
        if !self.files.net_jobs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        for r in done {
            match r {
                Ok(Some(path)) => {
                    // Connected: open it, and keep the Connect box's choice to save it.
                    if let Some(c) = self.files.connect.take() {
                        if let Some(site) = c.pending {
                            match c.editing {
                                Some(i) if i < self.files.prefs.sites.len() => self.files.prefs.sites[i] = site,
                                _ if c.save => self.files.prefs.sites.push(site),
                                _ => {}
                            }
                        } else if c.save && !self.files.prefs.servers.contains(&path) {
                            self.files.prefs.servers.push(path.clone());
                        }
                    }
                    self.ops.notice = Some((format!("Connected: {}", path.shown()), false));
                    // The Remote workspace said which pane it's for.
                    match self.files.rws.target.take().filter(|&p| p < self.files.panes.len() && remote::is_remote(&path)) {
                        Some(p) => {
                            self.files.set_focus(p);
                            self.files.go(Location::Dir(path));
                        }
                        None => self.files.go(Location::Dir(path)),
                    }
                }
                Ok(None) => self.ops.notice = Some(("Disconnected.".into(), false)),
                Err(e) => {
                    self.files.rws.target = None;
                    if let Some(c) = self.files.connect.as_mut() {
                        c.busy = false;
                        c.error = Some(e.clone());
                    } else {
                        self.ops.notice = Some((e, true));
                    }
                }
            }
            self.refresh_drives();
            self.net_refresh();
        }
    }

    fn net_job(&mut self, f: impl FnOnce() -> Result<Option<PathBuf>, String> + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        self.files.net_jobs.push(rx);
    }

    /// Network & Remote: network drives, the system's network locations and saved servers.
    pub(super) fn network_section(&mut self, ui: &mut egui::Ui, here: &Location, picks: &mut FPicks) {
        let pal = style::pal(ui);
        // Network drives the drive list found (connected), then what the system remembers.
        let mut rows: Vec<(PathBuf, String, String, bool, Option<f32>)> = Vec::new();
        for d in self.drives.iter().filter(|d| d.network) {
            let used = d.total.saturating_sub(d.free);
            let remote = self.files.net.iter().find(|n| n.path == d.root).map(|n| n.remote.clone()).unwrap_or_default();
            let tip = format!("{}{}\n{} free of {}", d.root.display(), if remote.is_empty() { String::new() } else { format!(" ({remote})") }, scan::human(d.free), scan::human(d.total));
            rows.push((d.root.clone(), drive_name(&d.root, &d.label, &remote), tip, false, (d.total > 0).then(|| used as f32 / d.total as f32)));
        }
        for n in &self.files.net {
            if rows.iter().any(|r| r.0 == n.path) {
                continue;
            }
            let state = if n.disconnected { "Not connected: click to reconnect" } else { "Connected" };
            rows.push((n.path.clone(), drive_name(&n.path, "", &n.remote), format!("{}\n{}\n{state}", n.path.display(), n.remote), n.disconnected, None));
        }
        let servers = self.files.prefs.servers.clone();
        // The Remote workspace lists saved sites in its own Sites section.
        let sites = if self.files.prefs.workspace == files::Workspace::Remote { Vec::new() } else { self.files.prefs.sites.clone() };
        let mut remove_server = None;
        let mut disconnect = None;
        let (mut open_site, mut open_site_tab, mut edit_site, mut remove_site) = (None, None, None, None);
        side_section_net(ui, |ui| {
            for (path, name, tip, off, frac) in &rows {
                let icon = if *off { "⚠" } else { "🖧" };
                let resp = files::side_row(ui, icon, name, *here == Location::Dir(path.clone()), if frac.is_some() { 30.0 } else { 24.0 }).on_hover_text(tip.as_str());
                if *off {
                    ui.painter().text(resp.rect.right_center() - vec2(8.0, 0.0), Align2::RIGHT_CENTER, "offline", FontId::proportional(10.5), pal.weak);
                }
                if let Some(f) = frac {
                    files::capacity_bar(ui, resp.rect, Some(*f));
                }
                self.place_row(ui, &resp, path, picks);
                resp.context_menu(|ui| {
                    if ui.add(egui::Button::new(style::mi("Disconnect"))).on_hover_text("Unmap or unmount it. Saved locations stay in the list.").clicked() {
                        disconnect = Some(path.clone());
                    }
                });
            }
            for (i, s) in servers.iter().enumerate() {
                if rows.iter().any(|r| &r.0 == s) {
                    continue;
                }
                let name = dnd::folder_name(s);
                let resp = files::side_row(ui, "🌐", &name, *here == Location::Dir(s.clone()), 24.0).on_hover_text(format!("{}\nSaved location", s.display()));
                self.place_row(ui, &resp, s, picks);
                resp.context_menu(|ui| {
                    if ui.add(egui::Button::new(style::mi("Remove from This List"))).clicked() {
                        remove_server = Some(i);
                    }
                });
            }
            let connected = remote::connected();
            for (i, site) in sites.iter().enumerate() {
                let Some(path) = site.path() else { continue };
                let on = remote::split(&path).is_some_and(|(s, _)| connected.contains(&s.url()));
                let at = here.dir().and_then(remote::split).zip(remote::split(&path)).is_some_and(|((a, _), (b, _))| a == b);
                let resp = files::side_row(ui, "🌐", &site.shown(), at, 24.0).on_hover_text(format!(
                    "{}\n{}{}",
                    site.url,
                    if on { "Connected" } else { "Click to connect" },
                    if path_scheme(&path).is_some_and(Scheme::insecure) { "\nNot encrypted: the password and files can be read on the network." } else { "" }
                ));
                if on {
                    ui.painter().circle_filled(resp.rect.right_center() - vec2(10.0, 0.0), 3.0, pal.accent);
                }
                if resp.clicked() {
                    open_site = Some(i);
                }
                if resp.middle_clicked() {
                    open_site_tab = Some(i);
                }
                if let Some(d) = self.folder_drop(ui, &resp, &path) {
                    picks.drop = Some(d);
                }
                resp.context_menu(|ui| {
                    if ui.add(egui::Button::new(style::mi("Open in New Tab"))).clicked() {
                        open_site_tab = Some(i);
                        ui.close();
                    }
                    if ui.add(egui::Button::new(style::mi("Edit…"))).clicked() {
                        edit_site = Some(i);
                        ui.close();
                    }
                    if on && ui.add(egui::Button::new(style::mi("Disconnect"))).clicked() {
                        remote::disconnect(&path);
                        ui.close();
                    }
                    if ui.add(egui::Button::new(style::mi("Forget Saved Password"))).on_hover_text("Remove its password from the system keychain, if one was kept there").clicked() {
                        let url = site.url.clone();
                        std::thread::spawn(move || remote::forget_password(&url));
                        self.ops.notice = Some(("Any saved password for it was removed from the keychain.".into(), false));
                        ui.close();
                    }
                    ui.separator();
                    if ui.add(egui::Button::new(style::mi("Remove from This List"))).clicked() {
                        remove_site = Some(i);
                        ui.close();
                    }
                });
            }
            if rows.is_empty() && servers.is_empty() && sites.is_empty() {
                ui.label(egui::RichText::new("No network locations").italics().color(pal.weak));
            }
            ui.add_space(2.0);
            if ui
                .add(egui::Button::new(egui::RichText::new("✚ Connect to Server…").small().color(pal.weak)).frame_when_inactive(false))
                .on_hover_text(if cfg!(windows) { "Map a network share (\\\\server\\share) to a drive letter, or connect to an SFTP, FTP or WebDAV server" } else { "Mount a network share (smb://server/share), or connect to an SFTP, FTP or WebDAV server" })
                .clicked()
            {
                self.files.connect = Some(ConnectBox { remember: true, save: true, fresh: true, ..Default::default() });
            }
        });
        if let Some(i) = remove_server {
            self.files.prefs.servers.remove(i);
        }
        if let Some(i) = open_site
            && let Some(p) = sites[i].path()
        {
            self.open_server(p);
        }
        if let Some(i) = open_site_tab
            && let Some(p) = sites[i].path()
        {
            self.files.new_tab(Location::Dir(p));
        }
        if let Some(i) = edit_site {
            self.files.connect = Some(ConnectBox::editing(i, &sites[i]));
        }
        if let Some(i) = remove_site {
            self.files.prefs.sites.remove(i);
        }
        if let Some(p) = disconnect {
            self.net_job(move || netdrives::disconnect(&p).map(|_| None));
        }
    }

    /// Connect to a server in the background (asking for what's needed), then open it: the
    /// folder in its address, or the account's own folder.
    pub(super) fn open_server(&mut self, path: PathBuf) {
        if !self.premium(crate::licence::Feature::Remote) {
            return;
        }
        let who = remote::split(&path).map(|(s, _)| s.short()).unwrap_or_default();
        self.ops.notice = Some((format!("Connecting to {who}…"), false));
        self.net_job(move || remote::connect(&path).map(Some));
    }

    /// Tell connections which key file each saved server uses. Cheap; no network.
    pub(super) fn site_options(&self) {
        for s in &self.files.prefs.sites {
            remote::set_options(&s.url, s.options());
        }
    }

    /// The Connect to Server box. Call once per frame.
    pub(super) fn connect_window(&mut self, ctx: &egui::Context) {
        self.ask_window(ctx);
        let Some(c) = self.files.connect.as_mut() else { return };
        let mut go = false;
        let mut cancel = false;
        let mut browse_key = false;
        egui::Modal::new(egui::Id::new("connect-server")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading(if c.editing.is_some() { "Edit Server" } else { "Connect to Server" });
            ui.add_space(6.0);
            let kind_name = |k: Option<Scheme>| k.map(Scheme::title).unwrap_or(if cfg!(windows) { "Network share (SMB, through Windows)" } else { "Network share (SMB, through the system)" });
            ui.add_enabled_ui(!c.busy && c.editing.is_none(), |ui| {
                egui::ComboBox::from_id_salt("connect-kind").width(300.0).selected_text(kind_name(c.kind)).show_ui(ui, |ui| {
                    ui.selectable_value(&mut c.kind, None, kind_name(None));
                    for k in Scheme::ALL {
                        ui.selectable_value(&mut c.kind, Some(k), kind_name(Some(k)));
                    }
                });
            });
            ui.add_space(6.0);
            let enter = |ui: &egui::Ui, r: &egui::Response| r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            match c.kind {
                None => {
                    let hint = if cfg!(windows) { r"\\server\share" } else { "smb://server/share" };
                    let r = ui.add(egui::TextEdit::singleline(&mut c.target).hint_text(hint).desired_width(f32::INFINITY));
                    if std::mem::take(&mut c.fresh) {
                        r.request_focus();
                    }
                    go = enter(ui, &r);
                    ui.add_space(4.0);
                    if cfg!(windows) {
                        ui.horizontal(|ui| {
                            ui.label("Drive letter");
                            ui.add(egui::TextEdit::singleline(&mut c.letter).desired_width(28.0).char_limit(1).hint_text("Z"));
                            ui.weak("(empty: the last free letter)");
                        });
                        ui.checkbox(&mut c.remember, "Reconnect when I sign in");
                    }
                    ui.checkbox(&mut c.save, "Add to Network & Remote");
                    ui.add_space(4.0);
                    ui.weak(if cfg!(windows) {
                        "Windows asks for a user name and password if the share needs one, and can remember it in Credential Manager. Filescape never sees it."
                    } else if cfg!(target_os = "macos") {
                        "macOS asks for a password if the share needs one, and can keep it in your Keychain. Filescape never sees it."
                    } else {
                        "Mounted through GVFS. If the share needs a password, connect once from your desktop's file manager so it's saved in your keyring."
                    });
                }
                Some(kind) => {
                    let port_hint = kind.default_port().to_string();
                    let user_hint = if kind == Scheme::Sftp { remote::local_user() } else { "anonymous".into() };
                    let mut focus_first = std::mem::take(&mut c.fresh);
                    egui::Grid::new("connect-fields").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                        ui.label("Server");
                        ui.horizontal(|ui| {
                            let r = ui.add(egui::TextEdit::singleline(&mut c.host).hint_text("render01.studio.lan, or a whole address").desired_width(250.0));
                            if std::mem::take(&mut focus_first) {
                                r.request_focus();
                            }
                            go |= enter(ui, &r);
                            ui.label("Port");
                            let r = ui.add(egui::TextEdit::singleline(&mut c.port).hint_text(port_hint).desired_width(52.0));
                            go |= enter(ui, &r);
                        });
                        ui.end_row();
                        ui.label("User name");
                        let r = ui.add(egui::TextEdit::singleline(&mut c.user).hint_text(user_hint).desired_width(250.0));
                        go |= enter(ui, &r);
                        ui.end_row();
                        ui.label("Folder");
                        let r = ui.add(egui::TextEdit::singleline(&mut c.folder).hint_text(if kind == Scheme::Sftp { "(your home folder)" } else { "/" }).desired_width(250.0));
                        go |= enter(ui, &r);
                        ui.end_row();
                        if kind == Scheme::Sftp {
                            ui.label("Key file");
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut c.key).hint_text("(the SSH agent, then ~/.ssh)").desired_width(250.0));
                                browse_key = ui.button("Choose…").clicked();
                            });
                            ui.end_row();
                        }
                        ui.label("Name");
                        ui.add(egui::TextEdit::singleline(&mut c.name).hint_text("(optional) shown in the sidebar").desired_width(250.0));
                        ui.end_row();
                    });
                    ui.add_space(4.0);
                    if c.editing.is_none() {
                        ui.checkbox(&mut c.save, "Add to Network & Remote");
                    }
                    ui.add_space(2.0);
                    if kind.insecure() {
                        ui.colored_label(ui.visuals().warn_fg_color, "Not encrypted: the password and the files can be read by anyone on the network between here and the server. Use it only on a network you trust.");
                    }
                    ui.weak(match kind {
                        Scheme::Sftp => "Filescape checks the server's identity, then signs in with your SSH agent or key files, or asks for a password.",
                        Scheme::Ftps => "The server's certificate must be one your system trusts.",
                        Scheme::Davs => "The server's certificate must be one your system trusts.",
                        _ => "",
                    });
                    ui.weak(format!("If a password is needed Filescape asks for it, and can keep it in {}. It is never saved in Filescape's own files.", keychain_name()));
                }
            }
            if let Some(e) = &c.error {
                ui.add_space(4.0);
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if c.busy {
                    ui.spinner();
                    ui.label("Connecting…");
                } else {
                    let ready = if c.kind.is_some() { !c.host.trim().is_empty() } else { !c.target.trim().is_empty() };
                    go |= ui.add_enabled(ready, egui::Button::new(if c.editing.is_some() { "Save and Connect" } else { "Connect" })).clicked();
                }
                cancel = ui.button(if c.busy { "Close" } else { "Cancel" }).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if browse_key {
            let start = crate::fsys::places::home().map(|h| h.join(".ssh"));
            let mut d = super::dialogs::Dialog::new().set_title("Choose an SSH key file");
            if let Some(s) = start.filter(|s| s.is_dir()) {
                d = d.set_directory(s);
            }
            super::dialogs::ask(&mut self.asking, d, super::dialogs::Pick::File, |app, f| {
                if let Some(c) = app.files.connect.as_mut() {
                    c.key = f.display().to_string();
                }
            });
        }
        if cancel {
            self.files.connect = None;
            return;
        }
        if !go || c.busy {
            return;
        }
        match c.kind {
            None if !c.target.trim().is_empty() => {
                c.busy = true;
                c.error = None;
                let (target, letter, remember) = (c.target.trim().to_string(), c.letter.trim().chars().next().filter(|c| c.is_ascii_alphabetic()), c.remember);
                if remote::parse(&target).is_some() {
                    // An sftp:// (or ftp://, dav://) address typed here.
                    c.busy = false;
                    c.host = target;
                    c.kind = Some(Scheme::Sftp);
                    let _ = c.url();
                    return;
                }
                self.net_job(move || netdrives::connect(&target, letter, remember).map(Some));
            }
            None => {}
            Some(_) => match c.url() {
                Ok(path) => {
                    let key = Some(c.key.trim()).filter(|k| !k.is_empty()).map(PathBuf::from);
                    let url = remote::show(&path).unwrap_or_default();
                    let mut site = match c.editing.and_then(|i| self.files.prefs.sites.get(i)) {
                        Some(old) => old.clone(),
                        None => SavedSite::default(),
                    };
                    (site.name, site.url, site.key) = (c.name.trim().to_string(), url.clone(), key);
                    remote::set_options(&url, site.options());
                    c.pending = Some(site);
                    c.busy = true;
                    c.error = None;
                    self.net_job(move || remote::connect(&path).map(Some));
                }
                Err(e) => c.error = Some(e),
            },
        }
    }

    /// A question from a connection: a new server's identity, a password, a passphrase.
    fn ask_window(&mut self, ctx: &egui::Context) {
        if self.files.asks.is_none() {
            self.files.asks = Some(install_asker(ctx));
            self.site_options();
        }
        if self.files.asking.is_none()
            && let Some(rx) = &self.files.asks
            && let Ok((ask, reply)) = rx.try_recv()
        {
            self.files.asking = Some(AskBox { ask, reply, text: String::new(), remember: false, fresh: true });
        }
        let Some(a) = self.files.asking.as_mut() else { return };
        let mut answer = None;
        egui::Modal::new(egui::Id::new("remote-ask")).show(ctx, |ui| {
            ui.set_width(460.0);
            let mono = |s: &str| egui::RichText::new(s).monospace();
            match &a.ask {
                Ask::NewHostKey { site, algorithm, fingerprint } => {
                    ui.heading("Connect to a new server?");
                    ui.add_space(6.0);
                    ui.label(format!("Filescape hasn't connected to {site} before. Before signing in, check that this is the server's key fingerprint (your server's administrator can tell you, or run ssh-keygen -lf on the server's key):"));
                    ui.add_space(6.0);
                    ui.label(mono(fingerprint));
                    ui.weak(algorithm.as_str());
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Connect and Remember").on_hover_text("Trust this key for this server from now on").clicked() {
                            answer = Some(Answer::Trust { always: true });
                        }
                        if ui.button("Connect Once").clicked() {
                            answer = Some(Answer::Trust { always: false });
                        }
                        if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            answer = Some(Answer::Cancel);
                        }
                    });
                }
                Ask::ChangedHostKey { site, algorithm, fingerprint, known_hosts } => {
                    ui.heading("The server's identity has changed");
                    ui.add_space(6.0);
                    ui.colored_label(ui.visuals().warn_fg_color, format!("{site} showed a different key from the one recorded in {known_hosts}. Someone could be intercepting the connection, so Filescape won't connect."));
                    ui.add_space(4.0);
                    ui.label("The key it showed now:");
                    ui.label(mono(fingerprint));
                    ui.weak(algorithm.as_str());
                    ui.add_space(4.0);
                    ui.label("If the server was reinstalled or its key was replaced on purpose, remove the old line from that file and connect again.");
                    ui.add_space(10.0);
                    if ui.button("Close").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter)) {
                        answer = Some(Answer::Cancel);
                    }
                }
                Ask::Insecure { site } => {
                    ui.heading("Connect without encryption?");
                    ui.add_space(6.0);
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        format!("{site} uses a protocol that isn't encrypted. Your password and the files can be read, and changed, by anyone on the network between here and the server."),
                    );
                    ui.add_space(4.0);
                    ui.label("Use it only on a network you trust. If the server offers SFTP, FTPS or https, use that instead.");
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Connect Anyway").on_hover_text("Don't ask again for this server while Filescape is open (saved servers remember it)").clicked() {
                            answer = Some(Answer::Trust { always: true });
                        }
                        if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            answer = Some(Answer::Cancel);
                        }
                    });
                }
                Ask::Overwrite { site, name } => {
                    ui.heading("Replace the server's copy?");
                    ui.add_space(6.0);
                    ui.label(format!("You saved {name}, but its copy on {site} has changed since you opened it, perhaps by someone else."));
                    ui.add_space(4.0);
                    ui.label("Sending yours replaces those changes.");
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Replace It").clicked() {
                            answer = Some(Answer::Trust { always: false });
                        }
                        if ui.button("Don't Send").on_hover_text("Keep the server's copy; your saved copy stays here. Save again to be asked again").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            answer = Some(Answer::Cancel);
                        }
                    });
                }
                Ask::Password { site, retry } | Ask::Passphrase { key: site, retry } => {
                    let passphrase = matches!(a.ask, Ask::Passphrase { .. });
                    ui.heading(if passphrase { "Passphrase for SSH key" } else { "Sign in" });
                    ui.add_space(6.0);
                    if *retry {
                        ui.colored_label(ui.visuals().error_fg_color, if passphrase { "That passphrase didn't unlock the key. Try again." } else { "That password didn't work. Try again." });
                    }
                    ui.label(if passphrase { format!("The key {site} is locked with a passphrase.") } else { format!("Password for {site}") });
                    let r = ui.add(egui::TextEdit::singleline(&mut a.text).password(true).desired_width(f32::INFINITY));
                    if std::mem::take(&mut a.fresh) {
                        r.request_focus();
                    }
                    ui.checkbox(&mut a.remember, format!("Remember in {}", keychain_name()));
                    ui.add_space(10.0);
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        if ui.button(if passphrase { "Unlock" } else { "Sign In" }).clicked() || enter {
                            answer = Some(Answer::Secret { text: std::mem::take(&mut a.text), remember: a.remember });
                        }
                        if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            answer = Some(Answer::Cancel);
                        }
                    });
                }
            }
        });
        if let Some(ans) = answer {
            let a = self.files.asking.take().unwrap();
            if let (Ask::Insecure { site }, Answer::Trust { .. }) = (&a.ask, &ans) {
                for s in self.files.prefs.sites.iter_mut() {
                    if s.path().and_then(|p| remote::site_url(&p)).as_deref() == Some(site.as_str()) {
                        s.allow_insecure = true;
                        remote::set_options(&s.url, s.options());
                    }
                }
            }
            let _ = a.reply.send(ans);
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            });
        }
    }
}

/// The system keychain's name, as people know it.
pub(super) fn keychain_name() -> &'static str {
    if cfg!(windows) {
        "Windows Credential Manager"
    } else if cfg!(target_os = "macos") {
        "your macOS Keychain"
    } else {
        "your desktop's keyring"
    }
}

fn path_scheme(p: &Path) -> Option<Scheme> {
    remote::split(p).map(|(s, _)| s.scheme)
}

/// "Media (Z:)", "nas/media" or a mount's name.
fn drive_name(root: &Path, label: &str, remote: &str) -> String {
    let r = root.shown();
    let r = r.trim_end_matches(['\\', '/']);
    let short = remote.trim_start_matches(r"\\").trim_start_matches("smb://").replace('\\', "/");
    if cfg!(windows) && r.len() == 2 {
        let what = if !label.is_empty() { label.to_string() } else { short };
        if what.is_empty() { r.to_string() } else { format!("{what} ({r})") }
    } else if !short.is_empty() {
        short
    } else {
        dnd::folder_name(root)
    }
}

/// The section title, folding like the others.
fn side_section_net(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    files::side_section(ui, "fm-net", "Network & Remote", true, add);
}

