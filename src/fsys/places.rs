//! Standard places for the sidebar: Home, Desktop, Documents, Downloads and so on, found the
//! way each OS defines them (Known Folders on Windows, XDG user dirs on Linux, the home
//! folder's standard names on macOS). Only cheap lookups: nothing here touches the network.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Home,
    Desktop,
    Documents,
    Downloads,
    Pictures,
    Music,
    Videos,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Applications,
}

impl Kind {
    pub fn icon(self) -> &'static str {
        match self {
            Kind::Home => "🏠",
            Kind::Desktop => "🖵",
            Kind::Documents => "🗐",
            Kind::Downloads => "⮋",
            Kind::Pictures => "🖼",
            Kind::Music => "🎵",
            Kind::Videos => "🎞",
            Kind::Applications => "🗔",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Place {
    pub kind: Kind,
    pub name: String,
    pub path: PathBuf,
}

pub fn home() -> Option<PathBuf> {
    #[cfg(windows)]
    let h = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let h = std::env::var_os("HOME");
    h.filter(|h| !h.is_empty()).map(PathBuf::from)
}

/// The standard places that exist on this computer, in sidebar order.
pub fn standard() -> Vec<Place> {
    let Some(home) = home() else { return Vec::new() };
    let mut out = vec![Place { kind: Kind::Home, name: "Home".into(), path: home.clone() }];
    for (kind, name, path) in folders(&home) {
        // These are local folders in the user's profile, so checking them is quick.
        if path != home && path.is_dir() {
            out.push(Place { kind, name: name.into(), path });
        }
    }
    out
}

#[cfg(target_os = "linux")]
fn folders(home: &Path) -> Vec<(Kind, &'static str, PathBuf)> {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
    let dirs = std::fs::read_to_string(config.join("user-dirs.dirs")).map(|s| parse_user_dirs(&s, home)).unwrap_or_default();
    let get = |key: &str, default: &str| dirs.iter().find(|(k, _)| k == key).map(|(_, p)| p.clone()).unwrap_or_else(|| home.join(default));
    vec![
        (Kind::Desktop, "Desktop", get("DESKTOP", "Desktop")),
        (Kind::Documents, "Documents", get("DOCUMENTS", "Documents")),
        (Kind::Downloads, "Downloads", get("DOWNLOAD", "Downloads")),
        (Kind::Pictures, "Pictures", get("PICTURES", "Pictures")),
        (Kind::Music, "Music", get("MUSIC", "Music")),
        (Kind::Videos, "Videos", get("VIDEOS", "Videos")),
    ]
}

#[cfg(target_os = "macos")]
fn folders(home: &Path) -> Vec<(Kind, &'static str, PathBuf)> {
    vec![
        (Kind::Applications, "Applications", PathBuf::from("/Applications")),
        (Kind::Desktop, "Desktop", home.join("Desktop")),
        (Kind::Documents, "Documents", home.join("Documents")),
        (Kind::Downloads, "Downloads", home.join("Downloads")),
        (Kind::Pictures, "Pictures", home.join("Pictures")),
        (Kind::Music, "Music", home.join("Music")),
        (Kind::Videos, "Movies", home.join("Movies")),
    ]
}

#[cfg(windows)]
fn folders(home: &Path) -> Vec<(Kind, &'static str, PathBuf)> {
    use windows_sys::Win32::UI::Shell as s;
    // Known Folders follow redirection (Documents on OneDrive, Downloads moved to D:\…).
    let known = |id: &windows_sys::core::GUID, fallback: &str| known_folder(id).unwrap_or_else(|| home.join(fallback));
    vec![
        (Kind::Desktop, "Desktop", known(&s::FOLDERID_Desktop, "Desktop")),
        (Kind::Documents, "Documents", known(&s::FOLDERID_Documents, "Documents")),
        (Kind::Downloads, "Downloads", known(&s::FOLDERID_Downloads, "Downloads")),
        (Kind::Pictures, "Pictures", known(&s::FOLDERID_Pictures, "Pictures")),
        (Kind::Music, "Music", known(&s::FOLDERID_Music, "Music")),
        (Kind::Videos, "Videos", known(&s::FOLDERID_Videos, "Videos")),
    ]
}

#[cfg(windows)]
fn known_folder(id: &windows_sys::core::GUID) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::SHGetKnownFolderPath;
    let mut p: windows_sys::core::PWSTR = std::ptr::null_mut();
    let hr = unsafe { SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut p) };
    let out = if hr == 0 && !p.is_null() {
        let len = (0..).take_while(|&i| unsafe { *p.add(i) } != 0).count();
        let s = unsafe { std::slice::from_raw_parts(p, len) };
        Some(PathBuf::from(std::ffi::OsString::from_wide(s)))
    } else {
        None
    };
    // Freed even on failure, as the documentation asks.
    unsafe { CoTaskMemFree(p as *const _) };
    out
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn folders(home: &Path) -> Vec<(Kind, &'static str, PathBuf)> {
    vec![
        (Kind::Desktop, "Desktop", home.join("Desktop")),
        (Kind::Documents, "Documents", home.join("Documents")),
        (Kind::Downloads, "Downloads", home.join("Downloads")),
    ]
}

/// Parse `~/.config/user-dirs.dirs`: lines like `XDG_DOWNLOAD_DIR="$HOME/Downloads"`.
/// Returns (key without `XDG_` and `_DIR`, path).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_user_dirs(text: &str, home: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let Some(key) = key.trim().strip_prefix("XDG_").and_then(|k| k.strip_suffix("_DIR")) else { continue };
        let value = value.trim().trim_matches('"');
        let path = if let Some(rest) = value.strip_prefix("$HOME") {
            let rest = rest.trim_start_matches('/');
            if rest.is_empty() { home.to_path_buf() } else { home.join(rest) }
        } else if value.starts_with('/') {
            PathBuf::from(value)
        } else {
            continue; // relative paths aren't allowed by the spec
        };
        out.push((key.to_string(), path));
    }
    out
}

/// The virtual top level: "This PC" on Windows, "Computer" elsewhere.
pub fn computer_name() -> &'static str {
    if cfg!(windows) { "This PC" } else { "Computer" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_dirs_parse() {
        let home = Path::new("/home/rich");
        let text = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOWNLOAD_DIR=\"$HOME/Téléchargements\"\n\
                    XDG_MUSIC_DIR=\"/mnt/media/music\"\nXDG_TEMPLATES_DIR=\"$HOME/\"\nXDG_BAD_DIR=\"relative\"\nnonsense\n";
        let d = parse_user_dirs(text, home);
        assert_eq!(
            d,
            vec![
                ("DESKTOP".to_string(), PathBuf::from("/home/rich/Desktop")),
                ("DOWNLOAD".to_string(), PathBuf::from("/home/rich/Téléchargements")),
                ("MUSIC".to_string(), PathBuf::from("/mnt/media/music")),
                ("TEMPLATES".to_string(), PathBuf::from("/home/rich")),
            ]
        );
    }

    #[test]
    fn standard_places_start_with_home() {
        if let Some(h) = home() {
            let p = standard();
            assert_eq!(p[0].kind, Kind::Home);
            assert_eq!(p[0].path, h);
            assert!(p.iter().skip(1).all(|p| p.path != h));
        }
    }
}
