//! Open With: the user's own list of programs, offered for the file types each is set for.
//!
//! A program is stored as its path and a list of arguments, one of which is `{file}`. The
//! file's path is passed as that one argument, never through a shell, so names with spaces,
//! quotes or `;` can't break the command. Files on servers or inside archives are copied out
//! first, as Open does.

use super::App;
use crate::fsys::{archive, remote};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct UserApp {
    pub name: String,
    pub exe: PathBuf,
    /// Arguments; `{file}` is replaced by the item's path (added at the end if it's missing).
    pub args: Vec<String>,
    /// Extensions it's offered for (`exr`, `nk`), `folder` for folders, or empty for everything.
    pub types: Vec<String>,
}

impl Default for UserApp {
    fn default() -> Self {
        UserApp {
            name: String::new(),
            exe: PathBuf::new(),
            args: vec!["{file}".into()],
            types: Vec::new(),
        }
    }
}

impl UserApp {
    pub fn fits(&self, ext: &str, is_dir: bool) -> bool {
        if self.types.is_empty() {
            return true;
        }
        let want = if is_dir {
            "folder".to_string()
        } else {
            ext.trim_start_matches('.').to_lowercase()
        };
        self.types.iter().any(|t| {
            t.trim().trim_start_matches('.').eq_ignore_ascii_case(&want) || t.trim() == "*"
        })
    }

    /// The command line, as program and separate arguments.
    pub fn command(&self, file: &Path) -> (PathBuf, Vec<std::ffi::OsString>) {
        let mut args: Vec<std::ffi::OsString> = Vec::new();
        let mut placed = false;
        for a in &self.args {
            if a.contains("{file}") {
                placed = true;
                if a == "{file}" {
                    args.push(file.as_os_str().to_owned());
                } else {
                    args.push(a.replace("{file}", &file.to_string_lossy()).into());
                }
            } else {
                args.push(a.into());
            }
        }
        if !placed {
            args.push(file.as_os_str().to_owned());
        }
        // A Mac app bundle is started through `open -a`.
        if cfg!(target_os = "macos") && self.exe.extension().is_some_and(|e| e == "app") {
            let mut a: Vec<std::ffi::OsString> = vec!["-a".into(), self.exe.as_os_str().to_owned()];
            if !args.is_empty() {
                // The file first; anything else after --args.
                let f = args
                    .iter()
                    .position(|x| x == file.as_os_str())
                    .map(|i| args.remove(i));
                a.extend(f);
                if !args.is_empty() {
                    a.push("--args".into());
                    a.extend(args);
                }
            }
            return (PathBuf::from("open"), a);
        }
        (self.exe.clone(), args)
    }

    pub fn run(&self, file: &Path) -> Result<(), String> {
        let local = local_copies(&[file.to_path_buf()])?.remove(0);
        let (exe, args) = self.command(&local);
        let mut cmd = std::process::Command::new(&exe);
        cmd.args(&args);
        if let Some(dir) = local.parent().filter(|_| !local.is_dir()) {
            cmd.current_dir(dir);
        } else {
            cmd.current_dir(&local);
        }
        cmd.spawn()
            .map(drop)
            .map_err(|e| format!("Couldn't start {}: {e}", self.name))
    }
}

/// The Manage Apps box.
#[derive(Default)]
pub(super) struct AppsBox {
    pub apps: Vec<UserApp>,
    /// Text being edited: (args, types) per app.
    pub text: Vec<(String, String)>,
}

impl AppsBox {
    pub fn new(apps: &[UserApp]) -> Self {
        AppsBox {
            apps: apps.to_vec(),
            text: apps
                .iter()
                .map(|a| (a.args.join(" "), a.types.join(", ")))
                .collect(),
        }
    }
}

fn app_name(exe: &Path) -> String {
    exe.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| exe.display().to_string())
}

/// Icons of apps in the Open With menu, small, read on worker threads and kept for the session.
mod icons {
    use eframe::egui;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    enum State {
        Loading,
        Pixels(Option<(u32, u32, Vec<u8>)>),
        Ready(Option<egui::TextureHandle>),
    }

