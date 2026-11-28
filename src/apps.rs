//! Applications and other bundles: folders that macOS treats as one item (`Safari.app`,
//! `Sparkle.framework`, `Serum.vst3`). The file manager shows an application with its own icon
//! and name and launches it on a double-click, while Show Package Contents still goes inside.
//!
//! Nothing here runs an application's code to learn about it: names, versions and icons come
//! from `Info.plist` and the bundle's icon file (or, on macOS, from the system's icon service),
//! read as data. Launching goes through the system (LaunchServices on macOS), so Gatekeeper and
//! the user's permissions stay in charge.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(target_os = "macos")]
mod mac;
pub mod lnk;
pub mod pe;

/// What kind of bundle a folder is. Stored in [`crate::fsys::listing::Entry::bundle`] as
/// [`Bundle::code`] (0 for none).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bundle {
    App,
    Framework,
    Plugin,
    Loadable,
    AudioUnit,
    Vst,
    Vst3,
    Extension,
    PrefPane,
    Kext,
    Screensaver,
    QuickLook,
    Spotlight,
    Service,
    /// A Windows program (`.exe`): a file, not a bundle, but shown with its own icon, name
    /// and details the same way.
    WinApp,
    /// A Windows shortcut (`.lnk`).
    Shortcut,
}

impl Bundle {
    const ALL: [Bundle; 16] = [
        Bundle::App,
        Bundle::Framework,
        Bundle::Plugin,
        Bundle::Loadable,
        Bundle::AudioUnit,
        Bundle::Vst,
        Bundle::Vst3,
        Bundle::Extension,
        Bundle::PrefPane,
        Bundle::Kext,
        Bundle::Screensaver,
        Bundle::QuickLook,
        Bundle::Spotlight,
        Bundle::Service,
        Bundle::WinApp,
        Bundle::Shortcut,
    ];

    /// By the folder's extension alone (any case).
    pub fn by_name(name: &str) -> Option<Bundle> {
        let ext = crate::scan::ext_of(name).to_ascii_lowercase();
        Some(match ext.as_str() {
            "app" => Bundle::App,
            "framework" => Bundle::Framework,
            "plugin" => Bundle::Plugin,
            "bundle" => Bundle::Loadable,
            "component" => Bundle::AudioUnit,
            "vst" => Bundle::Vst,
            "vst3" => Bundle::Vst3,
            "appex" => Bundle::Extension,
            "prefpane" => Bundle::PrefPane,
            "kext" => Bundle::Kext,
            "saver" => Bundle::Screensaver,
            "qlgenerator" => Bundle::QuickLook,
            "mdimporter" => Bundle::Spotlight,
            "service" => Bundle::Service,
            _ => return None,
        })
    }

    pub fn code(self) -> u8 {
        self as u8 + 1
    }

    pub fn from_code(c: u8) -> Option<Bundle> {
        Self::ALL.get((c as usize).checked_sub(1)?).copied()
    }

    /// What the Kind column calls it.
    pub fn kind(self) -> &'static str {
        match self {
            Bundle::App => "Application",
            Bundle::Framework => "Framework",
            Bundle::Plugin => "Plug-in",
            Bundle::Loadable => "Bundle",
            Bundle::AudioUnit => "Audio Unit plug-in",
            Bundle::Vst => "VST plug-in",
            Bundle::Vst3 => "VST3 plug-in",
            Bundle::Extension => "App extension",
            Bundle::PrefPane => "System Settings pane",
            Bundle::Kext => "Kernel extension",
            Bundle::Screensaver => "Screen saver",
            Bundle::QuickLook => "Quick Look plug-in",
            Bundle::Spotlight => "Spotlight importer",
            Bundle::Service => "Service",
            Bundle::WinApp => "Application",
            Bundle::Shortcut => "Shortcut",
        }
    }

    /// The extension to hide from the name (applications only, as Finder does).
    fn hidden_ext(self) -> bool {
        self == Bundle::App
    }
}

/// Whether the folder at `path` (named `name`) is a real bundle, not just a folder whose name
/// happens to end in `.app`: it must have the layout the system looks for. Only a couple of
/// `stat`s, and only for names with a bundle extension, so listings stay as fast as before.
/// 0 when it isn't one; otherwise [`Bundle::code`].
pub fn bundle_at(path: &Path, name: &str) -> u8 {
    let Some(b) = Bundle::by_name(name) else { return 0 };
    let has = |rel: &str| std::fs::symlink_metadata(path.join(rel)).is_ok();
    let ok = match b {
        // Mac apps keep Info.plist in Contents; iPhone and iPad apps installed on an Apple
        // silicon Mac are a wrapper around one; very old apps may have only Contents/MacOS.
        Bundle::App => has("Contents/Info.plist") || has("Contents/MacOS") || has("WrappedBundle") || has("Info.plist"),
        Bundle::Framework => has("Resources") || has("Versions") || has("Info.plist"),
        // VST3 plug-ins on Windows and Linux are bundles too (Contents/x86_64-win…).
        _ => has("Contents"),
    };
    if ok { b.code() } else { 0 }
}

/// The same check as [`bundle_at`] for a folder in a finished scan, from the children it
/// recorded (no disk access). Only applications: Storage Analysis treats them as one item.
pub fn node_is_app(n: &crate::scan::Node) -> bool {
    // Called for every folder on a path when the maps are coloured: no allocation here.
    let name = n.name.as_bytes();
    if !n.is_dir || name.len() < 5 || !name[name.len() - 4..].eq_ignore_ascii_case(b".app") || !prefs().recognise {
        return false;
    }
    let child = |name: &str| n.children.iter().find(|c| &*c.name == name);
    child("Contents").is_some_and(|c| c.is_dir && c.children.iter().any(|g| &*g.name == "Info.plist" || &*g.name == "MacOS"))
        || child("WrappedBundle").is_some()
        || child("Info.plist").is_some_and(|c| !c.is_dir)
}

