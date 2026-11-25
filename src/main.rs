//! dirmap: a WinDirStat-style disk usage visualizer.
//!
//!   dirmap [PATH]                      open the GUI (and scan PATH if given)
//!   dirmap --cli PATH [--top N]        print a text report instead
//!   --logical                          (with --cli) rank by logical size instead of size on disk
//!   --cloud-only                       (with --cli) rank by data only in the cloud
//!   --reclaimable                      (with --cli) rank by space freeing up cloud files would give back
//!   --cloud-mounts                     also walk mounted cloud drives (rclone, FUSE) inside PATH
//!   --all-fs                           descend into other mounted filesystems
//!   --save-snapshot FILE               (with --cli) also save the scan for later comparison
//!   --compare FILE                     open the GUI comparing against a saved snapshot

// No console window behind the GUI on Windows; --cli attaches to the calling terminal instead.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod asking;
mod actions;
mod app;
mod camera;
mod camraw;
mod categories;
mod cloud;
mod cloudops;
mod drives;
mod dupes;
mod epub;
mod elevate;
mod fontview;
mod formats;
mod fsys;
mod help;
mod highlight;
mod icons;
mod known;
mod kit;
mod licence;
mod models;
mod mapview;
mod netdrives;
mod officedoc;
mod pdfview;
mod pattern;
mod player;
mod psdview;
mod settings;
mod sheets;
mod shellmenu;
mod subset;
mod scan;
mod sequences;
mod snapshot;
mod soundinfo;
mod sunburst;
mod svg;
mod templates;
mod tilegpu;
mod video;
mod wave;
#[cfg(target_os = "macos")]
mod macaudio;
mod transfer;
mod media;
mod treemap;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// The product name. Also names the settings folder (see `templates::app_folder`).
pub const APP_ID: &str = "Filescape";

