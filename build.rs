//! Bundles the help pages in `docs/src` (Markdown and images) into the program, so the Help
//! Centre works offline and always matches the version it ships with. Also puts the AI engine's
//! libraries (Windows, Linux) beside the program, and tells the Linux program to look there.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    // On Linux the AI engine's libraries sit beside the program (the .tar.gz) or in
    // ../lib/filescape from it (the .deb, .rpm and AppImage): look for them there.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN:$ORIGIN/../lib/filescape");
    }
    engine_beside_program();
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

/// Copies the AI engine's libraries (llama.cpp, ggml and its backends: a processor build for
/// each kind of x86 processor, and Vulkan) from where llama-cpp-sys-2 built them to beside the
/// program and its tests, so `target/release/filescape` runs as it is and packaging finds them in
/// one place. Only on Windows and Linux; the Mac build has the engine built in.
fn engine_beside_program() {
    println!("cargo:rerun-if-env-changed=DEP_LLAMA_ROOT");
    let Ok(root) = std::env::var("DEP_LLAMA_ROOT") else { return };
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let root = PathBuf::from(root);
    // target/<profile>: the folder above `build`, wherever OUT_DIR sits under it.
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let Some(profile) = out.ancestors().find(|a| a.file_name().is_some_and(|n| n == "build")).and_then(Path::parent) else { return };
    let mut files = Vec::new();
    let (libs, ok): (PathBuf, fn(&str) -> bool) = if windows {
        (root.join("bin"), |n| n.ends_with(".dll"))
    } else {
        // The names the program asks for (libllama.so.0); the files they point to are copied.
        (root.join("lib"), |n| (n.starts_with("libllama") || n.starts_with("libggml")) && n.ends_with(".so.0"))
    };
    for dir in [libs, root.join("backends")] {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if ok(&name) || (dir.ends_with("backends") && (name.ends_with(".so") || name.ends_with(".dll"))) {
                files.push(e.path());
            }
        }
    }
    for dest in [profile.to_path_buf(), profile.join("deps")] {
        for f in &files {
            let to = dest.join(f.file_name().unwrap());
            // A link left by an earlier build would be written through: replace it.
            if to.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
                let _ = std::fs::remove_file(&to);
            }
            let same = |a: &Path, b: &Path| match (std::fs::metadata(a), std::fs::metadata(b)) {
                (Ok(x), Ok(y)) => x.len() == y.len() && x.modified().ok() <= y.modified().ok(),
                _ => false,
            };
            if !same(f, &to) {
                let _ = std::fs::copy(f, &to);
            }
        }
    }
}
