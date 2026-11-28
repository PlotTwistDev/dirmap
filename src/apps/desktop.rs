//! Linux application launchers (`.desktop` files) and AppImages, read as data. Launchers follow
//! freedesktop.org's Desktop Entry Specification; their icons come from the icon themes the way
//! the Icon Theme Specification looks them up (the hicolor theme every app installs into, then
//! the old pixmaps folder). AppImages are recognised from their ELF header.

use std::path::{Path, PathBuf};

/// What a launcher says, in the user's language where it has one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    /// "Application", "Link" (a web address) or "Directory".
    pub kind: String,
    pub name: Option<String>,
    /// "Web Browser".
    pub generic_name: Option<String>,
    pub comment: Option<String>,
    /// An icon name from the icon theme, or a full path.
    pub icon: Option<String>,
    /// The command it runs, as written (with %f and the like).
    pub exec: Option<String>,
    /// A program that must be installed for the launcher to be shown.
    pub try_exec: Option<String>,
    /// The folder the command runs in.
    pub path: Option<String>,
    /// Runs in a terminal window.
    pub terminal: bool,
    /// For a Link: the address it opens.
    pub url: Option<String>,
    /// Hidden from menus (NoDisplay or Hidden).
    pub hidden: bool,
    pub categories: Vec<String>,
    /// The application's version, where its packager wrote one (AppImages do).
    pub version: Option<String>,
}

/// The user's languages, most specific first ("en_GB", "en"), from the environment.
fn languages() -> Vec<String> {
    let lang = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|v| std::env::var(v).ok()).find(|v| !v.is_empty()).unwrap_or_default();
    let base = lang.split(['.', '@']).next().unwrap_or("").to_string();
    let mut out = Vec::new();
    if !base.is_empty() && base != "C" && base != "POSIX" {
        if let Some((l, _)) = base.split_once('_') {
            out.push(base.clone());
            out.push(l.to_string());
        } else {
            out.push(base);
        }
    }
    out
}

/// Undo a value's escapes: `\s` `\n` `\t` `\r` `\\`. Others are kept as written, for Exec's own
/// quoting to deal with.
fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Read a launcher. None when it has no `[Desktop Entry]` group.
pub fn entry(text: &str) -> Option<Entry> {
    entry_for(text, &languages())
}

fn entry_for(text: &str, langs: &[String]) -> Option<Entry> {
    let mut in_group = false;
    let mut found = false;
    // (key, locale) -> value, keeping the best locale for each key.
    let mut plain: Vec<(String, String)> = Vec::new();
    let mut local: Vec<(String, usize, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            found |= in_group;
            continue;
        }
        if !in_group {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        if let Some((key, loc)) = k.strip_suffix(']').and_then(|k| k.split_once('[')) {
            if let Some(rank) = langs.iter().position(|l| l == loc) {
                match local.iter_mut().find(|(lk, _, _)| lk == key) {
                    Some(e) if rank < e.1 => *e = (key.to_string(), rank, v.to_string()),
                    Some(_) => {}
                    None => local.push((key.to_string(), rank, v.to_string())),
                }
            }
        } else if !plain.iter().any(|(pk, _)| pk == k) {
            plain.push((k.to_string(), v.to_string()));
        }
    }
    if !found {
        return None;
    }
    let get = |key: &str| -> Option<String> {
        let v = local.iter().find(|(k, _, _)| k == key).map(|e| &e.2).or_else(|| plain.iter().find(|(k, _)| k == key).map(|e| &e.1))?;
        Some(unescape(v)).filter(|v| !v.trim().is_empty())
    };
    let yes = |key: &str| get(key).is_some_and(|v| v.eq_ignore_ascii_case("true"));
    Some(Entry {
        kind: get("Type").unwrap_or_else(|| "Application".into()),
        name: get("Name"),
        generic_name: get("GenericName"),
        comment: get("Comment"),
        icon: get("Icon"),
        exec: get("Exec"),
        try_exec: get("TryExec"),
        path: get("Path"),
        terminal: yes("Terminal"),
        url: get("URL"),
        hidden: yes("NoDisplay") || yes("Hidden"),
        categories: get("Categories").map(|c| c.split(';').map(str::trim).filter(|c| !c.is_empty()).map(String::from).collect()).unwrap_or_default(),
        version: get("X-AppImage-Version"),
    })
}

