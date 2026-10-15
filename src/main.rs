//! dirmap: a WinDirStat-style disk usage visualizer.
//!
//!   dirmap [PATH]                      open the GUI (and scan PATH if given)
//!   dirmap --cli PATH [--top N]        print a text report instead
//!   --all-fs                           descend into other mounted filesystems

mod app;
mod scan;
mod treemap;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn main() -> eframe::Result {
    let mut cli = false;
    let mut top = 15usize;
    let mut path = None;
    let mut opts = scan::Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--cli" => cli = true,
            "--all-fs" => opts.one_file_system = false,
            "--top" => top = args.next().and_then(|s| s.parse().ok()).unwrap_or(top),
            "-h" | "--help" => {
                println!("usage: dirmap [--cli] [--top N] [--all-fs] [PATH]");
                return Ok(());
            }
            _ => path = Some(PathBuf::from(a)),
        }
    }

    if cli {
        report(&path.unwrap_or_else(|| PathBuf::from(".")), opts, top);
        return Ok(());
    }

    // Try the default GPU renderer first and fall back to OpenGL (old GPUs, VMs, remote desktops).
    let run = |renderer, path: Option<PathBuf>| {
        let native = eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_title("dirmap")
                .with_inner_size([1280.0, 820.0]),
            renderer,
            ..Default::default()
        };
        eframe::run_native("dirmap", native, Box::new(move |_cc| Ok(Box::new(app::App::new(path, opts)))))
    };
    match run(eframe::Renderer::Wgpu, path.clone()) {
        Err(eframe::Error::Wgpu(e)) => {
            eprintln!("wgpu unavailable ({e}), falling back to OpenGL");
            run(eframe::Renderer::Glow, path)
        }
        r => r,
    }
}

fn report(root: &std::path::Path, opts: scan::Options, top: usize) {
    let progress = Arc::new(scan::Progress::default());
    let t = std::time::Instant::now();
    let tree = scan::scan(root, opts, &progress);
    println!(
        "{}: {} in {} files, {} folders ({:.2}s, {} unreadable)",
        root.display(),
        scan::human(tree.size),
        tree.files,
        progress.dirs.load(Ordering::Relaxed),
        t.elapsed().as_secs_f64(),
        progress.errors.load(Ordering::Relaxed)
    );
    let pct = |b: u64| if tree.size > 0 { b as f64 * 100.0 / tree.size as f64 } else { 0.0 };

    println!("\nLargest entries:");
    for c in tree.children.iter().take(top) {
        let slash = if c.is_dir { "/" } else { "" };
        println!("  {:>10}  {:5.1}%  {}{}", scan::human(c.size), pct(c.size), c.name, slash);
    }

    println!("\nBy extension:");
    for e in scan::ext_stats(&tree).iter().take(top) {
        let name = if e.ext.is_empty() { "(none)".into() } else { format!(".{}", e.ext) };
        println!("  {:>10}  {:5.1}%  {:>8} files  {}", scan::human(e.bytes), pct(e.bytes), e.files, name);
    }
}
