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
- **Cushion treemap** (van Wijk & van de Wetering, like WinDirStat) colored by file type. Hover for the
  path and size, click to select (the tree jumps to it), double-click to zoom into a folder, right-click to zoom out.
- **Settings** (saved between runs): theme, size units, filters (folders and file patterns to skip or include,
  minimum size, maximum age), tree columns, treemap look with live preview, hidden files, scanner threads.

## Run

```sh
cargo run --release -- /path/to/scan        # GUI
cargo run --release -- --cli --top 20 ~/    # text report, no window
```

Flags: `--all-fs` descends into other mounted filesystems, `--top N` sets the CLI row count.

On Linux the GUI needs X11 or Wayland plus `libxkbcommon-x11`. It uses the GPU (wgpu) and falls back
to OpenGL automatically.
