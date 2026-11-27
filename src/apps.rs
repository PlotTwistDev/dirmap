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
}

impl Bundle {
    const ALL: [Bundle; 14] = [
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
    #[cfg(target_os = "macos")]
    if let Some(i) = mac::icon(app, size) {
        return Some(i);
    }
    let file = icns_path(app)?;
    let data = std::fs::read(file).ok()?;
    let img = icns_image(&data, size)?;
    Some((img.width(), img.height(), img.into_raw()))
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

#[cfg(test)]
mod tests {
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
}
