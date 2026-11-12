# dirmap

A WinDirStat-style disk usage visualizer written in Rust (egui/eframe).

![screenshot](screenshot.png)

- **Drive picker** listing every drive (or mounted volume on Linux/macOS) with total, free and used space,
  network drives included. Click one to scan the whole drive, e.g. `D:\`.
- **Live scan view**: folders and files appear as they are found, with running sizes, a progress bar
  (an estimate when scanning a whole drive), speed, and the folder being read.
- **Parallel scan** (rayon) that never follows symlinks or junctions and, by default, stays on one drive.
- **Directory tree** with Explorer's own file icons on Windows, sortable Name / Size / Files / Last change
  columns and percentage bars. Only visible rows are drawn, so huge folders stay fast.
- **File types list** with totals per extension. Click one to highlight those files in the treemap.
- **Cushion treemap** (van Wijk & van de Wetering, like WinDirStat) colored by file type. Hovering shows the
  path, size and date in the status bar; click to select (the tree jumps to it), double-click or wheel up to zoom
  into a folder, right-click or wheel down to zoom out, and use the breadcrumbs to jump back up.
  File names and sizes are drawn on tiles that have room for them (can be turned off).
- **Treemap filters** that blank out everything else: by kind (audio, video, images, AI model weights such as
  gguf / safetensors / pt / onnx, archives, documents, code, programs, disk images) and by size
  (larger than / smaller than a size such as `500 MB` or `2 GB`), with a count of how many files and bytes match.
- **Sunburst view**: a ring chart of the same data, each ring one folder level deeper. Click to select,
  double-click or wheel to zoom, click the middle to go up.
- **Color by type, age or change**: age runs from red (changed today) to blue (10+ years old); change shows
  new, grown and shrunk files since a snapshot.
- **Size histogram** (Sizes tab): how many files and how much space fall in each size range. Click a bar to
  show only that range on the map.
- **Compare with a snapshot** (Changes tab): save a scan, scan again later, and see what is new, deleted, grew
  or shrank. `dirmap --cli PATH --save-snapshot FILE` saves one from a script; `dirmap --compare FILE PATH`
  opens the GUI comparing against it.
- **Duplicate finder** (Duplicates tab): files with identical contents, matched by size, then the first 64 KB,
  then the whole file, with how much space removing the extra copies would free.
- **Settings** (saved between runs): theme, size units, filters (folders and file patterns to skip or include,
  minimum size, maximum age), tree columns, treemap look with live preview, hidden files, scanner threads.

## Download

Every push to `main` builds Windows, Linux and macOS versions on GitHub Actions (the **build**
workflow; open a run and download from *Artifacts*). Pushing a tag such as `v0.2.0` also attaches them
to a GitHub release.

- **Windows**: `dirmap-windows-x64.zip` contains `dirmap.exe`. No install needed.
- **Linux**: `dirmap-linux-x64.tar.gz` contains the `dirmap` binary and a `dirmap.desktop` launcher
  (copy it to `~/.local/share/applications/`). Needs X11 or Wayland plus `libxkbcommon-x11`, which
  desktop installs already have.
- **macOS** (11 Big Sur or later, Apple silicon and Intel): `dirmap-macos-universal.zip` contains
  `dirmap.app`. It isn't notarized, so the first time, right-click it and choose **Open**, or run
  `xattr -dr com.apple.quarantine dirmap.app`. To scan your whole disk including other apps' data,
  give it **Full Disk Access** in System Settings > Privacy & Security; without it those folders are
  counted as unreadable.

## Build from source

Install Rust from https://rustup.rs (1.95 or newer), then:

```sh
git clone https://github.com/PlotTwistDev/dirmap
cd dirmap
cargo run --release -- /path/to/scan        # GUI
cargo run --release -- --cli --top 20 ~/    # text report, no window
```

The binary ends up in `target/release/`. The same commands work on Windows, Linux and macOS.

Flags: `--all-fs` descends into other mounted filesystems, `--top N` sets the CLI row count,
`--save-snapshot FILE` (with `--cli`) saves the scan for comparing later, `--compare FILE` opens the GUI
comparing against a saved snapshot.

The GUI uses the GPU (wgpu) and falls back to OpenGL automatically. On macOS, scanning `/` covers the
system and data volumes together (they are one disk in Finder) without counting anything twice.