/// How deep the first application on a path of child indices is: `Some(k)` when
/// `root.at(&path[..=k])` is an app (so everything below it belongs to that app).
pub fn app_on_path(root: &crate::scan::Node, path: &[u32]) -> Option<usize> {
    if !prefs().recognise {
        return None;
    }
    let mut n = root;
    for (k, &i) in path.iter().enumerate() {
        n = n.children.get(i as usize)?;
        if n.is_dir && node_is_app(n) {
            return Some(k);
        }
    }
    None
}

/// The applications in a scan, outermost only (an app's helper apps belong to it): each one's
/// path of child indices. Stops after `max`.
pub fn apps_in(root: &crate::scan::Node, max: usize) -> Vec<Vec<u32>> {
    fn walk(n: &crate::scan::Node, path: &mut Vec<u32>, out: &mut Vec<Vec<u32>>, max: usize) {
        for (i, c) in n.children.iter().enumerate() {
            if out.len() >= max {
                return;
            }
            if !c.is_dir {
                continue;
            }
            path.push(i as u32);
            if node_is_app(c) {
                out.push(path.clone());
            } else {
                walk(c, path, out, max);
            }
            path.pop();
        }
    }
    let mut out = Vec::new();
    if prefs().recognise {
        walk(root, &mut Vec::new(), &mut out, max);
    }
    out
}

/// What takes the space inside an application, biggest first: (part, what it is, its node).
/// The parts are the folders in `Contents` that people ask about; the rest is "Other".
pub fn parts(app: &crate::scan::Node) -> Vec<(&'static str, &'static str, u64)> {
    let Some(contents) = app.children.iter().find(|c| &*c.name == "Contents" && c.is_dir) else { return Vec::new() };
    let mut out: Vec<(&'static str, &'static str, u64)> = Vec::new();
    let mut other = app.size.saturating_sub(contents.size);
    for c in contents.children.iter() {
        let part = match &*c.name {
            "Frameworks" => Some(("Frameworks", "Libraries the app brings with it")),
            "Resources" => Some(("Resources", "Pictures, sounds, translations and other data")),
            "MacOS" => Some(("Program", "The app's own program")),
            "PlugIns" | "Extensions" => Some(("Plug-ins", "Plug-ins and extensions")),
            "Helpers" | "Library" | "XPCServices" | "Login Items" | "LoginItems" => Some(("Helpers", "Helper programs and background services")),
            "SharedSupport" => Some(("Shared support", "Extra content, such as installers or samples")),
            _ => None,
        };
        match part {
            Some((name, what)) => match out.iter_mut().find(|p| p.0 == name) {
                Some(p) => p.2 += c.size,
                None => out.push((name, what, c.size)),
            },
            None => other += c.size,
        }
    }
    if other > 0 {
        out.push(("Other", "Everything else in the bundle", other));
    }
    out.retain(|p| p.2 > 0);
    out.sort_by(|a, b| b.2.cmp(&a.2));
    out
}

/// What a file is by its name: a Windows program or shortcut (code as in [`Bundle::code`]), 0
/// for anything else. Just the name, so listings stay as fast as before.
pub fn file_code(name: &str) -> u8 {
    let n = name.as_bytes();
    let ends = |ext: &[u8]| n.len() > ext.len() && n[n.len() - ext.len()..].eq_ignore_ascii_case(ext);
    if ends(b".exe") {
        Bundle::WinApp.code()
    } else if ends(b".lnk") {
        Bundle::Shortcut.code()
    } else {
        0
    }
}

/// Items drawn with their own icon and shown with application details: Mac applications,
/// Windows programs and shortcuts.
pub fn own_icon(code: u8) -> bool {
    matches!(of(code), Some(Bundle::App | Bundle::WinApp | Bundle::Shortcut))
}

/// What the user chose under Settings > Files > Applications & Executables. Kept here (not
/// passed around) because names and icons are drawn in many places.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Prefs {
    /// Show bundles as applications (off: every bundle is a plain folder).
    pub recognise: bool,
    pub icons: bool,
    pub hide_ext: bool,
    pub double_click: bool,
    /// Double-click and Enter go inside, as before.
    pub as_folders: bool,
}

static PREFS: AtomicU8 = AtomicU8::new(0b00111);

/// Held by tests that change the preferences, or need the defaults, so they don't race.
#[cfg(test)]
pub static PREFS_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn set_prefs(p: Prefs) {
    let bits = p.recognise as u8 | (p.icons as u8) << 1 | (p.hide_ext as u8) << 2 | (p.double_click as u8) << 3 | (p.as_folders as u8) << 4;
    PREFS.store(bits, Ordering::Relaxed);
}

pub fn prefs() -> Prefs {
    let b = PREFS.load(Ordering::Relaxed);
    Prefs { recognise: b & 1 != 0, icons: b & 2 != 0, hide_ext: b & 4 != 0, double_click: b & 8 != 0, as_folders: b & 16 != 0 }
}

/// The bundle an entry is, if bundles are recognised at all.
pub fn of(code: u8) -> Option<Bundle> {
    if code == 0 || !prefs().recognise {
        return None;
    }
    Bundle::from_code(code)
}

/// An application (by the listing's `bundle` code) that the user sees as one.
pub fn is_app(code: u8) -> bool {
    of(code) == Some(Bundle::App)
}

/// Applications run only on the system they were made for.
pub const CAN_LAUNCH: bool = cfg!(target_os = "macos");

/// Opening this entry (double-click, Enter, Open) launches it rather than going inside.
pub fn launches(code: u8) -> bool {
    let p = prefs();
    CAN_LAUNCH && is_app(code) && !p.as_folders && p.double_click
}