    fn all() -> &'static Mutex<HashMap<PathBuf, State>> {
        static A: OnceLock<Mutex<HashMap<PathBuf, State>>> = OnceLock::new();
        A.get_or_init(Default::default)
    }

    /// The app's icon, or None while it loads (or when it has none).
    pub fn get(ctx: &egui::Context, app: &Path) -> Option<egui::TextureHandle> {
        let mut m = all().lock().unwrap_or_else(|e| e.into_inner());
        match m.get_mut(app) {
            Some(State::Ready(t)) => return t.clone(),
            Some(State::Loading) => return None,
            Some(s @ State::Pixels(_)) => {
                let State::Pixels(px) = std::mem::replace(s, State::Loading) else { unreachable!() };
                let tex = px.map(|(w, h, rgba)| {
                    let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                    ctx.load_texture(format!("open-with:{}", app.display()), img, egui::TextureOptions::LINEAR)
                });
                *s = State::Ready(tex.clone());
                return tex;
            }
            None => {}
        }
        m.insert(app.to_path_buf(), State::Loading);
        drop(m);
        let (ctx, app) = (ctx.clone(), app.to_path_buf());
        let _ = std::thread::Builder::new().name("open-with-icon".into()).spawn(move || {
            let px = crate::apps::icon(&app, 32);
            all().lock().unwrap_or_else(|e| e.into_inner()).insert(app, State::Pixels(px));
            ctx.request_repaint();
        });
        None
    }
}

/// The app chosen from a menu (menus pass plain values; this holds the rest).
static PICKED: std::sync::Mutex<Option<crate::openapps::Handler>> = std::sync::Mutex::new(None);

/// At most this many apps are listed; the rest are a click away under Choose.
const MENU_APPS: usize = 12;

/// One app's row in a menu: its icon and name, and "Default" for the system's choice.
pub(super) fn app_row(ui: &mut egui::Ui, h: &crate::openapps::Handler) -> egui::Response {
    let icon = icons::get(ui.ctx(), &h.app);
    let text = crate::app::style::mi(&h.name);
    let mut b = match icon {
        Some(t) => egui::Button::image_and_text(egui::Image::new(&t).fit_to_exact_size(egui::vec2(16.0, 16.0)), h.name.as_str()),
        None => egui::Button::new(text),
    };
    if h.default {
        b = b.shortcut_text("Default");
    }
    ui.add(b).on_hover_text(h.app.display().to_string())
}

