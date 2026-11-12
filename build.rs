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
}

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