/// The name to show: "Safari" for `Safari.app` when extensions are hidden.
pub fn shown_name(name: &str, code: u8) -> &str {
    match of(code) {
        Some(b) if b.hidden_ext() && prefs().hide_ext => {
            let stem = &name[..name.len() - crate::scan::ext_of(name).len()];
            stem.strip_suffix('.').filter(|s| !s.is_empty()).unwrap_or(name)
        }
        _ => name,
    }
}

/// An application's facts from its `Info.plist`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    pub name: Option<String>,
    pub id: Option<String>,
    pub version: Option<String>,
    pub build: Option<String>,
    pub min_os: Option<String>,
    pub executable: Option<String>,
    pub icon_file: Option<String>,
    pub copyright: Option<String>,
    /// The publisher a Windows program names (CompanyName).
    pub company: Option<String>,
    /// What a Windows program says it is (FileDescription).
    pub description: Option<String>,
}

/// Where a bundle keeps `Info.plist`.
fn plist_path(app: &Path) -> Option<PathBuf> {
    ["Contents/Info.plist", "Resources/Info.plist", "Info.plist"].iter().map(|r| app.join(r)).find(|p| p.is_file())
}

/// Read a bundle's `Info.plist` (XML or binary). `None` when there's none or it's damaged.
pub fn info(app: &Path) -> Option<Info> {
    let v = plist::Value::from_file(plist_path(app)?).ok()?;
    let d = v.as_dictionary()?;
    let s = |k: &str| d.get(k).and_then(|v| v.as_string()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    Some(Info {
        name: s("CFBundleDisplayName").or_else(|| s("CFBundleName")),
        id: s("CFBundleIdentifier"),
        version: s("CFBundleShortVersionString"),
        build: s("CFBundleVersion"),
        min_os: s("LSMinimumSystemVersion"),
        executable: s("CFBundleExecutable"),
        icon_file: s("CFBundleIconFile"),
        copyright: s("NSHumanReadableCopyright"),
        ..Default::default()
    })
}

/// Why an application can't be launched, found by looking at it (before asking the system,
/// which gives vaguer reasons). `None` when it looks fine.
pub fn problem(app: &Path) -> Option<String> {
    let name = app.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let shown = shown_name(&name, Bundle::App.code()).to_string();
    if std::fs::read_dir(app).is_err() {
        return Some(format!("Filescape can't open “{shown}”: you don't have permission to read it."));
    }
    // An iPhone or iPad app is a wrapper; the system knows how to open it.
    if app.join("WrappedBundle").exists() || app.join("Wrapper").is_dir() {
        return None;
    }
    let info = info(app);
    if let Some(exe) = info.as_ref().and_then(|i| i.executable.clone()) {
        if !app.join("Contents/MacOS").join(&exe).exists() && !app.join(&exe).exists() {
            return Some(format!("“{shown}” is damaged or incomplete: its program ({exe}) is missing. Try installing it again."));
        }
    } else if info.is_none() && !app.join("Contents/MacOS").is_dir() {
        return Some(format!("“{shown}” is damaged or incomplete: it has no Info.plist or program. Try installing it again."));
    }
    None
}

/// Launch an application through the system. Slow (it waits for the system to answer), so
/// call it off the UI thread.
pub fn launch(app: &Path) -> Result<(), String> {
    if crate::fsys::remote::is_remote(app) || crate::fsys::archive::inside(app) {
        return Err("Applications can only be launched from a drive on this computer.".into());
    }
    if !CAN_LAUNCH {
        return Err("This is a macOS application. It can only be launched on a Mac.".into());
    }
    if let Some(why) = problem(app) {
        return Err(why);
    }
    #[cfg(target_os = "macos")]
    {
        mac::launch(app)
    }
    #[cfg(not(target_os = "macos"))]
    {
        unreachable!()
    }
}

/// An application's icon as RGBA pixels (`size` across, or the nearest the icon has). On macOS
/// the system's own icon (as Finder shows it, including apps that only have an asset catalogue);
/// elsewhere, or if that fails, the bundle's `.icns` file. Never for apps in a cloud folder (that
/// could download them). Slow: call it off the UI thread.
pub fn icon(app: &Path, size: u32) -> Option<(u32, u32, Vec<u8>)> {
    if crate::cloud::root_of(&crate::cloud::roots(), app).is_some() {
        return None;
    }
    let name = app.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let code = file_code(&name);
    if code != 0 {
        let img = if code == Bundle::WinApp.code() { pe::icon_file(app, size) } else { shortcut_icon(app, size) };
        return img.map(|i| (i.width(), i.height(), i.into_raw()));
    }
    #[cfg(target_os = "macos")]
    if let Some(i) = mac::icon(app, size) {
        return Some(i);
    }
    let file = icns_path(app)?;
    let data = std::fs::read(file).ok()?;
    let img = icns_image(&data, size)?;
    Some((img.width(), img.height(), img.into_raw()))
}

/// The icon a shortcut shows: from its icon location (a program or an .ico file), else its
/// target's. Only files on this computer (a Windows path means nothing elsewhere).
fn shortcut_icon(link: &Path, size: u32) -> Option<image::RgbaImage> {
    let sc = lnk::shortcut(&std::fs::read(link).ok()?)?;
    let from = |p: &str| -> Option<image::RgbaImage> {
        let p = PathBuf::from(lnk::expand(p));
        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ico")) {
            let data = std::fs::read(&p).ok()?;
            return image::load_from_memory_with_format(&data, image::ImageFormat::Ico).ok().map(|i| i.into_rgba8());
        }
        pe::icon_file(&p, size)
    };
    sc.icon.as_ref().and_then(|(p, _)| from(p)).or_else(|| from(sc.target.as_deref()?))
}

