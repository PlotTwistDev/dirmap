//! The sidebar's Network & Remote section and the Connect to Server box.
//!
//! It lists network drives and mounts (with their state), the server locations the user saved,
//! and connections the system remembers (Windows mapped drives, GVFS mounts on Linux). Connecting
//! and disconnecting go through the operating system on a worker thread, so a dead server shows
//! an error instead of freezing the window, and dirmap never sees a password.

use super::files::FPicks;
use super::*;
use crate::fsys::history::Location;
use crate::netdrives;
use std::path::Path;

/// The Connect to Server box.
#[derive(Default)]
pub(super) struct ConnectBox {
    pub target: String,
    /// Windows: the drive letter ("" for the first free one) and whether to reconnect at sign-in.
    pub letter: String,
    pub remember: bool,
    /// Also add it to the saved servers.
    pub save: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub fresh: bool,
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
                    if let Some(c) = self.files.connect.take()
                        && c.save
                        && !self.files.prefs.servers.contains(&path)
                    {
                        self.files.prefs.servers.push(path.clone());
                    }
                    self.ops.notice = Some((format!("Connected: {}", path.display()), false));
                    self.files.go(Location::Dir(path));
                }
                Ok(None) => self.ops.notice = Some(("Disconnected.".into(), false)),
                Err(e) => {
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
        let mut remove_server = None;
        let mut disconnect = None;
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
                    if ui.button("Disconnect").on_hover_text("Unmap or unmount it. Saved locations stay in the list.").clicked() {
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
                    if ui.button("Remove from This List").clicked() {
                        remove_server = Some(i);
                    }
                });
            }
            if rows.is_empty() && servers.is_empty() {
                ui.label(egui::RichText::new("No network locations").italics().color(pal.weak));
            }
            ui.add_space(2.0);
            if ui
                .add(egui::Button::new(egui::RichText::new("✚ Connect to Server…").small().color(pal.weak)).frame_when_inactive(false))
                .on_hover_text(if cfg!(windows) { "Map a network share (\\\\server\\share) to a drive letter" } else { "Mount a network share (smb://server/share)" })
                .clicked()
            {
                self.files.connect = Some(ConnectBox { remember: true, save: true, fresh: true, ..Default::default() });
            }
        });
        if let Some(i) = remove_server {
            self.files.prefs.servers.remove(i);
        }
        if let Some(p) = disconnect {
            self.net_job(move || netdrives::disconnect(&p).map(|_| None));
        }
    }

    /// The Connect to Server box. Call once per frame.
    pub(super) fn connect_window(&mut self, ctx: &egui::Context) {
        let Some(c) = self.files.connect.as_mut() else { return };
        let mut go = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::new("connect-server")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading("Connect to Server");
            ui.add_space(6.0);
            let hint = if cfg!(windows) { r"\\server\share" } else { "smb://server/share" };
            let r = ui.add(egui::TextEdit::singleline(&mut c.target).hint_text(hint).desired_width(f32::INFINITY));
            if std::mem::take(&mut c.fresh) {
                r.request_focus();
            }
            go = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
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
                "Windows asks for a user name and password if the share needs one, and can remember it in Credential Manager. dirmap never sees it."
            } else if cfg!(target_os = "macos") {
                "macOS asks for a password if the share needs one, and can keep it in your Keychain. dirmap never sees it."
            } else {
                "Mounted through GVFS. If the share needs a password, connect once from your desktop's file manager so it's saved in your keyring."
            });
            ui.weak("SFTP, FTP and WebDAV connections aren't built in yet.");
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
                    go |= ui.add_enabled(!c.target.trim().is_empty(), egui::Button::new("Connect")).clicked();
                }
                cancel = ui.button(if c.busy { "Close" } else { "Cancel" }).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if cancel {
            self.files.connect = None;
            return;
        }
        if go && !c.busy && !c.target.trim().is_empty() {
            c.busy = true;
            c.error = None;
            let (target, letter, remember) = (c.target.trim().to_string(), c.letter.trim().chars().next().filter(|c| c.is_ascii_alphabetic()), c.remember);
            self.net_job(move || netdrives::connect(&target, letter, remember).map(Some));
        }
    }
}

/// "Media (Z:)", "nas/media" or a mount's name.
fn drive_name(root: &Path, label: &str, remote: &str) -> String {
    let r = root.display().to_string();
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