/// The command line an Exec value runs, split into the program and its arguments, with no
/// files given: `%f` `%F` `%u` `%U` (and the old codes) are dropped, `%i` becomes the icon,
/// `%c` the name and `%k` the launcher's own path. None when it's empty or badly quoted.
pub fn command(exec: &str, e: &Entry, file: &Path) -> Option<Vec<String>> {
    // Split by the spec's quoting: "..." with \" \` \$ \\ escaped inside.
    let mut words: Vec<(String, bool)> = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut had = false;
    let mut it = exec.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' => {
                quoted = !quoted;
                had = true;
            }
            '\\' if quoted => match it.next() {
                Some(n @ ('"' | '`' | '$' | '\\')) => cur.push(n),
                Some(n) => {
                    cur.push('\\');
                    cur.push(n);
                }
                None => return None,
            },
            c if c.is_whitespace() && !quoted => {
                if had || !cur.is_empty() {
                    words.push((std::mem::take(&mut cur), had));
                }
                had = false;
            }
            c => cur.push(c),
        }
    }
    if quoted {
        return None;
    }
    if had || !cur.is_empty() {
        words.push((cur, had));
    }
    let mut out = Vec::new();
    for (w, was_quoted) in words {
        if !was_quoted {
            match w.as_str() {
                "%f" | "%F" | "%u" | "%U" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => continue,
                "%i" => {
                    if let Some(i) = &e.icon {
                        out.push("--icon".into());
                        out.push(i.clone());
                    }
                    continue;
                }
                _ => {}
            }
        }
        // Codes inside a longer word.
        let mut s = String::new();
        let mut it = w.chars();
        while let Some(c) = it.next() {
            if c != '%' {
                s.push(c);
                continue;
            }
            match it.next() {
                Some('%') => s.push('%'),
                Some('c') => s.push_str(e.name.as_deref().unwrap_or("")),
                Some('k') => s.push_str(&file.to_string_lossy()),
                _ => {}
            }
        }
        out.push(s);
    }
    (!out.is_empty() && !out[0].is_empty()).then_some(out)
}

/// Where a program named on a command line is: itself when it's a path, else found on PATH.
pub fn which(program: &str) -> Option<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return p.is_file().then_some(p);
    }
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(program)).find(|p| p.is_file() && executable(p)))
}

/// Whether the file may be run as a program (any execute bit; on other systems, never).
pub fn executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = p;
        false
    }
}

/// The folders the system looks in for shared data: the user's own, then the system's.
fn data_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(d) => out.push(PathBuf::from(d)),
        None => {
            if let Some(h) = std::env::var_os("HOME") {
                out.push(PathBuf::from(h).join(".local/share"));
            }
        }
    }
    let sys = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    out.extend(sys.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    out
}

/// A launcher is trusted when it's installed (in an `applications` folder the system reads
/// menus from) or the user has allowed it to run (its execute bit, which GNOME's and KDE's
/// "Allow launching" set). Others could run anything, so they aren't launched: Filescape never
/// changes the permission itself.
pub fn trusted(file: &Path) -> bool {
    let installed = file.parent().is_some_and(|dir| data_dirs().iter().any(|d| dir.starts_with(d.join("applications"))));
    installed || executable(file)
}

/// The file an icon name stands for, the nearest to `size` pixels: a full path as it is,
/// otherwise looked up in the hicolor theme (every app installs its icon there) in each data
/// folder, then the old pixmaps folder. PNG and SVG only.
pub fn find_icon(name: &str, size: u32) -> Option<PathBuf> {
    find_icon_in(name, size, &data_dirs())
}