/// The bundle's icon file, from `CFBundleIconFile` (".icns" may be left off) or the usual names.
fn icns_path(app: &Path) -> Option<PathBuf> {
    let res = app.join("Contents/Resources");
    let named = info(app).and_then(|i| i.icon_file).map(|f| if f.to_ascii_lowercase().ends_with(".icns") { f } else { format!("{f}.icns") });
    named.into_iter().chain(["AppIcon.icns".to_string(), "Icon.icns".to_string()]).map(|f| res.join(f)).find(|p| p.is_file())
}

/// The picture in an `.icns` file closest to `size` (preferring larger), from the PNG
/// entries modern icons are made of.
pub fn icns_image(data: &[u8], size: u32) -> Option<image::RgbaImage> {
    if data.get(..4)? != b"icns" {
        return None;
    }
    let mut best: Option<(u32, &[u8])> = None;
    let mut at = 8;
    while at + 8 <= data.len() {
        let kind = &data[at..at + 4];
        let len = u32::from_be_bytes(data[at + 4..at + 8].try_into().ok()?) as usize;
        if len < 8 || at + len > data.len() {
            break;
        }
        let body = &data[at + 8..at + len];
        at += len;
        // Pixel sizes of the entries that hold a PNG (or JPEG 2000, which is skipped).
        let px = match kind {
            b"icp4" => 16,
            b"icp5" | b"ic11" => 32,
            b"icp6" | b"ic12" => 64,
            b"ic07" => 128,
            b"ic08" | b"ic13" => 256,
            b"ic09" | b"ic14" => 512,
            b"ic10" => 1024,
            _ => continue,
        };
        if !body.starts_with(b"\x89PNG") {
            continue;
        }
        // Closest to the size asked for, a larger one winning a tie (it scales down well).
        let score = |p: u32| if p >= size { p - size } else { (size - p) * 4 };
        if best.is_none_or(|(b, _)| score(px) < score(b)) {
            best = Some((px, body));
        }
    }
    let img = image::load_from_memory_with_format(best?.1, image::ImageFormat::Png).ok()?.into_rgba8();
    Some(if img.width() > size * 2 { image::imageops::resize(&img, size, size, image::imageops::FilterType::Triangle) } else { img })
}

/// Everything the preview pane shows about an application. Slow to work out (it reads the
/// program's header, adds up the bundle and, on macOS, asks the system's `codesign` and `spctl`
/// tools about it), so it's made on a worker thread: see [`DetailsCache`].
#[derive(Clone, Debug, Default)]
pub struct Details {
    pub info: Option<Info>,
    /// "Apple silicon", "Intel", "Universal (Apple silicon and Intel)".
    pub arch: Option<String>,
    /// Bytes, space on disk, and files in the bundle.
    pub size: (u64, u64, u64),
    /// Who signed it, as macOS says ("Developer ID: Acme Ltd (TEAMID)", "Mac App Store",
    /// "Apple", "Signed for this Mac only", "Not signed").
    pub signed: Option<String>,
    /// Whether Gatekeeper accepts it, and why ("Notarised by Apple", "Rejected").
    pub gatekeeper: Option<String>,
    /// Downloaded from the internet and not opened yet (the quarantine flag).
    pub quarantined: bool,
    /// What's wrong with it, if it can't be launched.
    pub problem: Option<String>,
    /// A Windows program that runs in a console window (a command-line tool).
    pub console: bool,
    /// For a shortcut: what it says (the details above are its target's).
    pub shortcut: Option<lnk::Shortcut>,
}

impl Details {
    /// The publisher: from the signature when there is one, else the copyright line.
    pub fn publisher(&self) -> Option<String> {
        let from_sig = self.signed.as_deref().and_then(|s| {
            let s = s.strip_prefix("Developer ID: ")?;
            Some(s.rsplit_once(" (").map_or(s, |(n, _)| n).to_string())
        });
        from_sig.or_else(|| match self.signed.as_deref() {
            Some("Apple") => Some("Apple".into()),
            _ => None,
        })
        .or_else(|| self.info.as_ref()?.company.clone())
        .or_else(|| self.info.as_ref()?.copyright.clone())
    }
}

/// Work out an application's details (slow: call it off the UI thread). Nothing in the app is
/// run: its program's header is read as data, and macOS's own tools check the signature.
pub fn details(app: &Path) -> Details {
    let name = app.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match Bundle::from_code(file_code(&name)) {
        Some(Bundle::WinApp) => return program_details(app),
        Some(Bundle::Shortcut) => return shortcut_details(app),
        _ => {}
    }
    let info = info(app);
    let exe = info.as_ref().and_then(|i| i.executable.clone()).map(|e| app.join("Contents/MacOS").join(e));
    let arch = exe.as_deref().and_then(arch_of);
    let size = bundle_size(app);
    let (signed, gatekeeper) = signature(app);
    Details { info, arch, size, signed, gatekeeper, quarantined: quarantined(app), problem: problem(app), ..Default::default() }
}

/// Space a file takes on the drive (its length where the system doesn't say).
fn alloc_of(md: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        md.blocks() * 512
    }
    #[cfg(not(unix))]
    {
        md.len()
    }
}

/// A Windows program's details, from its header and version information.
fn program_details(exe: &Path) -> Details {
    let meta = std::fs::metadata(exe).ok();
    let size = meta.as_ref().map_or((0, 0, 1), |m| (m.len(), alloc_of(m), 1));
    let Some(p) = pe::program_file(exe) else {
        let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        return Details { size, problem: Some(format!("“{name}” isn't a Windows program, or it's damaged.")), ..Default::default() };
    };
    let info = Info {
        name: p.product.clone().or_else(|| p.description.clone()),
        version: p.product_version.clone(),
        build: p.file_version.clone().filter(|f| Some(f) != p.product_version.as_ref()),
        copyright: p.copyright.clone(),
        company: p.company.clone(),
        description: p.description.clone().filter(|d| Some(d) != p.product.as_ref()),
        executable: p.original_name.clone(),
        ..Default::default()
    };
    Details {
        info: Some(info),
        arch: p.arch.clone().map(|a| format!("Windows, {a}")),
        size,
        signed: Some(if p.signed { "Signed (Windows checks the signature when it runs)".into() } else { "Not signed".into() }),
        quarantined: marked_downloaded(exe),
        console: p.console,
        ..Default::default()
    }
}