fn main() -> eframe::Result {
    templates::migrate_old_folder();
    crash_log();
    let mut cli = false;
    let mut top = 15usize;
    let mut path = None;
    let mut save_snapshot = None;
    let mut compare = None;
    let mut restarted = false;
    let mut manage = None;
    let mut list_models = false;
    let mut opts = scan::Options { one_file_system: true, ..Default::default() };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--cli" => cli = true,
            "--all-fs" => opts.one_file_system = false,
            "--logical" => opts.metric = scan::Metric::Logical,
            "--cloud-only" => opts.metric = scan::Metric::CloudOnly,
            "--reclaimable" => opts.metric = scan::Metric::Reclaimable,
            "--cloud-mounts" => opts.cloud_mounts = true,
            "--free-up" => manage = Some(cloudops::Action::FreeUp),
            "--keep" => manage = Some(cloudops::Action::Keep),
            "--models" => list_models = true,
            "--top" => top = args.next().and_then(|s| s.parse().ok()).unwrap_or(top),
            "--save-snapshot" => save_snapshot = args.next().map(PathBuf::from),
            "--compare" => compare = args.next().map(PathBuf::from),
            // Added when Filescape restarts itself as administrator; stops it trying twice.
            elevate::ELEVATED_FLAG => restarted = true,
            "-h" | "--help" => {
                println!("usage: filescape [--cli [--top N] [--logical | --cloud-only | --reclaimable] [--save-snapshot FILE]] [--cloud-mounts] [--compare FILE] [--all-fs] [PATH]");
                println!("       filescape --cli --free-up PATH   free up the cloud files in PATH (they stay in the cloud)");
                println!("       filescape --cli --keep PATH      keep the cloud files in PATH on this computer (downloads them)");
                println!("       filescape --cli --models PATH    list the AI models in PATH and their copies");
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
        let path = path.unwrap_or_else(|| PathBuf::from("."));
        if let Some(a) = manage {
            if !cloudops::supported() {
                eprintln!("{}: this system has no supported way to free up or keep cloud files.", a.label());
                std::process::exit(2);
            }
            // Like bulk work in the app, freeing up or keeping from the command line is Pro.
            if !licence::Licensing::load().allows(licence::Feature::CloudManagement) {
                eprintln!("{}: freeing up or keeping cloud files from the command line needs a Filescape Pro licence. Enter one in the app under Settings > Licensing, or use the file's right-click menu in the app to do one file at a time.", a.label());
                std::process::exit(3);
            }
            let o = cloudops::apply(std::slice::from_ref(&path), a, &std::sync::atomic::AtomicBool::new(false));
            println!("{}", o.summary(a));
            for (p, why) in &o.reasons {
                println!("  {}: {why}", p.display());
            }
            std::process::exit(if o.failed > 0 { 1 } else { 0 });
        }
        if list_models {
            models_report(&path, opts);
            return Ok(());
        }
        report(&path, opts, top, save_snapshot);
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

    // Try one renderer and fall back to the other (old GPUs, VMs, remote desktops).
    let run = |renderer, path: Option<PathBuf>| {
        let native = eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_title(APP_ID)
                .with_inner_size([1280.0, 820.0])
                .with_min_inner_size([640.0, 420.0])
                .with_icon(app_icon()),
            renderer,
            wgpu_options: wgpu_options(),
            ..Default::default()
        };
        let all_fs = !opts.one_file_system;
        let compare = compare.clone();
        eframe::run_native(APP_ID, native, Box::new(move |cc| Ok(Box::new(app::App::new(cc, path, all_fs, compare)))))
    };
    // On Windows, OpenGL comes first: with DirectX 12 and Vulkan the panels shake while the
    // window edge is dragged (Rich, 18 Nov 2026). DIRMAP_RENDERER=glow or wgpu picks one.
    let choice = std::env::var("DIRMAP_RENDERER").unwrap_or_default().to_ascii_lowercase();
    let glow_first = match choice.as_str() {
        "glow" | "opengl" => true,
        "wgpu" => false,
        _ => cfg!(windows),
    };
    if glow_first {
        return match run(eframe::Renderer::Glow, path.clone()) {
            Err(eframe::Error::Glutin(e)) if choice.is_empty() => {
                eprintln!("OpenGL unavailable ({e}), trying wgpu");
                run(eframe::Renderer::Wgpu, path)
            }
            Err(eframe::Error::NoGlutinConfigs(_, e)) if choice.is_empty() => {
                eprintln!("OpenGL unavailable ({e}), trying wgpu");
                run(eframe::Renderer::Wgpu, path)
            }
            r => r,
        };
    }
    match run(eframe::Renderer::Wgpu, path.clone()) {
        Err(eframe::Error::Wgpu(e)) => {
            eprintln!("wgpu unavailable ({e}), falling back to OpenGL");
            run(eframe::Renderer::Glow, path)
        }
        r => r,
    }
}

/// `--cli --models`: the AI models in a folder, largest first.
fn models_report(root: &std::path::Path, opts: scan::Options) {
    let progress = Arc::new(scan::Progress::default());
    let tree = scan::scan(root, &opts, &progress, &scan::new_live(root));
    let Some(list) = models::find(&tree, root, &models::Progress::default()) else { return };
    let total: u64 = list.iter().map(|m| m.size).sum();
    println!("{}: {} AI models, {}", root.display(), list.iter().filter(|m| !m.orphan).count(), scan::human(total));
    for m in &list {
        let mut note = String::new();
        if m.orphan {
            note.push_str("  [not used by any Ollama model]");
        } else if m.extra > 0 {
            note.push_str("  [copy]");
        } else if !m.copies.is_empty() {
            note.push_str("  [has copies]");
        }
        if m.runnable() {
            note.push_str("  [can run]");
        }
        println!("{:>10}  {}  ({} · {}){note}", scan::human(m.size), m.name, m.details(), m.source.label());
        for f in &m.files {
            println!("{:>10}    {}", "", f.path.display());
        }
    }
    let extra: u64 = list.iter().map(|m| m.extra).sum();
    if extra > 0 {
        println!("{} in extra copies", scan::human(extra));
    }
}