fn find_icon_in(name: &str, size: u32, dirs: &[PathBuf]) -> Option<PathBuf> {
    if name.starts_with('/') {
        let p = PathBuf::from(name);
        return p.is_file().then_some(p);
    }
    // Names sometimes carry an extension they shouldn't.
    let name = name.strip_suffix(".png").or_else(|| name.strip_suffix(".svg")).unwrap_or(name);
    if name.is_empty() || name.contains('/') {
        return None;
    }
    const SIZES: [u32; 11] = [16, 22, 24, 32, 48, 64, 96, 128, 256, 512, 1024];
    // The size asked for first, then larger ones (they scale down well), then smaller.
    let mut order: Vec<u32> = SIZES.iter().copied().filter(|&s| s >= size).collect();
    order.extend(SIZES.iter().rev().copied().filter(|&s| s < size));
    let mut bases: Vec<PathBuf> = dirs.iter().map(|d| d.join("icons")).collect();
    if let Some(h) = std::env::var_os("HOME") {
        bases.insert(0, PathBuf::from(h).join(".icons"));
    }
    for base in &bases {
        let theme = base.join("hicolor");
        if !theme.is_dir() {
            continue;
        }
        for s in &order {
            let p = theme.join(format!("{s}x{s}/apps/{name}.png"));
            if p.is_file() {
                return Some(p);
            }
        }
        let p = theme.join(format!("scalable/apps/{name}.svg"));
        if p.is_file() {
            return Some(p);
        }
    }
    for d in dirs {
        for ext in ["png", "svg"] {
            let p = d.join(format!("pixmaps/{name}.{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// A launcher's icon as pixels, about `size` across.
pub fn icon_file(file: &Path, size: u32) -> Option<image::RgbaImage> {
    let meta = std::fs::metadata(file).ok()?;
    if meta.len() > 1 << 20 {
        return None;
    }
    let e = entry(&std::fs::read_to_string(file).ok()?)?;
    load_icon(&find_icon(e.icon.as_deref()?, size)?, size)
}

fn load_icon(p: &Path, size: u32) -> Option<image::RgbaImage> {
    if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz")) {
        let d = crate::svg::render(p, size).ok()?;
        return image::RgbaImage::from_raw(d.width, d.height, d.rgba);
    }
    if std::fs::metadata(p).ok()?.len() > 8 << 20 {
        return None;
    }
    let img = image::open(p).ok()?.into_rgba8();
    Some(if img.width() > size * 2 { image::imageops::resize(&img, size, size, image::imageops::FilterType::Triangle) } else { img })
}

/// What an ELF program's header says.
#[derive(Clone, Debug, PartialEq)]
pub struct Elf {
    /// "64-bit x86-64", "ARM64".
    pub arch: Option<String>,
    /// 1 or 2 for an AppImage, else None.
    pub appimage: Option<u8>,
}

/// Read an ELF header (the first 20 bytes are enough). None when it isn't one.
pub fn elf(head: &[u8]) -> Option<Elf> {
    if head.get(..4)? != b"\x7fELF" {
        return None;
    }
    let little = *head.get(5)? == 1;
    let machine = if little { u16::from_le_bytes([*head.get(18)?, *head.get(19)?]) } else { u16::from_be_bytes([*head.get(18)?, *head.get(19)?]) };
    let arch = match machine {
        0x3e => Some("64-bit x86-64"),
        0x03 => Some("32-bit x86"),
        0xb7 => Some("ARM64"),
        0x28 => Some("32-bit ARM"),
        0xf3 => Some("RISC-V"),
        0x15 => Some("PowerPC 64-bit"),
        _ => None,
    }
    .map(String::from);
    // AppImages mark themselves in the header's padding: "AI" and their type.
    let appimage = (head.get(8..10)? == b"AI").then(|| head[10]).filter(|t| *t == 1 || *t == 2);
    Some(Elf { arch, appimage })
}

/// Read a file's ELF header.
pub fn elf_file(p: &Path) -> Option<Elf> {
    use std::io::Read;
    let mut head = [0u8; 64];
    let n = std::fs::File::open(p).ok()?.read(&mut head).ok()?;
    elf(&head[..n])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str = "\
# A comment
[Desktop Entry]
Type=Application
Name=Paint
Name[fr]=Peinture
Name[en_GB]=Paint (UK)
GenericName=Image Editor
Comment=Draw\\sthings
Icon=acme-paint
Exec=env FOO=1 \"/opt/Acme Paint/paint\" --name %c %U
Path=/opt/Acme Paint
Terminal=false
Categories=Graphics;2DGraphics;
X-AppImage-Version=2.1

[Desktop Action New]
Name=New Window
Exec=paint --new
";

    #[test]
    fn reads_a_launcher() {
        let e = entry_for(SAMPLE, &[]).unwrap();
        assert_eq!(e.kind, "Application");
        assert_eq!(e.name.as_deref(), Some("Paint"));
        assert_eq!(e.generic_name.as_deref(), Some("Image Editor"));
        assert_eq!(e.comment.as_deref(), Some("Draw things"));
        assert_eq!(e.icon.as_deref(), Some("acme-paint"));
        assert_eq!(e.path.as_deref(), Some("/opt/Acme Paint"));
        assert_eq!(e.categories, ["Graphics", "2DGraphics"]);
        assert_eq!(e.version.as_deref(), Some("2.1"));
        assert!(!e.terminal && !e.hidden);
        // The best language wins, and keys of other groups are ignored.
        assert_eq!(entry_for(SAMPLE, &["en_GB".into(), "en".into()]).unwrap().name.as_deref(), Some("Paint (UK)"));
        assert_eq!(entry_for(SAMPLE, &["fr_CA".into(), "fr".into()]).unwrap().name.as_deref(), Some("Peinture"));
        assert!(entry_for("[Something Else]\nName=x\n", &[]).is_none());
        assert!(entry_for("just text", &[]).is_none());
    }

    #[test]
    fn builds_the_command() {
        let e = entry_for(SAMPLE, &[]).unwrap();
        let f = Path::new("/home/a/paint.desktop");
        assert_eq!(command(e.exec.as_deref().unwrap(), &e, f).unwrap(), ["env", "FOO=1", "/opt/Acme Paint/paint", "--name", "Paint"]);
        assert_eq!(command("app %i %k 100%%", &e, f).unwrap(), ["app", "--icon", "acme-paint", "/home/a/paint.desktop", "100%"]);
        assert_eq!(command(r#"sh -c "echo \"hi\" \$HOME""#, &e, f).unwrap(), ["sh", "-c", "echo \"hi\" $HOME"]);
        assert_eq!(command(r#""""#, &e, f), None);
        assert_eq!(command("open \"unclosed", &e, f), None);
        assert_eq!(command("   ", &e, f), None);
    }

    #[test]
    fn finds_icons_in_the_theme() {
        let tmp = std::env::temp_dir().join(format!("fs-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let theme = tmp.join("icons/hicolor");
        for s in ["32x32", "128x128"] {
            std::fs::create_dir_all(theme.join(s).join("apps")).unwrap();
            image::RgbaImage::new(4, 4).save(theme.join(s).join("apps/acme.png")).unwrap();
        }
        std::fs::create_dir_all(theme.join("scalable/apps")).unwrap();
        std::fs::write(theme.join("scalable/apps/vector.svg"), "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'/>").unwrap();
        std::fs::create_dir_all(tmp.join("pixmaps")).unwrap();
        image::RgbaImage::new(4, 4).save(tmp.join("pixmaps/old.png")).unwrap();
        let dirs = [tmp.clone()];
        assert_eq!(find_icon_in("acme", 64, &dirs), Some(theme.join("128x128/apps/acme.png")));
        assert_eq!(find_icon_in("acme.png", 16, &dirs), Some(theme.join("32x32/apps/acme.png")));
        assert_eq!(find_icon_in("vector", 64, &dirs), Some(theme.join("scalable/apps/vector.svg")));
        assert_eq!(find_icon_in("old", 64, &dirs), Some(tmp.join("pixmaps/old.png")));
        assert_eq!(find_icon_in("../../etc/passwd", 64, &dirs), None);
        assert_eq!(find_icon_in("missing", 64, &dirs), None);
        assert!(load_icon(&theme.join("scalable/apps/vector.svg"), 32).is_some());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn reads_elf_headers() {
        let mut h = vec![0u8; 64];
        h[..4].copy_from_slice(b"\x7fELF");
        h[4] = 2;
        h[5] = 1;
        h[18] = 0x3e;
        assert_eq!(elf(&h), Some(Elf { arch: Some("64-bit x86-64".into()), appimage: None }));
        h[8..11].copy_from_slice(b"AI\x02");
        assert_eq!(elf(&h).unwrap().appimage, Some(2));
        h[18] = 0xb7;
        assert_eq!(elf(&h).unwrap().arch.as_deref(), Some("ARM64"));
        assert_eq!(elf(b"MZ not an elf"), None);
        assert_eq!(elf(b"\x7fEL"), None);
    }
}