/// A shortcut's details: what it says, and its target's details when the target is a program
/// on this computer.
fn shortcut_details(link: &Path) -> Details {
    let size = std::fs::metadata(link).map_or((0, 0, 1), |m| (m.len(), alloc_of(&m), 1));
    let Some(sc) = std::fs::read(link).ok().and_then(|d| lnk::shortcut(&d)) else {
        return Details { size, problem: Some("This shortcut is damaged: it doesn't say where it points.".into()), ..Default::default() };
    };
    let target = sc.target.as_deref().map(lnk::expand).map(PathBuf::from);
    let mut d = match target.as_deref() {
        Some(t) if cfg!(windows) && file_code(&t.to_string_lossy()) == Bundle::WinApp.code() && t.is_file() => program_details(t),
        _ => Details::default(),
    };
    if cfg!(windows) && target.as_deref().is_some_and(|t| !t.exists()) {
        d.problem = Some("What this shortcut points to isn't there any more: it was moved, renamed or uninstalled.".into());
    }
    d.size = size;
    d.shortcut = Some(sc);
    d
}

/// Windows marks files downloaded from the internet (the "mark of the web") so SmartScreen
/// checks them when they're opened.
fn marked_downloaded(path: &Path) -> bool {
    #[cfg(windows)]
    {
        let mut ads = path.as_os_str().to_owned();
        ads.push(":Zone.Identifier");
        std::fs::read_to_string(ads).is_ok_and(|z| z.lines().any(|l| l.trim().strip_prefix("ZoneId=").is_some_and(|z| z.trim() == "3" || z.trim() == "4")))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Which processors a Mach-O program runs on, from its header.
pub fn arch_of(exe: &Path) -> Option<String> {
    use std::io::Read;
    let mut head = vec![0u8; 4096];
    let n = std::fs::File::open(exe).ok()?.read(&mut head).ok()?;
    arch_label(&cpus(&head[..n])?)
}

const CPU_X86_64: u32 = 0x0100_0007;
const CPU_ARM64: u32 = 0x0100_000c;

/// The CPU types in a Mach-O header (thin or universal).
fn cpus(h: &[u8]) -> Option<Vec<u32>> {
    let be = |at: usize| h.get(at..at + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()));
    let le = |at: usize| h.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    match be(0)? {
        // A universal binary: a table of architectures (32- or 64-bit entries). Java class
        // files share the magic number but have a version where the count would be.
        m @ (0xcafe_babe | 0xcafe_babf) => {
            let n = be(4)? as usize;
            if n == 0 || n > 16 {
                return None;
            }
            let step = if m == 0xcafe_babf { 32 } else { 20 };
            (0..n).map(|i| be(8 + i * step)).collect()
        }
        // Thin, little-endian 64- or 32-bit.
        0xcffa_edfe | 0xcefa_edfe => Some(vec![le(4)?]),
        _ => None,
    }
}

fn arch_label(cpus: &[u32]) -> Option<String> {
    let arm = cpus.contains(&CPU_ARM64);
    let intel = cpus.contains(&CPU_X86_64);
    Some(match (arm, intel) {
        (true, true) => "Universal (Apple silicon and Intel)",
        (true, false) => "Apple silicon",
        (false, true) => "Intel",
        _ => "Other (older or unusual processor)",
    }.into())
}

/// Bytes, space on disk and files in a bundle, not following links.
fn bundle_size(dir: &Path) -> (u64, u64, u64) {
    let (mut bytes, mut alloc, mut files) = (0, 0, 0);
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                todo.push(e.path());
            } else {
                bytes += md.len();
                files += 1;
                alloc += alloc_of(&md);
            }
        }
    }
    (bytes, alloc, files)
}

/// The signature and Gatekeeper's verdict, from macOS's `codesign` and `spctl`.
#[cfg(target_os = "macos")]
fn signature(app: &Path) -> (Option<String>, Option<String>) {
    use std::process::Command;
    let err = |c: &mut Command| c.output().ok().map(|o| String::from_utf8_lossy(&o.stderr).into_owned());
    let signed = err(Command::new("/usr/bin/codesign").args(["-dv", "--verbose=2"]).arg(app)).map(|t| signer(&t));
    let gk = err(Command::new("/usr/sbin/spctl").args(["-a", "-vv", "-t", "exec"]).arg(app)).map(|t| gatekeeper(&t));
    (signed, gk)
}

#[cfg(not(target_os = "macos"))]
fn signature(_app: &Path) -> (Option<String>, Option<String>) {
    (None, None)
}

/// Who signed it, from `codesign -dv` output.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn signer(t: &str) -> String {
    if t.contains("not signed at all") {
        return "Not signed".into();
    }
    if t.contains("Signature=adhoc") {
        return "Signed for this Mac only (ad hoc)".into();
    }
    let first = t.lines().find_map(|l| l.strip_prefix("Authority=")).unwrap_or("");
    if first.contains("Apple Mac OS Application Signing") {
        "Mac App Store".into()
    } else if first == "Software Signing" {
        "Apple".into()
    } else if let Some(d) = first.strip_prefix("Developer ID Application: ") {
        format!("Developer ID: {d}")
    } else if first.is_empty() {
        "Signed (no details)".into()
    } else {
        first.to_string()
    }
}

