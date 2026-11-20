//! dirmap: a WinDirStat-style disk usage visualizer.
//!
//!   dirmap [PATH]                      open the GUI (and scan PATH if given)
//!   dirmap --cli PATH [--top N]        print a text report instead
//!   --all-fs                           descend into other mounted filesystems
//!   --save-snapshot FILE               (with --cli) also save the scan for later comparison
//!   --compare FILE                     open the GUI comparing against a saved snapshot

// No console window behind the GUI on Windows; --cli attaches to the calling terminal instead.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod actions;
mod app;
mod camera;
mod categories;
mod drives;
mod dupes;
mod elevate;
mod fsys;
mod help;
mod icons;
mod mapview;
mod netdrives;
mod pattern;
mod player;
mod settings;
mod shellmenu;
mod subset;
mod scan;
mod sequences;
mod snapshot;
mod sunburst;
mod templates;
mod tilegpu;
mod transfer;
mod media;
mod treemap;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn main() -> eframe::Result {
    crash_log();
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

    // Try one renderer and fall back to the other (old GPUs, VMs, remote desktops).
    let run = |renderer, path: Option<PathBuf>| {
        let native = eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_title("dirmap")
                .with_inner_size([1280.0, 820.0])
                .with_min_inner_size([640.0, 420.0]),
            renderer,
            wgpu_options: wgpu_options(),
            ..Default::default()
        };
        let all_fs = !opts.one_file_system;
        let compare = compare.clone();
        eframe::run_native("dirmap", native, Box::new(move |cc| Ok(Box::new(app::App::new(cc, path, all_fs, compare)))))
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
            "dirmap {} crashed ({}, thread {thread})\n\n{info}\n\n{backtrace}\n",
            env!("CARGO_PKG_VERSION"),
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        );
        let file = eframe::storage_dir("dirmap").map(|d| d.join("crash.log"));
        let saved = file.as_ref().is_some_and(|f| std::fs::create_dir_all(f.parent().unwrap_or(f)).and_then(|_| std::fs::write(f, &text)).is_ok());
        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
            let where_ = if saved { format!("\n\nThe details are saved in\n{}", file.as_ref().unwrap().display()) } else { String::new() };
            let msg = format!("dirmap stopped because of a bug:\n\n{info}{where_}");
            let w = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
            unsafe { MessageBoxW(std::ptr::null_mut(), w(&msg).as_ptr(), w("dirmap crashed").as_ptr(), MB_OK | MB_ICONERROR) };
        }
        let _ = saved;
    }));
}
