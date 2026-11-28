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

fn pick(h: &crate::openapps::Handler, p: Pick) -> Option<Pick> {
    *PICKED.lock().unwrap_or_else(|e| e.into_inner()) = Some(h.clone());
    Some(p)
}

/// At most this many apps are listed; the rest are a click away under Choose.
const MENU_APPS: usize = 12;

use crate::openapps::Handler;
use crate::openapps::prefs::{OpenPrefs, Source, kind_key, kind_name};

/// Why an app is where it is in the list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Place {
    Default,
    Pinned,
    Recent,
    Other,
}

/// The list as shown: the system's default first, then pinned apps (in the order pinned), then
/// the ones used last for this type, then the rest by name.
pub(super) fn ordered<'a>(list: &'a [crate::openapps::Handler], prefs: &OpenPrefs, t: &crate::openapps::Type) -> Vec<(&'a crate::openapps::Handler, Place)> {
    let recent = t.key().and_then(|k| prefs.recent.get(&k)).cloned().unwrap_or_default();
    let mut out: Vec<(&crate::openapps::Handler, Place)> = list.iter().filter(|h| h.default).map(|h| (h, Place::Default)).collect();
    let add = |h: &'a crate::openapps::Handler, p: Place, out: &mut Vec<(&'a crate::openapps::Handler, Place)>| {
        if !out.iter().any(|(x, _)| x.id == h.id) {
            out.push((h, p));
        }
    };
    for id in &prefs.pinned {
        if let Some(h) = list.iter().find(|h| &h.id == id) {
            add(h, Place::Pinned, &mut out);
        }
    }
    for id in &recent {
        if let Some(h) = list.iter().find(|h| &h.id == id) {
            add(h, Place::Recent, &mut out);
        }
    }
    for h in list {
        add(h, Place::Other, &mut out);
    }
    out
}

/// What a file is, worked out once for the item being shown (reading the start of a file with
/// no extension every frame would be wasteful).
pub(super) fn type_of(path: &Path, is_dir: bool) -> crate::openapps::Type {
    type Seen = std::collections::HashMap<(PathBuf, bool), crate::openapps::Type>;
    thread_local! {
        static SEEN: std::cell::RefCell<Seen> = std::cell::RefCell::new(Seen::new());
    }
    SEEN.with(|s| {
        let mut s = s.borrow_mut();
        let key = (path.to_path_buf(), is_dir);
        if let Some(t) = s.get(&key) {
            return t.clone();
        }
        // A big selection is seen once; don't keep every file ever shown.
        if s.len() > 4096 {
            s.clear();
        }
        let t = crate::openapps::identify(path, is_dir);
        s.insert(key, t.clone());
        t
    })
}

/// Is this item a folder (only asked of the disk for items on this computer)?
fn is_folder(p: &Path) -> bool {
    !archive::inside(p) && !remote::is_remote(p) && p.is_dir()
}