/// Gatekeeper's verdict, from `spctl -a -vv` output.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn gatekeeper(t: &str) -> String {
    let source = t.lines().find_map(|l| l.trim().strip_prefix("source=")).unwrap_or("");
    if t.contains(": accepted") || t.lines().any(|l| l.trim() == "accepted") {
        match source {
            "Notarized Developer ID" => "Allowed: notarised by Apple".into(),
            "Apple System" => "Allowed: part of macOS".into(),
            "Mac App Store" => "Allowed: from the Mac App Store".into(),
            "" => "Allowed".into(),
            s => format!("Allowed ({s})"),
        }
    } else if t.contains("rejected") {
        if source.is_empty() { "Blocked by Gatekeeper".into() } else { format!("Blocked by Gatekeeper ({source})") }
    } else {
        "Unknown".into()
    }
}

/// Whether macOS has marked it as downloaded from the internet and not yet opened.
fn quarantined(app: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let (Ok(p), Ok(n)) = (std::ffi::CString::new(app.as_os_str().as_bytes()), std::ffi::CString::new("com.apple.quarantine")) else { return false };
        // SAFETY: NUL-terminated strings; a null buffer asks only for the size.
        unsafe { libc::getxattr(p.as_ptr(), n.as_ptr(), std::ptr::null_mut(), 0, 0, 0) >= 0 }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        false
    }
}

/// Applications' details, made one at a time on a worker thread and kept by path and modified
/// time (an updated app is read again).
#[derive(Default)]
pub struct DetailsCache {
    map: std::collections::HashMap<PathBuf, (i64, Option<std::sync::Arc<Details>>)>,
    worker: Option<(std::sync::mpsc::Sender<(PathBuf, i64)>, std::sync::mpsc::Receiver<(PathBuf, i64, Details)>)>,
}

