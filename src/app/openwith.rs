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
        let local = if archive::inside(file) {
            super::files::archive_temp(file)
                .map_err(|e| format!("Couldn't take it out of the archive: {e}"))?
        } else if remote::is_remote(file) {
            remote::fetch(file, 4 << 30).map_err(|e| {
                format!(
                    "Couldn't fetch it from the server: {}",
                    remote::describe(&e)
                )
            })?
        } else {
            file.to_path_buf()
        };
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

/// The Open With submenu: the apps for this kind of item, then Choose and Manage.
pub(super) fn menu(ui: &mut egui::Ui, apps: &[UserApp], ext: &str, is_dir: bool) -> Option<Pick> {
    let mut out = None;
    ui.menu_button(crate::app::style::mi("Open With"), |ui| {
        let fit: Vec<(usize, &UserApp)> = apps
            .iter()
            .enumerate()
            .filter(|(_, a)| a.fits(ext, is_dir))
            .collect();
        for (i, a) in &fit {
            if ui
                .button(&a.name)
                .on_hover_text(a.exe.display().to_string())
                .clicked()
            {
                out = Some(Pick::Run(*i));
            }
        }
        if fit.is_empty() {
            ui.weak("No apps added for this type yet");
        }
        ui.separator();
        let what = if is_dir {
            "folders".to_string()
        } else if ext.is_empty() {
            "these".to_string()
        } else {
            format!(".{ext} files")
        };
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
    Choose,
    Manage,
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
                let mut dlg = rfd::FileDialog::new().set_title("Choose an app");
                if cfg!(windows) {
                    dlg = dlg.add_filter("Programs", &["exe", "bat", "cmd"]);
                }
                if let Some(exe) = dlg.pick_file() {
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
                    let i = match self
                        .files
                        .prefs
                        .apps
                        .iter()
                        .position(|a| a.exe == app.exe && a.args == app.args)
                    {
                        Some(i) => {
                            let a = &mut self.files.prefs.apps[i];
                            for t in app.types {
                                if !a.types.contains(&t) && !a.types.is_empty() {
                                    a.types.push(t);
                                }
                            }
                            i
                        }
                        None => {
                            self.files.prefs.apps.push(app);
                            self.files.prefs.apps.len() - 1
                        }
                    };
                    self.open_with(Pick::Run(i), paths);
                }
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
                        if ui.button(shown).on_hover_text(a.exe.display().to_string()).clicked()
                            && let Some(p) = rfd::FileDialog::new().set_title("Choose an app").pick_file()
                        {
                            if a.name.is_empty() {
                                a.name = app_name(&p);
                            }
                            a.exe = p;
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
        if add
            && let Some(p) = rfd::FileDialog::new()
                .set_title("Choose an app")
                .pick_file()
        {
            b.apps.push(UserApp {
                name: app_name(&p),
                exe: p,
                ..Default::default()
            });
            b.text.push(("{file}".into(), String::new()));
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
