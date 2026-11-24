//! Bundles the help pages in `docs/src` (Markdown and images) into the program, so the Help
//! Centre works offline and always matches the version it ships with.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs").join("src");
    // A directory here makes Cargo rerun this script when anything inside it changes.
    println!("cargo:rerun-if-changed=docs/src");

    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();

    let mut out = String::from("/// Help pages: (path under docs/src with `/` separators, Markdown).\n");
    out.push_str("pub static PAGES: &[(&str, &str)] = &[\n");
    for f in files.iter().filter(|f| has_ext(f, "md")) {
        writeln!(out, "    ({:?}, include_str!({:?})),", rel(&root, f), f.display().to_string()).unwrap();
    }
    out.push_str("];\n\n/// Help images: (path under docs/src, PNG bytes).\n");
    out.push_str("pub static IMAGES: &[(&str, &[u8])] = &[\n");
    for f in files.iter().filter(|f| has_ext(f, "png")) {
        writeln!(out, "    ({:?}, include_bytes!({:?})),", rel(&root, f), f.display().to_string()).unwrap();
    }
    out.push_str("];\n");

    let dest = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("help_bundle.rs");
    std::fs::write(dest, out).unwrap();

    windows_icon();
    release_date();
    no_test_mode_in_release();
}

/// The release date the update entitlement is checked against (`FILESCAPE_RELEASE_DATE`, Unix
/// seconds). Taken from `SOURCE_DATE_EPOCH` if set, else the date of the commit being built, so it
/// can't be forgotten at release time. Without git (a source tarball) it falls back to now.
fn release_date() {
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
    let from_env = std::env::var("SOURCE_DATE_EPOCH").ok().and_then(|s| s.trim().parse::<i64>().ok());
    let from_git = || {
        let out = std::process::Command::new("git").args(["log", "-1", "--format=%ct"]).output().ok()?;
        String::from_utf8(out.stdout).ok()?.trim().parse::<i64>().ok()
    };
    let now = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let ts = from_env.or_else(from_git).unwrap_or_else(now);
    println!("cargo:rustc-env=FILESCAPE_RELEASE_DATE={ts}");
}

/// Test mode issues its own serials, so it must never reach customers.
fn no_test_mode_in_release() {
    println!("cargo:rerun-if-env-changed=FILESCAPE_ALLOW_LICENCE_DEV");
    let dev = std::env::var_os("CARGO_FEATURE_LICENCE_DEV").is_some();
    let release = std::env::var("PROFILE").is_ok_and(|p| p == "release");
    let allowed = std::env::var("FILESCAPE_ALLOW_LICENCE_DEV").is_ok_and(|v| v == "1");
    if dev && release && !allowed {
        panic!("the licence-dev feature (test serials) must not be in a release build; set FILESCAPE_ALLOW_LICENCE_DEV=1 only for a local test build");
    }
}

/// The app icon for Explorer, the taskbar and Alt+Tab, built into filescape.exe. Needs the
/// Windows SDK's resource compiler; without it the program still builds, just without the icon.
#[cfg(windows)]
fn windows_icon() {
    println!("cargo:rerun-if-changed=packaging/filescape.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("packaging/filescape.ico");
    if let Err(e) = res.compile() {
        println!("cargo:warning=no icon in filescape.exe: {e}");
    }
}

#[cfg(not(windows))]
fn windows_icon() {}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

fn rel(root: &Path, p: &Path) -> String {
    let r = p.strip_prefix(root).unwrap();
    r.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}