impl DetailsCache {
    /// The details if ready; otherwise asks for them (and the context repaints when they come).
    pub fn get(&mut self, ctx: &eframe::egui::Context, app: &Path, stamp: i64) -> Option<std::sync::Arc<Details>> {
        if let Some((_, rx)) = &self.worker {
            while let Ok((p, st, d)) = rx.try_recv() {
                if let Some(e) = self.map.get_mut(&p)
                    && e.0 == st
                {
                    e.1 = Some(std::sync::Arc::new(d));
                }
            }
        }
        match self.map.get(app) {
            Some((st, d)) if *st == stamp => return d.clone(),
            _ => {}
        }
        if self.map.len() > 200 {
            self.map.clear();
        }
        self.map.insert(app.to_path_buf(), (stamp, None));
        let (tx, _) = self.worker.get_or_insert_with(|| {
            let (ask, jobs) = std::sync::mpsc::channel::<(PathBuf, i64)>();
            let (done, rx) = std::sync::mpsc::channel();
            let ctx = ctx.clone();
            let _ = std::thread::Builder::new().name("app-details".into()).spawn(move || {
                while let Ok((p, st)) = jobs.recv() {
                    let d = details(&p);
                    if done.send((p, st, d)).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            });
            (ask, rx)
        });
        let _ = tx.send((app.to_path_buf(), stamp));
        None
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("filescape-apps-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Sketchpad</string>
<key>CFBundleIdentifier</key><string>com.example.sketchpad</string>
<key>CFBundleShortVersionString</key><string>2.4.1</string>
<key>CFBundleVersion</key><string>2041</string>
<key>CFBundleExecutable</key><string>Sketchpad</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
</dict></plist>"#;

    fn make_app(dir: &Path, name: &str, with_exe: bool) -> PathBuf {
        let app = dir.join(name);
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("Contents/Info.plist"), PLIST).unwrap();
        if with_exe {
            std::fs::write(app.join("Contents/MacOS/Sketchpad"), b"\xcf\xfa\xed\xfe").unwrap();
        }
        app
    }

    #[test]
    fn recognises_real_bundles_only() {
        let d = tmp("recognise");
        let app = make_app(&d, "Sketchpad.app", true);
        assert_eq!(bundle_at(&app, "Sketchpad.app"), Bundle::App.code());
        // A plain folder that happens to be called .app.
        std::fs::create_dir_all(d.join("Notes.app/stuff")).unwrap();
        assert_eq!(bundle_at(&d.join("Notes.app"), "Notes.app"), 0);
        std::fs::create_dir_all(d.join("Sparkle.framework/Versions/A")).unwrap();
        assert_eq!(Bundle::from_code(bundle_at(&d.join("Sparkle.framework"), "Sparkle.framework")), Some(Bundle::Framework));
        std::fs::create_dir_all(d.join("Serum.VST3/Contents")).unwrap();
        assert_eq!(Bundle::from_code(bundle_at(&d.join("Serum.VST3"), "Serum.VST3")), Some(Bundle::Vst3));
        assert_eq!(bundle_at(&d.join("Docs"), "Docs"), 0);
        for b in Bundle::ALL {
            assert_eq!(Bundle::from_code(b.code()), Some(b));
        }
        assert_eq!(Bundle::from_code(0), None);
    }

    #[test]
    fn names_and_prefs() {
        let _g = PREFS_TEST.lock().unwrap_or_else(|e| e.into_inner());
        let app = Bundle::App.code();
        set_prefs(Prefs { recognise: true, icons: true, hide_ext: true, double_click: true, as_folders: false });
        assert_eq!(shown_name("Safari.app", app), "Safari");
        assert_eq!(shown_name("Visual Studio Code.APP", app), "Visual Studio Code");
        assert_eq!(shown_name(".app", app), ".app");
        assert_eq!(shown_name("Safari.app", 0), "Safari.app");
        assert_eq!(shown_name("Sparkle.framework", Bundle::Framework.code()), "Sparkle.framework");
        assert_eq!(launches(app), CAN_LAUNCH);
        set_prefs(Prefs { recognise: true, icons: true, hide_ext: false, double_click: true, as_folders: true });
        assert_eq!(shown_name("Safari.app", app), "Safari.app");
        assert!(!launches(app));
        set_prefs(Prefs { recognise: false, icons: true, hide_ext: true, double_click: true, as_folders: false });
        assert_eq!(shown_name("Safari.app", app), "Safari.app");
        assert!(!is_app(app));
        set_prefs(Prefs { recognise: true, icons: true, hide_ext: true, double_click: true, as_folders: false });
    }

    /// A scanned folder tree: `name/` makes a folder of the following items up to `..`.
    pub(crate) fn scanned(items: &[(&str, u64)]) -> crate::scan::Node {
        use crate::scan::Node;
        fn node(name: &str, size: u64, children: Vec<Node>) -> Node {
            let is_dir = name.ends_with('/');
            let mut n = Node { name: name.trim_end_matches('/').into(), size, logical: size, alloc: size, flags: 0, files: 1, modified: 0, is_dir, children };
            if is_dir {
                n.resum();
            }
            n
        }
        let mut stack: Vec<(String, Vec<Node>)> = vec![("root/".into(), Vec::new())];
        for &(name, size) in items {
            if name == ".." {
                let (n, kids) = stack.pop().unwrap();
                stack.last_mut().unwrap().1.push(node(&n, 0, kids));
            } else if name.ends_with('/') {
                stack.push((name.into(), Vec::new()));
            } else {
                stack.last_mut().unwrap().1.push(node(name, size, Vec::new()));
            }
        }
        while stack.len() > 1 {
            let (n, kids) = stack.pop().unwrap();
            stack.last_mut().unwrap().1.push(node(&n, 0, kids));
        }
        let (n, kids) = stack.pop().unwrap();
        node(&n, 0, kids)
    }

    /// Two real apps (one with a helper app inside), a fake one and a photo.
    pub(crate) fn sample_scan() -> crate::scan::Node {
        const MB: u64 = 1024 * 1024;
        scanned(&[
            ("Applications/", 0),
            ("Slack.app/", 0),
            ("Contents/", 0),
            ("Info.plist", 2_000),
            ("MacOS/", 0),
            ("Slack", 40 * MB),
            ("..", 0),
            ("Frameworks/", 0),
            ("Electron Framework.framework/", 0),
            ("Engine", 300 * MB),
            ("..", 0),
            ("Slack Helper.app/", 0),
            ("Contents/", 0),
            ("Info.plist", 1_000),
            ("..", 0),
            ("..", 0),
            ("..", 0),
            ("Resources/", 0),
            ("icon.png", 5 * MB),
            ("..", 0),
            ("..", 0),
            ("..", 0),
            ("Notes.app/", 0),
            ("readme.txt", 10),
            ("..", 0),
            ("Calculator.app/", 0),
            ("Contents/", 0),
            ("MacOS/", 0),
            ("Calculator", 3 * MB),
            ("..", 0),
            ("..", 0),
            ("..", 0),
            ("..", 0),
            ("photo.png", 2 * MB),
        ])
    }

    #[test]
    fn apps_in_a_scan() {
        let _g = PREFS_TEST.lock().unwrap_or_else(|e| e.into_inner());
        let t = sample_scan();
        let apps = apps_in(&t, usize::MAX);
        let names: Vec<&str> = apps.iter().map(|p| &*t.at(p).unwrap().name).collect();
        // Outermost only (the helper app belongs to Slack), and not the fake Notes.app.
        assert_eq!(apps.len(), 2, "{names:?}");
        assert!(names.contains(&"Slack.app") && names.contains(&"Calculator.app"), "{names:?}");
        let slack = apps.iter().find(|p| &*t.at(p).unwrap().name == "Slack.app").unwrap();
        // Everything below an app is on that app's path.
        let mut deep = slack.clone();
        deep.extend([0, 0]);
        assert_eq!(app_on_path(&t, &deep), Some(slack.len() - 1));
        let photo = vec![t.children.iter().position(|c| &*c.name == "photo.png").unwrap() as u32];
        assert_eq!(app_on_path(&t, &photo), None);
        let parts = parts(t.at(slack).unwrap());
        assert_eq!(parts[0].0, "Frameworks", "{parts:?}");
        assert_eq!(parts.iter().map(|p| p.0).collect::<Vec<_>>(), ["Frameworks", "Program", "Resources", "Other"]);
        assert_eq!(parts.iter().map(|p| p.2).sum::<u64>(), t.at(slack).unwrap().size, "the parts add up to the app");
    }

    #[test]
    fn windows_programs_and_shortcuts() {
        let _g = PREFS_TEST.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(file_code("Paint.EXE"), Bundle::WinApp.code());
        assert_eq!(file_code("Paint.lnk"), Bundle::Shortcut.code());
        assert_eq!(file_code(".exe"), 0);
        assert_eq!(file_code("notes.txt"), 0);
        assert!(own_icon(Bundle::WinApp.code()) && own_icon(Bundle::Shortcut.code()) && own_icon(Bundle::App.code()));
        assert!(!own_icon(Bundle::Framework.code()) && !is_app(Bundle::WinApp.code()), "only Mac apps open as bundles");
        assert_eq!(Bundle::from_code(Bundle::WinApp.code()).unwrap().kind(), "Application");
        let d = tmp("windows");
        let exe = d.join("paint.exe");
        std::fs::write(&exe, pe::tests::sample_exe()).unwrap();
        let det = details(&exe);
        let i = det.info.clone().unwrap();
        assert_eq!(i.name.as_deref(), Some("Acme Paint Studio"));
        assert_eq!(det.publisher().as_deref(), Some("Acme Ltd"));
        assert_eq!((i.version.as_deref(), i.build.as_deref()), (Some("3.2.1"), Some("3.2.1.77")));
        assert_eq!(det.arch.as_deref(), Some("Windows, 64-bit (x64)"));
        assert_eq!(det.size.2, 1);
        assert!(det.problem.is_none());
        let (w, h, px) = icon(&exe, 32).unwrap();
        assert_eq!((w, h, px.len()), (32, 32, 32 * 32 * 4));
        // Not a program at all.
        let fake = d.join("fake.exe");
        std::fs::write(&fake, b"just text").unwrap();
        assert!(details(&fake).problem.unwrap().contains("isn't a Windows program"));
        // A shortcut: what it says, whatever the system.
        let link = d.join("Paint.lnk");
        std::fs::write(&link, lnk::tests::sample_lnk("C:\\Program Files\\Acme\\paint.exe")).unwrap();
        let sd = details(&link);
        let sc = sd.shortcut.clone().unwrap();
        assert_eq!(sc.arguments.as_deref(), Some("--new-window"));
        assert!(sc.target.unwrap().ends_with("paint.exe"));
        std::fs::write(&link, b"broken").unwrap();
        assert!(details(&link).problem.unwrap().contains("damaged"));
    }

    #[test]
    fn reads_info_and_finds_problems() {
        let d = tmp("info");
        let app = make_app(&d, "Sketchpad.app", true);
        let i = info(&app).unwrap();
        assert_eq!(i.name.as_deref(), Some("Sketchpad"));
        assert_eq!(i.id.as_deref(), Some("com.example.sketchpad"));
        assert_eq!((i.version.as_deref(), i.build.as_deref(), i.min_os.as_deref()), (Some("2.4.1"), Some("2041"), Some("12.0")));
        assert!(problem(&app).is_none());
        let broken = make_app(&d, "Broken.app", false);
        assert!(problem(&broken).unwrap().contains("program (Sketchpad) is missing"));
        std::fs::create_dir_all(d.join("Empty.app/Contents")).unwrap();
        assert!(problem(&d.join("Empty.app")).unwrap().contains("damaged"));
        // A damaged Info.plist is no Info.
        std::fs::write(broken.join("Contents/Info.plist"), "not a plist").unwrap();
        assert!(info(&broken).is_none());
    }

    #[test]
    fn icns_picks_the_nearest_png() {
        let png = |px: u32| {
            let mut out = std::io::Cursor::new(Vec::new());
            image::RgbaImage::from_pixel(px, px, image::Rgba([px as u8, 0, 0, 255])).write_to(&mut out, image::ImageFormat::Png).unwrap();
            out.into_inner()
        };
        let mut body = Vec::new();
        for (kind, px) in [(b"ic07", 128), (b"ic08", 256), (b"ic09", 512)] {
            let p = png(px);
            body.extend_from_slice(kind);
            body.extend_from_slice(&((p.len() + 8) as u32).to_be_bytes());
            body.extend_from_slice(&p);
        }
        let mut file = b"icns".to_vec();
        file.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
        file.extend_from_slice(&body);
        assert_eq!(icns_image(&file, 128).unwrap().width(), 128);
        assert_eq!(icns_image(&file, 200).unwrap().width(), 256);
        assert_eq!(icns_image(&file, 64).unwrap().width(), 128);
        assert!(icns_image(b"nope", 64).is_none());
        // And through an app's Info.plist.
        let d = tmp("icns");
        let app = make_app(&d, "Sketchpad.app", true);
        std::fs::create_dir_all(app.join("Contents/Resources")).unwrap();
        std::fs::write(app.join("Contents/Resources/AppIcon.icns"), &file).unwrap();
        #[cfg(not(target_os = "macos"))]
        assert_eq!(icon(&app, 128).map(|i| i.0), Some(128));
    }

    #[test]
    fn architectures_and_signatures() {
        let thin = |cpu: u32| {
            let mut h = vec![0xcf, 0xfa, 0xed, 0xfe];
            h.extend_from_slice(&cpu.to_le_bytes());
            h
        };
        assert_eq!(arch_label(&cpus(&thin(CPU_ARM64)).unwrap()).unwrap(), "Apple silicon");
        assert_eq!(arch_label(&cpus(&thin(CPU_X86_64)).unwrap()).unwrap(), "Intel");
        let mut fat = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 2];
        for cpu in [CPU_X86_64, CPU_ARM64] {
            fat.extend_from_slice(&cpu.to_be_bytes());
            fat.extend_from_slice(&[0; 16]);
        }
        assert_eq!(arch_label(&cpus(&fat).unwrap()).unwrap(), "Universal (Apple silicon and Intel)");
        // A Java class file (same magic, a version where the count would be).
        assert!(cpus(&[0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 52]).is_none());
        assert!(cpus(b"#!/bin/sh").is_none());

        assert_eq!(signer("Executable=/x\nAuthority=Developer ID Application: Acme Ltd (AB12CD34)\nAuthority=Developer ID Certification Authority\n"), "Developer ID: Acme Ltd (AB12CD34)");
        assert_eq!(signer("Authority=Apple Mac OS Application Signing\n"), "Mac App Store");
        assert_eq!(signer("Authority=Software Signing\n"), "Apple");
        assert_eq!(signer("Signature=adhoc\n"), "Signed for this Mac only (ad hoc)");
        assert_eq!(signer("/x: code object is not signed at all\n"), "Not signed");
        assert_eq!(gatekeeper("/A.app: accepted\nsource=Notarized Developer ID\norigin=Developer ID Application: Acme\n"), "Allowed: notarised by Apple");
        assert_eq!(gatekeeper("/A.app: rejected\n"), "Blocked by Gatekeeper");
        let d = Details { signed: Some("Developer ID: Acme Ltd (AB12CD34)".into()), ..Default::default() };
        assert_eq!(d.publisher().as_deref(), Some("Acme Ltd"));
    }

    #[test]
    fn details_of_a_bundle() {
        let d = tmp("details");
        let app = make_app(&d, "Sketchpad.app", true);
        let x = details(&app);
        assert_eq!(x.info.unwrap().version.as_deref(), Some("2.4.1"));
        assert_eq!(x.size.2, 2);
        assert!(x.problem.is_none());
        assert_eq!(x.arch, None, "4 bytes is too short a header to tell");
    }
}