/// The different types among several items, the first item's first.
pub(super) fn types_of(paths: &[PathBuf], first_is_dir: bool) -> Vec<crate::openapps::Type> {
    let mut out: Vec<crate::openapps::Type> = Vec::new();
    for (i, p) in paths.iter().enumerate() {
        let t = type_of(p, if i == 0 { first_is_dir } else { is_folder(p) });
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// The apps that open every type, from each type's list as shown (with `default` marking the
/// app it opens in): those in the first list that every other list has too, marked default
/// only where it's the default for all of them.
pub(super) fn common(lists: &[Vec<Handler>]) -> Vec<Handler> {
    let Some((first, rest)) = lists.split_first() else { return Vec::new() };
    first
        .iter()
        .filter(|h| rest.iter().all(|l| l.iter().any(|x| x.id == h.id)))
        .map(|h| Handler { default: h.default && rest.iter().all(|l| l.iter().any(|x| x.id == h.id && x.default)), ..h.clone() })
        .collect()
}

/// The apps for an item: the system's list, or None while it's still being looked up.
fn found_for(ctx: &egui::Context, t: &crate::openapps::Type) -> Option<std::sync::Arc<Vec<crate::openapps::Handler>>> {
    let ctx = ctx.clone();
    match crate::openapps::apps_for(t, move || ctx.request_repaint()) {
        crate::openapps::Apps::Found(f) => Some(f),
        crate::openapps::Apps::Looking => None,
    }
}

/// The apps listed for items of these types (those that open every one), and the system
/// default's id for the first; None while any type's apps are still being looked up.
fn suggested(ctx: &egui::Context, prefs: &OpenPrefs, types: &[crate::openapps::Type]) -> Option<(Vec<Handler>, Option<String>)> {
    let mut lists = Vec::new();
    let mut system = None;
    for (i, ty) in types.iter().enumerate() {
        let (list, sys) = prefs.listing(ty, &found_for(ctx, ty)?);
        if i == 0 {
            system = sys;
        }
        lists.push(list);
    }
    Some((common(&lists), system))
}

/// One app's row in a menu: its icon and name, "Default" for the system's choice, and a pin
/// (shown while pinned, or on hover) that keeps it near the top of every list.
fn app_row(ui: &mut egui::Ui, h: &crate::openapps::Handler, place: Place, system: bool, prefs: &OpenPrefs) -> Option<Pick> {
    let icon = icons::get(ui.ctx(), &h.app);
    let mut b = match icon {
        Some(t) => egui::Button::image_and_text(egui::Image::new(&t).fit_to_exact_size(egui::vec2(16.0, 16.0)), h.name.as_str()),
        None => egui::Button::new(crate::app::style::mi(&h.name)),
    };
    // Room for the tick or the pin at the end.
    b = b.shortcut_text(match place {
        Place::Default => "Default\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}",
        _ if system => "System default\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}",
        _ => "\u{a0}\u{a0}\u{a0}\u{a0}",
    });
    let row = ui.add(b).on_hover_text(h.app.display().to_string());
    let mut out = None;
    if row.clicked() {
        out = pick(h, Pick::System);
    }
    let end = egui::Rect::from_center_size(egui::pos2(row.rect.right() - 11.0, row.rect.center().y), egui::vec2(18.0, 18.0));
    if place == Place::Default {
        crate::kit::icons::paint(ui.painter(), end.shrink(3.0), crate::kit::Icon::Check, ui.visuals().text_color());
    } else {
        let pinned = prefs.is_pinned(&h.id);
        let r = end;
        let pin = ui.interact(r, row.id.with("pin"), egui::Sense::click());
        let tip = if pinned { "Unpin: stop keeping it near the top" } else { "Pin: keep it near the top of every Open With list" };
        let show = pinned || ui.rect_contains_pointer(row.rect);
        if show {
            let v = ui.visuals();
            let c = if pin.hovered() || pinned { v.strong_text_color() } else { v.weak_text_color() };
            crate::kit::icons::paint(ui.painter(), r.shrink(3.0), crate::kit::Icon::Pin, c);
        }
        if pin.on_hover_text(tip).clicked() {
            out = pick(h, Pick::Pin);
        }
    }
    out
}

/// The list shared by the right-click Open With menu and the Open button's arrow: the apps,
/// then the ones added by hand, then Choose and Manage.
pub(super) fn app_list(ui: &mut egui::Ui, apps: &[UserApp], prefs: &OpenPrefs, paths: &[PathBuf], ext: &str, is_dir: bool) -> Option<Pick> {
    let mut out = None;
    let types = types_of(paths, is_dir);
    let Some(t) = types.first().cloned() else { return None };
    let many = types.len() > 1;
    // Apps on another system (a Windows program on a Mac drive) are never offered.
    let (on_system, system) = match suggested(ui.ctx(), prefs, &types) {
        Some((l, s)) => (Some(l), s),
        None => (None, None),
    };
    match &on_system {
        None => {
            ui.weak("Looking for apps…");
        }
        Some(list) => {
            for (h, place) in ordered(list, prefs, &t).into_iter().take(MENU_APPS) {
                let sys = !many && place != Place::Default && system.as_deref() == Some(h.id.as_str());
                if let Some(p) = app_row(ui, h, place, sys, prefs) {
                    out = Some(p);
                }
            }
        }
    }
    let fits_all = |a: &UserApp| {
        a.fits(ext, is_dir)
            && paths.iter().skip(1).all(|p| {
                let e = p.file_name().map(|n| crate::scan::ext_of(&n.to_string_lossy()).to_lowercase()).unwrap_or_default();
                a.fits(&e, is_folder(p))
            })
    };
    let fit: Vec<(usize, &UserApp)> = apps.iter().enumerate().filter(|(_, a)| fits_all(a)).collect();
    if !fit.is_empty() && on_system.as_ref().is_some_and(|l| !l.is_empty()) {
        ui.separator();
    }
    for (i, a) in &fit {
        if ui.button(&a.name).on_hover_text(format!("Added by you: {}", a.exe.display())).clicked() {
            out = Some(Pick::Run(*i));
        }
    }
    if fit.is_empty() && on_system.as_ref().is_some_and(|l| l.is_empty()) {
        ui.weak(if many { "No app opens all of these" } else { "No apps found for this type" });
    }
    ui.separator();
    let what = if t == crate::openapps::Type::Unknown && !ext.is_empty() { format!(".{ext} files") } else { t.label() };
    if ui.add(egui::Button::new(crate::app::style::mi("Choose Another Application…"))).on_hover_text(format!("Search all your apps for one to open {what} with")).clicked() {
        out = Some(Pick::Choose);
    }
    if !many && t != crate::openapps::Type::Unknown && ui.add(egui::Button::new(crate::app::style::mi("Change Default Application…"))).on_hover_text(format!("Choose the app Filescape opens {what} in")).clicked() {
        out = Some(Pick::ChangeDefault);
    }
    if let Some((h, src)) = prefs.own_default(&t).filter(|_| !many) {
        let whose = match src {
            Source::Kind => t.category().map(kind_name).unwrap_or_default(),
            _ => what.clone(),
        };
        if ui.add(egui::Button::new(crate::app::style::mi("Use the System's Default Again"))).on_hover_text(format!("Stop opening {whose} in {} and use the system's usual app", h.name)).clicked() {
            out = Some(Pick::ResetDefault);
        }
    }
    if ui.add(egui::Button::new(crate::app::style::mi("Manage Applications…"))).clicked() {
        out = Some(Pick::Manage);
    }
    out
}

/// The Open With submenu of the right-click menu.
pub(super) fn menu(ui: &mut egui::Ui, apps: &[UserApp], prefs: &OpenPrefs, paths: &[PathBuf], ext: &str, is_dir: bool) -> Option<Pick> {
    ui.menu_button(crate::app::style::mi("Open With"), |ui| app_list(ui, apps, prefs, paths, ext, is_dir)).inner.flatten()
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Pick {
    Run(usize),
    /// An app the system found (the one in [`PICKED`]).
    System,
    /// Pin or unpin the app in [`PICKED`].
    Pin,
    /// Pick a program file to open it with.
    Browse,
    /// The searchable picker, set to make the app Filescape's default for the type.
    ChangeDefault,
    /// Forget Filescape's own default for the item's type (or kind).
    ResetDefault,
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

/// What Choose Another Application does with the app once it's picked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum How {
    /// Just open it this time.
    Once,
    /// Open it, and list the app for this type from now on.
    Remember,
    /// Make it Filescape's default for this type.
    TypeDefault,
    /// Make it Filescape's default for this whole kind of file.
    KindDefault,
}

/// The Choose Another Application box, while it's open.
pub(super) struct Picker {
    paths: Vec<PathBuf>,
    t: crate::openapps::Type,
    /// Every type among the items (several when they differ: then it can only open them).
    types: Vec<crate::openapps::Type>,
    search: String,
    chosen: Option<Handler>,
    how: How,
    /// Asking whether to change the system's default.
    confirm: bool,
    /// Put the cursor in the search box (the first frame).
    focus: bool,
    /// Change Default: save the choice without opening the item.
    just_set: bool,
}

/// A row in the picker: its icon and name, selected when it's the chosen one.
fn picker_row(ui: &mut egui::Ui, h: &Handler, chosen: bool) -> egui::Response {
    ui.horizontal(|ui| {
        let r = ui.allocate_response(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
        let v = ui.visuals();
        if chosen {
            ui.painter().rect_filled(r.rect, 4.0, v.selection.bg_fill);
        } else if r.hovered() {
            ui.painter().rect_filled(r.rect, 4.0, v.widgets.hovered.weak_bg_fill);
        }
        let mut x = r.rect.left() + 6.0;
        if let Some(t) = icons::get(ui.ctx(), &h.app) {
            egui::Image::new(&t).paint_at(ui, egui::Rect::from_min_size(egui::pos2(x, r.rect.center().y - 8.0), egui::vec2(16.0, 16.0)));
        }
        x += 24.0;
        let colour = if chosen { v.selection.stroke.color } else { v.text_color() };
        ui.painter().text(egui::pos2(x, r.rect.center().y), egui::Align2::LEFT_CENTER, &h.name, egui::FontId::proportional(13.5), colour);
        if h.default {
            ui.painter().text(egui::pos2(r.rect.right() - 8.0, r.rect.center().y), egui::Align2::RIGHT_CENTER, "Default", egui::FontId::proportional(12.0), v.weak_text_color());
        }
        r.on_hover_text(h.app.display().to_string())
    })
    .inner
}

impl App {
    /// A preview pane's Open button: "Open in <the usual app>" (just "Open" when the pane is
    /// narrow or no usual app is known), with an arrow for the same list as Open With.
    pub(super) fn open_button(&mut self, ui: &mut egui::Ui, path: &Path, is_dir: bool) {
        use crate::kit::{self, Icon};
        let t = type_of(path, is_dir);
        let found = found_for(ui.ctx(), &t);
        let def = self.settings.open_with.default_for(&t, found.as_deref().map(|v| v.as_slice())).map(|(h, _)| h);
        let full = def.as_ref().map(|h| format!("Open in {}", h.name));
        let width = |s: &str| ui.painter().layout_no_wrap(s.to_string(), egui::FontId::proportional(kit::LABEL), egui::Color32::WHITE).size().x;
        let label = match &full {
            Some(f) if width(f) + 76.0 <= ui.available_width() * 0.6 => f.as_str(),
            _ => "Open",
        };
        let tip = match &def {
            Some(h) => format!("Open it in {}, its usual app", h.name),
            None => "Open it with its usual app".to_string(),
        };
        let (main, arrow) = kit::split_button(ui, Some(Icon::External), label, "Open with another app");
        if main.on_hover_text(tip).clicked() {
            self.open_default(path.to_path_buf(), is_dir, found.as_deref().map(|v| v.as_slice()));
        }
        let ext = if is_dir { String::new() } else { path.file_name().map(|n| crate::scan::ext_of(&n.to_string_lossy()).to_lowercase()).unwrap_or_default() };
        let mut chosen = None;
        egui::Popup::menu(&arrow).show(|ui| {
            ui.set_min_width(220.0);
            chosen = app_list(ui, &self.files.prefs.apps, &self.settings.open_with, &[path.to_path_buf()], &ext, is_dir);
        });
        if let Some(p) = chosen {
            self.open_with(p, vec![path.to_path_buf()]);
        }
    }

    /// Open several files in their usual apps. Those Filescape has its own default for go to
    /// that app together (one copy of it, where it takes several); the rest go to the system.
    pub(super) fn open_files(&mut self, paths: Vec<PathBuf>) {
        let mut groups: Vec<(Handler, Vec<PathBuf>)> = Vec::new();
        let mut rest = Vec::new();
        for p in paths {
            match self.settings.open_with.own_default(&type_of(&p, false)) {
                Some((h, _)) => match groups.iter_mut().find(|(g, _)| g.id == h.id) {
                    Some((_, v)) => v.push(p),
                    None => groups.push((h, vec![p])),
                },
                None => rest.push(p),
            }
        }
        for (h, files) in groups {
            let _ = pick(&h, Pick::System);
            self.open_with(Pick::System, files);
        }
        for p in rest {
            self.open_file(p);
        }
    }

    /// Open an item in its usual app: Filescape's default for its type or kind, else the
    /// system's; with neither, ask which app (Choose Another Application).
    pub(super) fn open_default(&mut self, path: PathBuf, is_dir: bool, found: Option<&[Handler]>) {
        let t = type_of(&path, is_dir);
        match self.settings.open_with.default_for(&t, found) {
            Some((h, Source::Type | Source::Kind)) => {
                let _ = pick(&h, Pick::System);
                self.open_with(Pick::System, vec![path]);
            }
            None if found.is_some() || t == crate::openapps::Type::Unknown => self.open_with(Pick::Choose, vec![path]),
            _ => self.files.spawn(move || crate::actions::open(&path)),
        }
    }

    /// The Choose Another Application box: a search over the apps suggested for the type and
    /// every other app, what to do with the choice, and Open.
    pub(super) fn picker_window(&mut self, ctx: &egui::Context) {
        use crate::kit::{self, Icon, Kind};
        let Some(pk) = &mut self.files.picker else { return };
        let prefs = &self.settings.open_with;
        let mut close = false;
        let mut go = false;
        let mut browse = false;
        let mut system = false;
        let what = pk.t.label();
        let kind = pk.t.category();
        let name = pk.paths.first().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let found = suggested(ctx, prefs, &pk.types).is_some();
        let c2 = ctx.clone();
        let all = match crate::openapps::all_apps(&pk.t, move || c2.request_repaint()) {
            crate::openapps::Apps::Found(f) => Some(f),
            crate::openapps::Apps::Looking => None,
        };
        let modal = egui::Modal::new(egui::Id::new("open-with-picker")).show(ctx, |ui| {
            ui.set_width(540.0);
            ui.heading("Choose an Application");
            let many = pk.paths.len() > 1;
            ui.weak(if many { format!("Open {} items with:", pk.paths.len()) } else { format!("Open \u{201c}{name}\u{201d} with:") });
            ui.add_space(4.0);
            let search = ui.add(egui::TextEdit::singleline(&mut pk.search).hint_text("Search applications").desired_width(f32::INFINITY));
            if pk.focus {
                search.request_focus();
                pk.focus = !search.has_focus();
            }
            let q = pk.search.trim().to_lowercase();
            let fits = |h: &Handler| q.is_empty() || h.name.to_lowercase().contains(&q);
            ui.add_space(4.0);
            let mut first = None;
            egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, false]).show(ui, |ui| {
                let (suggested, _) = suggested(ctx, prefs, &pk.types).unwrap_or_default();
                let suggested: Vec<Handler> = ordered(&suggested, prefs, &pk.t).into_iter().map(|(h, _)| h.clone()).filter(|h| fits(h)).collect();
                if !found {
                    ui.weak("Looking for apps…");
                } else if !suggested.is_empty() {
                    ui.label(egui::RichText::new("Suggested").small().strong());
                    for h in &suggested {
                        first.get_or_insert_with(|| h.clone());
                        let r = picker_row(ui, h, pk.chosen.as_ref().is_some_and(|c| c.id == h.id));
                        if r.clicked() {
                            pk.chosen = Some(h.clone());
                        }
                        if r.double_clicked() {
                            go = true;
                        }
                    }
                }
                let others: Vec<Handler> = all.iter().flat_map(|a| a.iter()).filter(|h| fits(h) && !suggested.iter().any(|s| s.id == h.id)).cloned().collect();
                if !others.is_empty() {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(if suggested.is_empty() { "Applications" } else { "Other applications" }).small().strong());
                    for h in &others {
                        first.get_or_insert_with(|| h.clone());
                        let r = picker_row(ui, h, pk.chosen.as_ref().is_some_and(|c| c.id == h.id));
                        if r.clicked() {
                            pk.chosen = Some(h.clone());
                        }
                        if r.double_clicked() {
                            go = true;
                        }
                    }
                } else if all.is_none() {
                    ui.weak("Looking for other apps…");
                }
                if found && all.is_some() && suggested.is_empty() && others.is_empty() {
                    ui.weak(if q.is_empty() { "No apps found. Use Browse to pick a program." } else { "No apps match." });
                }
            });
            // Enter in the search box takes the first match.
            if search.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && let Some(h) = first {
                pk.chosen = Some(h);
                go = true;
            }
            ui.add_space(6.0);
            if pk.types.len() > 1 {
                ui.weak("These are different types of file, so the app opens them this once.");
            } else if pk.t != crate::openapps::Type::Unknown {
                ui.radio_value(&mut pk.how, How::Once, "Open once");
                ui.radio_value(&mut pk.how, How::Remember, format!("Open, and list it for {what} from now on"));
                ui.radio_value(&mut pk.how, How::TypeDefault, format!("Always open {what} with it in Filescape"));
                if let Some(c) = kind {
                    ui.radio_value(&mut pk.how, How::KindDefault, format!("Always open all {} with it in Filescape", kind_name(c)));
                }
                ui.weak("Filescape's choices don't change your system's default apps.");
            }
            ui.add_space(6.0);
            if pk.confirm {
                let app = pk.chosen.as_ref().map(|h| h.name.clone()).unwrap_or_default();
                ui.separator();
                ui.label(format!("Make {app} the default for {what} on this computer? Every app, not just Filescape, will open them with it."));
                ui.horizontal(|ui| {
                    if kit::button(ui, Kind::Primary, None, "Change System Default").clicked() {
                        system = true;
                    }
                    if kit::button(ui, Kind::Secondary, None, "Cancel").clicked() {
                        pk.confirm = false;
                    }
                });
                return;
            }
            ui.horizontal(|ui| {
                if kit::button(ui, Kind::Secondary, Some(Icon::Folder), "Browse…").on_hover_text("Pick a program file instead").clicked() {
                    browse = true;
                }
                let can_system = pk.chosen.is_some() && pk.t != crate::openapps::Type::Unknown && pk.types.len() == 1;
                let label = if crate::openapps::SETS_SYSTEM_DEFAULT { "Change System Default…" } else { "Change System Default (Windows)…" };
                let r = ui.add_enabled_ui(can_system, |ui| kit::button(ui, Kind::Secondary, None, label)).inner;
                if r.on_hover_text("Make it the default for every app on this computer, not just Filescape").clicked() {
                    if crate::openapps::SETS_SYSTEM_DEFAULT {
                        pk.confirm = true;
                    } else {
                        system = true;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let done = if pk.just_set && matches!(pk.how, How::TypeDefault | How::KindDefault) { "Use as Default" } else { "Open" };
                    let r = ui.add_enabled_ui(pk.chosen.is_some(), |ui| kit::button(ui, Kind::Primary, None, done)).inner;
                    if r.clicked() {
                        go = true;
                    }
                    if kit::button(ui, Kind::Secondary, None, "Cancel").clicked() {
                        close = true;
                    }
                });
            });
        });
        if modal.should_close() {
            close = true;
        }
        if system {
            let (Some(h), Some(sample)) = (pk.chosen.clone(), pk.paths.first().cloned()) else { return };
            let t = pk.t.clone();
            pk.confirm = false;
            self.files.spawn(move || crate::openapps::set_system_default(&h, &t, &sample));
            self.files.picker = None;
            return;
        }
        if browse {
            let p = self.files.picker.take().map(|p| p.paths).unwrap_or_default();
            self.open_with(Pick::Browse, p);
            return;
        }
        if go && let Some(pk) = self.files.picker.take() {
            let Some(h) = pk.chosen.clone() else { return };
            let t = pk.t.clone();
            let saved = Handler { default: false, ..h.clone() };
            let how = if pk.types.len() > 1 { How::Once } else { pk.how };
            match how {
                How::Once => {}
                How::Remember => self.assoc(|o| o.remember(&t, &saved)),
                How::TypeDefault => {
                    if let Some(k) = t.key() {
                        self.assoc(|o| {
                            o.defaults.insert(k.clone(), saved.clone());
                        });
                    }
                }
                How::KindDefault => {
                    if let Some(c) = t.category() {
                        self.assoc(|o| {
                            o.kind_defaults.insert(kind_key(c), saved.clone());
                        });
                    }
                }
            }
            if pk.just_set && matches!(how, How::TypeDefault | How::KindDefault) {
                return;
            }
            let _ = pick(&h, Pick::System);
            self.open_with(Pick::System, pk.paths);
            return;
        }
        if close {
            self.files.picker = None;
        }
    }

    /// Change Filescape's Open With choices, in the settings and in the Settings window's draft
    /// while it's open (so Apply there doesn't put back what was just changed).
    pub(super) fn assoc(&mut self, f: impl Fn(&mut OpenPrefs)) {
        f(&mut self.settings.open_with);
        if self.dialog.open {
            f(&mut self.dialog.draft.open_with);
        }
    }

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
                let is_dir = is_folder(&first);
                let types = types_of(&paths, is_dir);
                self.assoc(|o| types.iter().for_each(|t| o.used(t, &h.id)));
                self.files.spawn(move || {
                    let r = crate::openapps::open(&h, &local_copies(&paths)?);
                    // An app that's gone: look again next time.
                    if r.is_err() {
                        crate::openapps::forget();
                    }
                    r
                });
            }
            Pick::Choose | Pick::ChangeDefault => {
                let is_dir = is_folder(&first);
                let types = types_of(&paths, is_dir);
                let t = types[0].clone();
                let how = if pick == Pick::ChangeDefault { How::TypeDefault } else { How::Once };
                let chosen = (pick == Pick::ChangeDefault).then(|| self.settings.open_with.own_default(&t).map(|(h, _)| h)).flatten();
                self.files.picker = Some(Picker { paths, t, types, search: String::new(), chosen, how, confirm: false, focus: true, just_set: pick == Pick::ChangeDefault });
            }
            Pick::ResetDefault => {
                let is_dir = is_folder(&first);
                let t = type_of(&first, is_dir);
                match self.settings.open_with.own_default(&t) {
                    Some((_, Source::Type)) => {
                        let k = t.key().unwrap_or_default();
                        self.assoc(|o| {
                            o.defaults.remove(&k);
                        });
                    }
                    Some((_, Source::Kind)) => {
                        let k = t.category().map(kind_key).unwrap_or_default();
                        self.assoc(|o| {
                            o.kind_defaults.remove(&k);
                        });
                    }
                    _ => {}
                }
            }
            Pick::Browse => {
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
            Pick::Pin => {
                if let Some(h) = PICKED.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    self.assoc(|o| o.toggle_pin(&h.id));
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
    fn several_types_list_the_apps_that_open_them_all() {
        let h = |id: &str, default: bool| Handler { id: id.into(), name: id.into(), app: PathBuf::new(), default };
        let txt = vec![h("vim", true), h("kate", false), h("writer", false)];
        let md = vec![h("kate", false), h("vim", true), h("typora", false)];
        let both = common(&[txt.clone(), md]);
        let got: Vec<_> = both.iter().map(|h| (h.id.as_str(), h.default)).collect();
        assert_eq!(got, [("vim", true), ("kate", false)]);
        let png = vec![h("gimp", true), h("vim", false)];
        let got: Vec<_> = common(&[txt.clone(), png]).iter().map(|h| (h.id.clone(), h.default)).collect();
        assert_eq!(got, [("vim".to_string(), false)]);
        assert_eq!(common(&[txt.clone()]), txt);
        assert!(common(&[]).is_empty());
    }

    #[test]
    fn lists_default_pinned_recent_then_the_rest() {
        use crate::openapps::{Handler, Type};
        let h = |id: &str, default: bool| Handler { id: id.into(), name: id.into(), app: PathBuf::new(), default };
        let list = vec![h("vim", true), h("atom", false), h("kate", false), h("zed", false)];
        let t = Type::Ext("txt".into());
        let mut prefs = OpenPrefs::default();
        prefs.toggle_pin("zed");
        prefs.toggle_pin("vim");
        prefs.used(&t, "kate");
        prefs.used(&Type::Ext("md".into()), "atom");
        let got: Vec<_> = ordered(&list, &prefs, &t).into_iter().map(|(h, p)| (h.id.as_str(), p)).collect();
        assert_eq!(got, [("vim", Place::Default), ("zed", Place::Pinned), ("kate", Place::Recent), ("atom", Place::Other)]);
        prefs.toggle_pin("zed");
        assert!(!prefs.is_pinned("zed"));
        for i in 0..9 {
            prefs.used(&t, &format!("app{i}"));
        }
        prefs.used(&t, "app3");
        let r = &prefs.recent["txt"];
        assert_eq!((r.len(), r[0].as_str(), r[1].as_str()), (crate::openapps::prefs::RECENTS, "app3", "app8"));
        prefs.used(&Type::Unknown, "x");
        assert_eq!(prefs.recent.len(), 2);
    }

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