/// The Open With submenu: the apps the system knows can open this kind of item (the default
/// first), the apps added by hand, then Choose and Manage.
pub(super) fn menu(ui: &mut egui::Ui, apps: &[UserApp], path: &Path, ext: &str, is_dir: bool) -> Option<Pick> {
    let mut out = None;
    ui.menu_button(crate::app::style::mi("Open With"), |ui| {
        let t = crate::openapps::identify(path, is_dir);
        let ctx = ui.ctx().clone();
        let found = crate::openapps::apps_for(&t, move || ctx.request_repaint());
        // Apps on another system (a Windows program on a Mac drive) are never offered.
        let on_system = match &found {
            crate::openapps::Apps::Found(f) => Some(f.clone()),
            crate::openapps::Apps::Looking => None,
        };
        match &on_system {
            None => {
                ui.weak("Looking for apps…");
            }
            Some(list) => {
                for h in list.iter().take(MENU_APPS) {
                    if app_row(ui, h).clicked() {
                        *PICKED.lock().unwrap_or_else(|e| e.into_inner()) = Some(h.clone());
                        out = Some(Pick::System);
                    }
                }
            }
        }
        let fit: Vec<(usize, &UserApp)> = apps
            .iter()
            .enumerate()
            .filter(|(_, a)| a.fits(ext, is_dir))
            .collect();
        if !fit.is_empty() && on_system.as_ref().is_some_and(|l| !l.is_empty()) {
            ui.separator();
        }
        for (i, a) in &fit {
            if ui
                .button(&a.name)
                .on_hover_text(format!("Added by you: {}", a.exe.display()))
                .clicked()
            {
                out = Some(Pick::Run(*i));
            }
        }
        if fit.is_empty() && on_system.as_ref().is_some_and(|l| l.is_empty()) {
            ui.weak("No apps found for this type");
        }
        ui.separator();
        let what = if t == crate::openapps::Type::Unknown && !ext.is_empty() { format!(".{ext} files") } else { t.label() };
        if ui.add(egui::Button::new(crate::app::style::mi(format!("Choose an App for {what}…")))).clicked() {
            out = Some(Pick::Choose);
        }
        if ui.add(egui::Button::new(crate::app::style::mi("Manage Apps…"))).clicked() {
            out = Some(Pick::Manage);
        }
    });
    out
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Pick {
    Run(usize),
    /// An app the system found (the one in [`PICKED`]).
    System,
    Choose,
    Manage,
}

/// Copies of files on servers or inside archives, made on this computer (as Open does); files
/// here are used as they are.
fn local_copies(files: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    files
        .iter()
        .map(|file| {
            if archive::inside(file) {
                super::files::archive_temp(file).map_err(|e| format!("Couldn't take it out of the archive: {e}"))
            } else if remote::is_remote(file) {
                remote::edit::open_copy(file).map_err(|e| format!("Couldn't fetch it from the server: {}", remote::describe(&e)))
            } else {
                Ok(file.to_path_buf())
            }
        })
        .collect()
}

impl App {
    pub(super) fn open_with(&mut self, pick: Pick, paths: Vec<PathBuf>) {
        let Some(first) = paths.first().cloned() else {
            return;
        };
        match pick {
            Pick::Run(i) => {
                let Some(app) = self.files.prefs.apps.get(i).cloned() else {
                    return;
                };
                self.files
                    .spawn(move || paths.iter().try_for_each(|p| app.run(p)));
            }
            Pick::System => {
                let Some(h) = PICKED.lock().unwrap_or_else(|e| e.into_inner()).take() else {
                    return;
                };
                self.files.spawn(move || {
                    let r = crate::openapps::open(&h, &local_copies(&paths)?);
                    // An app that's gone: look again next time.
                    if r.is_err() {
                        crate::openapps::forget();
                    }
                    r
                });
            }
            Pick::Choose => {
                let is_dir = self
                    .files
                    .cur()
                    .entry_named(
                        &first
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default(),
                    )
                    .is_some_and(|e| e.is_dir);
                let ext = if is_dir {
                    "folder".to_string()
                } else {
                    first
                        .file_name()
                        .map(|n| crate::scan::ext_of(&n.to_string_lossy()).to_lowercase())
                        .unwrap_or_default()
                };
                let mut dlg = super::dialogs::Dialog::new().set_title("Choose an app");
                if cfg!(windows) {
                    dlg = dlg.add_filter("Programs", &["exe", "bat", "cmd"]);
                }
                self.ask_path(dlg, super::dialogs::Pick::File, move |this, exe| {
                    let types = if ext.is_empty() {
                        Vec::new()
                    } else {
                        vec![ext]
                    };
                    let app = UserApp {
                        name: app_name(&exe),
                        exe,
                        args: vec!["{file}".into()],
                        types,
                    };
                    let i = match this
                        .files
                        .prefs
                        .apps
                        .iter()
                        .position(|a| a.exe == app.exe && a.args == app.args)
                    {
                        Some(i) => {
                            let a = &mut this.files.prefs.apps[i];
                            for t in app.types {
                                if !a.types.contains(&t) && !a.types.is_empty() {
                                    a.types.push(t);
                                }
                            }
                            i
                        }
                        None => {
                            this.files.prefs.apps.push(app);
                            this.files.prefs.apps.len() - 1
                        }
                    };
                    this.open_with(Pick::Run(i), paths);
                });
            }
            Pick::Manage => self.files.apps_box = Some(AppsBox::new(&self.files.prefs.apps)),
        }
    }

    pub(super) fn apps_window(&mut self, ctx: &egui::Context) {
        let Some(b) = &mut self.files.apps_box else {
            return;
        };
        let mut save = false;
        let mut cancel = false;
        let mut add = false;
        let mut remove = None;
        let mut choose = None;
        egui::Modal::new(egui::Id::new("apps-box")).show(ctx, |ui| {
            ui.set_width(640.0);
            ui.heading("Open With: Apps");
            ui.weak("Each app is offered on the right-click menu's Open With for the types listed (none: everything; \"folder\" for folders). {file} in the arguments is where the item's path goes.");
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                egui::Grid::new("apps-grid").num_columns(5).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.strong("Name");
                    ui.strong("Program");
                    ui.strong("Arguments");
                    ui.strong("Types");
                    ui.label("");
                    ui.end_row();
                    for (i, (a, (args, types))) in b.apps.iter_mut().zip(b.text.iter_mut()).enumerate() {
                        ui.add_sized([120.0, 22.0], egui::TextEdit::singleline(&mut a.name));
                        let shown = a.exe.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Choose…".into());
                        if ui.button(shown).on_hover_text(a.exe.display().to_string()).clicked() {
                            choose = Some(i);
                        }
                        ui.add_sized([170.0, 22.0], egui::TextEdit::singleline(args).hint_text("{file}"));
                        ui.add_sized([120.0, 22.0], egui::TextEdit::singleline(types).hint_text("everything"));
                        if ui.small_button("Remove").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
            });
            if b.apps.is_empty() {
                ui.weak("No apps yet.");
            }
            ui.add_space(6.0);
            add = ui.button("Add App…").clicked();
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                save = ui.button("Save").clicked();
                cancel = ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if let Some(i) = remove {
            b.apps.remove(i);
            b.text.remove(i);
        }
        if add {
            let d = super::dialogs::Dialog::new().set_title("Choose an app");
            self.ask_path(d, super::dialogs::Pick::File, |this, p| {
                if let Some(b) = &mut this.files.apps_box {
                    b.apps.push(UserApp {
                        name: app_name(&p),
                        exe: p,
                        ..Default::default()
                    });
                    b.text.push(("{file}".into(), String::new()));
                }
            });
            return;
        }
        if let Some(i) = choose {
            let d = super::dialogs::Dialog::new().set_title("Choose an app");
            self.ask_path(d, super::dialogs::Pick::File, move |this, p| {
                if let Some(a) = this.files.apps_box.as_mut().and_then(|b| b.apps.get_mut(i)) {
                    if a.name.is_empty() {
                        a.name = app_name(&p);
                    }
                    a.exe = p;
                }
            });
            return;
        }
        if cancel {
            self.files.apps_box = None;
        } else if save {
            let b = self.files.apps_box.take().unwrap();
            self.files.prefs.apps = b
                .apps
                .into_iter()
                .zip(b.text)
                .filter(|(a, _)| !a.exe.as_os_str().is_empty())
                .map(|(mut a, (args, types))| {
                    a.args = split_args(&args);
                    a.types = types
                        .split([',', ' '])
                        .map(|t| t.trim().trim_start_matches('.').to_lowercase())
                        .filter(|t| !t.is_empty())
                        .collect();
                    if a.name.trim().is_empty() {
                        a.name = app_name(&a.exe);
                    }
                    a
                })
                .collect();
        }
    }
}

/// Arguments typed on one line: split on spaces, with "double quotes" keeping spaces in one.
pub(super) fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_keep_the_path_whole() {
        let a = UserApp {
            name: "nuke".into(),
            exe: "/opt/nuke".into(),
            args: split_args(r#"--view "{file}" -x"#),
            types: vec!["nk".into(), "exr".into()],
        };
        assert_eq!(a.args, ["--view", "{file}", "-x"]);
        let f = Path::new("/a dir/x; rm -rf ~.exr");
        let (exe, args) = a.command(f);
        if !cfg!(target_os = "macos") {
            assert_eq!(exe, PathBuf::from("/opt/nuke"));
            assert_eq!(
                args,
                ["--view", "/a dir/x; rm -rf ~.exr", "-x"].map(std::ffi::OsString::from)
            );
        }
        assert!(a.fits("EXR", false) && !a.fits("png", false) && !a.fits("", true));
        let any = UserApp {
            args: vec![],
            ..a.clone()
        };
        assert_eq!(any.command(f).1.last().unwrap(), f.as_os_str());
        let folders = UserApp {
            types: vec!["folder".into()],
            ..a
        };
        assert!(folders.fits("", true) && !folders.fits("exr", false));
        assert!(UserApp::default().fits("anything", false));
    }
}
