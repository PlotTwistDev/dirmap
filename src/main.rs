//! dirmap: a WinDirStat-style disk usage visualizer.
//!
//!   dirmap [PATH]                      open the GUI (and scan PATH if given)
//!   dirmap --cli PATH [--top N]        print a text report instead
//!   --all-fs                           descend into other mounted filesystems
//!   --save-snapshot FILE               (with --cli) also save the scan for later comparison
//!   --compare FILE                     open the GUI comparing against a saved snapshot

// No console window behind the GUI on Windows; --cli attaches to the calling terminal instead.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod camera;
mod categories;
mod drives;
mod dupes;
mod elevate;
mod icons;
mod mapview;
mod settings;
mod scan;
mod snapshot;
mod sunburst;
mod tilegpu;
mod treemap;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn main() -> eframe::Result {
    let mut cli = false;
    let mut top = 15usize;
    let mut path = None;
    let mut save_snapshot = None;
    let mut compare = None;
    let mut restarted = false;
    let mut opts = scan::Options { one_file_system: true, ..Default::default() };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--cli" => cli = true,
            "--all-fs" => opts.one_file_system = false,
            "--top" => top = args.next().and_then(|s| s.parse().ok()).unwrap_or(top),
            "--save-snapshot" => save_snapshot = args.next().map(PathBuf::from),
            "--compare" => compare = args.next().map(PathBuf::from),
            // Added when dirmap restarts itself as administrator; stops it trying twice.
            elevate::ELEVATED_FLAG => restarted = true,
            "-h" | "--help" => {
                println!("usage: dirmap [--cli [--top N] [--save-snapshot FILE]] [--compare FILE] [--all-fs] [PATH]");
                return Ok(());
            }
            _ => path = Some(PathBuf::from(a)),
        }
    }

    if cli {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
        report(&path.unwrap_or_else(|| PathBuf::from(".")), opts, top, save_snapshot);
        return Ok(());
    }

    // "Always run as administrator": hand over to an elevated copy before any window opens.
    // If the Windows prompt is declined, carry on without.
    if elevate::can_elevate()
        && !restarted
        && !elevate::is_elevated()
        && elevate::load_mode() == elevate::AdminMode::Always
        && elevate::relaunch_elevated()
    {
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
        let all_fs = !opts.one_file_system;
        let compare = compare.clone();
        eframe::run_native("dirmap", native, Box::new(move |cc| Ok(Box::new(app::App::new(cc, path, all_fs, compare)))))
    };
    // DIRMAP_RENDERER=glow forces OpenGL (for testing that path).
    if std::env::var("DIRMAP_RENDERER").is_ok_and(|r| r.eq_ignore_ascii_case("glow")) {
        return run(eframe::Renderer::Glow, path);
    }
    match run(eframe::Renderer::Wgpu, path.clone()) {
        Err(eframe::Error::Wgpu(e)) => {
            eprintln!("wgpu unavailable ({e}), falling back to OpenGL");
            run(eframe::Renderer::Glow, path)
        }
        r => r,
    }
}

fn report(root: &std::path::Path, opts: scan::Options, top: usize, save_snapshot: Option<PathBuf>) {
    let progress = Arc::new(scan::Progress::default());
    let t = std::time::Instant::now();
    let tree = scan::scan(root, &opts, &progress, &scan::new_live(root));
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
    if let Some(f) = save_snapshot {
        match snapshot::save(&f, root, &tree) {
            Ok(()) => println!("\nSnapshot saved to {}", f.display()),
            Err(e) => eprintln!("\nCouldn't save the snapshot: {e}"),
        }
    }
}