fn report(root: &std::path::Path, opts: scan::Options, top: usize, save_snapshot: Option<PathBuf>) {
    let progress = Arc::new(scan::Progress::default());
    let t = std::time::Instant::now();
    let tree = scan::scan(root, &opts, &progress, &scan::new_live(root));
    println!(
        "{}: {} on disk, {} logical, in {} files, {} folders ({:.2}s, {} unreadable)",
        root.display(),
        scan::human(tree.alloc),
        scan::human(tree.logical),
        tree.files,
        progress.dirs.load(Ordering::Relaxed),
        t.elapsed().as_secs_f64(),
        progress.errors.load(Ordering::Relaxed)
    );
    let cloud = scan::cloud_tally(&tree);
    if cloud.any() || cloud.online_only > 0 {
        let roots = crate::cloud::roots();
        let mut names: Vec<&str> = crate::cloud::root_of(&roots, root).into_iter().chain(crate::cloud::roots_below(&roots, root)).map(|r| r.provider.name()).collect();
        names.sort_unstable();
        names.dedup();
        println!(
            "Cloud{}: {} files, {} downloaded, {} online-only ({} only in the cloud), {} partly downloaded, {} unknown, {} can be freed up",
            if names.is_empty() { String::new() } else { format!(" ({})", names.join(", ")) },
            cloud.cloud_files,
            cloud.downloaded,
            cloud.online_only,
            scan::human(cloud.online_bytes),
            cloud.partial,
            cloud.unknown,
            scan::human(cloud.reclaimable_bytes)
        );
    }
    println!("Sizes below: {} (bytes are {})", opts.metric.label().to_lowercase(), scan::human(tree.size));
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

/// The wgpu setup (used when DIRMAP_RENDERER=wgpu on Windows). Use DirectX 12 rather than letting wgpu pick: its Vulkan path
/// presents several frames late on many machines, so highlights trail behind the mouse.
/// `WGPU_BACKEND` (vulkan, dx12, gl) still overrides it, for testing.
fn wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    #[allow(unused_mut)]
    let mut opts = eframe::egui_wgpu::WgpuConfiguration::default().with_surface_config(eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY);
    #[cfg(windows)]
    if std::env::var_os("WGPU_BACKEND").is_none() {
        let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
        setup.instance_descriptor.backends = eframe::wgpu::Backends::DX12;
        opts.wgpu_setup = eframe::egui_wgpu::WgpuSetup::CreateNew(setup);
    }
    opts
}

/// On a panic, write what happened to `crash.log` in dirmap's settings folder, and on Windows
/// say so in a message box: the window has no console, so the message would otherwise be lost.
fn crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let backtrace = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let text = format!(
            "Filescape {} crashed ({}, thread {thread})\n\n{info}\n\n{backtrace}\n",
            env!("CARGO_PKG_VERSION"),
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        );
        let file = eframe::storage_dir(APP_ID).map(|d| d.join("crash.log"));
        let saved = file.as_ref().is_some_and(|f| std::fs::create_dir_all(f.parent().unwrap_or(f)).and_then(|_| std::fs::write(f, &text)).is_ok());
        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
            let where_ = if saved { format!("\n\nThe details are saved in\n{}", file.as_ref().unwrap().display()) } else { String::new() };
            let msg = format!("Filescape stopped because of a bug:\n\n{info}{where_}");
            let w = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
            unsafe { MessageBoxW(std::ptr::null_mut(), w(&msg).as_ptr(), w("Filescape crashed").as_ptr(), MB_OK | MB_ICONERROR) };
        }
        let _ = saved;
    }));
}

/// The window and taskbar icon: the tile-grid logo from filescape.app (packaging/filescape.svg).
fn app_icon() -> std::sync::Arc<eframe::egui::IconData> {
    let png = include_bytes!("../packaging/filescape-window.png");
    let icon = match image::load_from_memory_with_format(png, image::ImageFormat::Png) {
        Ok(img) => {
            let img = img.to_rgba8();
            let (width, height) = img.dimensions();
            eframe::egui::IconData { rgba: img.into_raw(), width, height }
        }
        Err(_) => eframe::egui::IconData::default(),
    };
    std::sync::Arc::new(icon)
}
