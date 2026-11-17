# dirmap 2.0: from disk analyser to file manager

Design document for the dirmap 2.0 brief. It covers the brief's deliverables A to E:

- **A.** An audit of the code as it stands (main at `160216c`, 17 Nov 2026).
- **B.** A comparison with other file managers.
- **C.** The proposed interface.
- **D.** The architecture.
- **E.** The phased plan.

The last section records what Phase 1 delivered.

This file sits outside `docs/src` on purpose. The manual describes what dirmap does today; this
document describes what it is going to do.

---

## A. Audit of the current application

### A.1 How the code is laid out

About 16,300 lines of Rust in 28 files:

| Area | Files | Lines | What it does |
|---|---|---:|---|
| App shell | `app.rs` | 2,840 | One `App` struct (~80 fields): toolbar, panels, side tabs, scan jobs, sunburst, status bar, `eframe::App::ui` |
| Scan-backed file manager | `app/browser.rs` | 1,860 | Tabs, List/Icons/Columns/Tree layouts, multi-select, rename, copy/cut/paste, drag and drop. **Every view reads the scan tree** |
| Menus and file operations | `app/menus.rs`, `actions.rs` | 990 + 540 | Right-click menus; background `run_op` jobs; confirmations; `apply_updates` patches the scan tree after an operation |
| Custom filter, sequences | `app/custom.rs`, `pattern.rs`, `sequences.rs`, `subset.rs` | 780 + 660 + 320 + 480 | Filter language, background filtering, "matches only" subset trees |
| Scanner | `scan.rs` | 490 | Parallel rayon walk with a live tree for progress, then `freeze` into a size-sorted `Node` tree |
| Treemap | `treemap.rs`, `mapview.rs`, `camera.rs`, `tilegpu.rs` | 670 + 1,890 + 630 + 600 | Squarified layout, camera, tiles built in the background, GPU cushion shading (wgpu or glow) |
| Other analysis | `sunburst.rs`, `dupes.rs`, `snapshot.rs`, `categories.rs` | small | Sunburst, duplicate finder, snapshot save and diff, file categories |
| Platform | `drives.rs`, `elevate.rs`, `icons.rs` | small | Drive list, Windows elevation, Explorer icons on Windows and drawn icons elsewhere |
| Help | `help/*`, `docs/src` | 1,950 + 57 pages | Built-in Help Centre. Its tests keep the manual in step with the code |

### A.2 What is fully implemented

The audit read the code rather than trusting the README. All of these work today:

- **Scanning.** Uses rayon, never follows symlinks or junctions, and stays on one file system by
  default. Progress shows live through `LiveDir`. A scan can be cancelled, and settings such as
  exclusions, minimum size and maximum age apply while scanning. Unreadable items are counted. The
  scan path has no shared lock (`Progress` uses atomics and a `try_lock` label).
  - Baseline on this container: 200,000 files in 2,041 folders take 0.08 s warm and 0.3–0.5 s
    cold (`dirmap --cli`).
  - Rich's figure is 1,338,012 files and 518.8 GB in about 47 s on Windows.
- **Directory tree.** Virtualised, with size bars, file counts, dates and sortable columns.
- **GPU treemap.**
  - Cushion shading, pointer light and bevel.
  - A single camera handles cursor-anchored zoom, pan, history and animated drill-in and drill-out.
  - Tiles are built in the background and only for the area in view.
  - Falls back from wgpu to OpenGL and then to the CPU.
- **Sunburst.** Select, zoom and go up.
- **Side tabs.** File types, Sizes (histogram), Changes (snapshot diff), Duplicates and Sequences.
- **Filters.**
  - Categories, size, and the Custom pattern language (wildcards, `#` digits, `{frame:…}` ranges,
    negation).
  - Two modes: *Highlight* (keeps the layout) and *Show matches only* (subset tree with a
    path map back to the full tree).
- **The scan-backed file manager panel** (PR #10 and #11), above the map.
  - Tabs, back and forward per tab, breadcrumbs, and List, Icons, Columns and Tree layouts.
  - Natural sort, Ctrl and Shift multi-selection, type-to-select and keyboard movement.
  - Rename (F2), New Folder, copy, cut and paste, and drag and drop (Alt copies).
  - It shares the folder and selection with the map.
- **Right-click actions.**
  - Open, Copy Path, Reveal, Terminal, Refresh, Properties (Windows shell dialog) and SHA-256.
  - Add to exclusion filter, Delete, Recycle or Trash, Remove Empty, Empty Folder and Move To.
  - Every operation runs on a background thread. Afterwards only the affected paths are re-read
    and patched into the tree (`apply_updates`), so there is no rescan.
- **Settings** (persisted with eframe storage), **CLI report and snapshot**, and **Windows
  elevation prompt**.
- **Help Centre and manual.** F1 is context-sensitive, and 21 tests enforce that the docs match
  the code.

### A.3 Partially implemented

| Feature | State |
|---|---|
| File manager | Complete as a UI, but **it can only show what was scanned**. With no scan, the window shows the drive picker and nothing else. You cannot browse a folder without scanning it first, and you cannot see a file created since the scan. |
| Navigation history | The file manager panel has back and forward per tab. The treemap has its own camera history, and the sunburst has none. That makes three separate models. |
| Keyboard focus | `browser.focused` decides whether keys go to the file manager or the map. Some shortcuts are global (`tree_shortcuts` acts on the scan selection wherever focus is). |
| Duplicates | Matches by size, then the first 64 KB, then the whole file. It uses `DefaultHasher` (64-bit SipHash), not a byte comparison or a cryptographic hash, and has no bulk "keep one" action. |
| Drives | Listed on a background thread. Network drives are flagged. There are no Quick Access places such as Home or Documents, and no favourites. |
| Layout | Fixed egui panels: right side tabs, top file manager, a filter bar, and the map in the centre. Panels can be resized but not collapsed, maximised or saved. |

### A.4 Missing

- Browsing without a scan.
- A navigation sidebar: places, devices, network and favourites.
- An editable path bar with UNC paths.
- Workspaces.
- An operation queue with progress, rate, pause, conflicts and retry.
- Undo.
- Search.
- Previews and thumbnails.
- Dual pane.
- Batch rename.
- File system watchers.
- Session restore of tabs.
- Archive handling.
- A "This PC" or "Computer" view.

### A.5 UI limitations seen in the screenshot and the code

- A single toolbar mixes scan controls (Drives, path, Browse, Scan) with map controls (◀ ▶,
  Zoom to selection, Zoom out, Whole map). In the file manager panel, the path and back and forward
  are repeated with different meanings: one acts on the map history, the other on the tab history.
- Without a scan, the whole window is the drive picker. Everything is built around scanning first.
- Every panel is always visible, with no way to give the map or the list the whole window.

### A.6 Reusable pieces

| Piece | Reused for |
|---|---|
| `actions.rs`: `open`, `reveal`, `terminal`, `trash`, `delete`, `copy_into`, `move_into`, `rename`, `new_folder`, `free_name` | All file operations in every workspace. They take paths on disk, not scan indices. |
| `menus.rs`: `run_op` / `OpJob` / `apply_updates` | A first version of the operation queue. Results already patch the scan tree. |
| `browser.rs`: `natural_cmp`, `kind_of`, `SortKey`, `keep_in_view`, the tab struct (paths on disk, not indices) | Sorting, kind labels, scrolling, tab history |
| `drives.rs` (already threaded) | Devices group of the sidebar, "This PC" view |
| `icons.rs` (Explorer icons on Windows, drawn icons elsewhere) | Every file list |
| `App::tree_path_of(disk)` and `full_path(tree)` | The bridge between paths on disk and scan entries, for linked selection and folder sizes |

### A.7 Technical risks found

1. **Data-loss bug in `actions::rename` (fixed in Phase 1).** It treated a case-only change as
   "the same file" and skipped the "already exists" check. On a case-sensitive file system (Linux,
   case-sensitive APFS), renaming `readme` to `README` while a separate `README` existed silently
   replaced it. The check now asks whether both names really are the same file.
2. **`move_into` and `copy_into` check, then act.** Between the existence check and `fs::rename`,
   another process can create the destination. On Unix, `rename` then overwrites it. A cross-volume
   move deletes the source without verifying the copy.
   - **Phase 2 fix:** check and act in one step where the OS allows it (`renameat2(RENAME_NOREPLACE)`
     on Linux, `MoveFileExW` without `REPLACE_EXISTING` on Windows, `renamex_np(RENAME_EXCL)` on
     macOS).
   - Also: verify the size and timestamp of a cross-volume copy before deleting the source.
3. **The `App` god-object.** About 80 fields, and each new panel adds more. The Phase 1 file
   manager lives in its own module with its own state struct. It touches `App` only through a
   handful of fields.
4. **Scan coupling.** The existing browser addresses items as `Vec<u32>` index paths into the
   scan tree. Those indices change on every rescan and every `apply_updates`. The new browser uses
   paths on disk as identity, and translates with `tree_path_of` only when it needs scan data.
5. **Blocking calls on network paths.**
   - `GetDiskFreeSpaceExW` and `statvfs` on a dead share can block for tens of seconds. They
     already run on the drives thread.
   - New code must never call `metadata`, `exists` or `canonicalize` on a user path from the UI
     thread. Phase 1 follows that rule: every listing, existence check and drive query runs on a
     worker.
6. **Global shortcuts.** `tree_shortcuts` and Ctrl+F act on the scan whatever is focused. In the
   File Manager workspace they are turned off, so Delete or Ctrl+C can never act on a hidden
   treemap selection.
7. **No byte comparison before deleting duplicates.** See A.3. This needs fixing before any bulk
   duplicate removal ships.

### A.8 Tests that exist

`cargo test --release` runs 88 tests (86 pass and 2 are ignored by design). It runs in CI on
Windows, Linux and macOS (`.github/workflows/build.yml`).

| Area | Tests |
|---|---|
| Help and manual consistency | 21 |
| Custom filter pattern language | 12 |
| Camera | 11 |
| Map view | 8 |
| Subset trees | 8 |
| Actions | 5 |
| Sequences | 5 |
| Treemap | 4 |
| Browser sorting and scrolling | 3 |
| Others | 1–2 each |

There are no UI-level tests. GUI checks are manual, under Xvfb (see the project notes).

---

## B. Competitive analysis

### B.1 What each one does best, and the common complaints

| Product | Strengths worth learning from | Common complaints |
|---|---|---|
| **Windows File Explorer** | Familiar. Shell integration (context menus, previews, OneDrive). Home/Gallery. Tabs since Win 11 22H2 | Slow to open and slow in large folders (Microsoft said in 2026 that it is working on this). Hangs on disconnected network drives. Weak search. No folder sizes. No dual pane |
| **macOS Finder** | Column view, Quick Look (space bar), the sidebar, tags, previews in the inspector | Can't show folder sizes in list view without "Calculate all sizes", which is slow. Column widths aren't remembered. No cut and paste of files. Weak handling of network shares |
| **KDE Dolphin** | Split view (F3), embedded terminal (F4), editable breadcrumbs, a filter bar, the Places panel, tabs, services menus | Many options and panels can feel busy. Earlier breadcrumb (URL navigator) usability problems that KDE has since worked on |
| **GNOME Files** | Simplicity, good defaults, typing starts a search | Split view (F3) was removed and has been asked for repeatedly. Few power features. Type-ahead search replaced type-to-select |
| **Directory Opus** | Flexible layouts and toolbars, dual pane, a strong batch rename, scripting, folder formats per path, a powerful filter and selection language | Price, Windows only, a steep learning curve |
| **Total Commander** | Two panels and keyboard-first copying between them (F5/F6), a queue for operations, plugins | Dated look, Windows only, cryptic for newcomers |
| **Double Commander** | A Total Commander-style interface on every OS | Rough edges, uneven per-platform integration |
| **XYplorer** | Tabs, fast search with a rich query syntax, scripting, portable | Windows only, the UI is dense |
| **OneCommander** | Modern look, Miller columns by default, a mix of dual pane and columns | Windows only, uneven performance in very large folders |
| **WizTree** | Fast scanning (reads the NTFS MFT directly), a treemap | Analysis only, NTFS only for the fast path |
| **WinDirStat / QDirStat** | Cushion treemap with an extension list, cleanup actions | Slow scans (WinDirStat before 2.0), no browsing |
| **Filelight** | A sunburst navigator | Analysis only, few file actions |

Sources:

- [Windows Latest: File Explorer getting faster in 2026](https://www.windowslatest.com/2026/03/30/windows-11-file-explorer-is-finally-getting-faster-in-2026-but-its-been-slower-than-windows-10-for-years)
- [Microsoft Tech Community: Explorer slow with network drives](https://techcommunity.microsoft.com/discussions/windows11/file-explorer-in-windows-11-opens-slowly-especially-on-computers-with-network-dr/4528805)
- [Fixing Explorer lag in large folders](https://allthings.how/fix-file-explorer-lag-when-browsing-large-folders-on-windows-11)
- [MacRumors: Finder column widths](https://forums.macrumors.com/threads/finder-column-view-column-widths.2439561/page-2)
- [Finder alternatives 2026 (ForkLift, Path Finder, Commander One, QSpace)](https://macos-tahoe.com/blog/best-finder-alternatives-mac-file-managers-2026/)
- [GNOME Discourse: bring back split view](https://discourse.gnome.org/t/feature-request-reintroduce-dual-pane-split-view-mode-in-nautilus/28364)
- [KDE: Dolphin URL navigator issues](https://pointieststick.com/2020/11/15/were-working-on-dolphins-url-navigator-teething-issues)
- [zabkat: comparison of Windows file managers](https://www.zabkat.com/declone/best-windows-file-manager.htm)

### B.2 Feature comparison

Legend: ● has it, ◐ partly, ○ missing. The **dirmap now** column reflects main before Phase 1.

| Capability | Explorer | Finder | Dolphin | Opus / TC | WizTree / WinDirStat | **dirmap now** | **dirmap 2.0 target** |
|---|---|---|---|---|---|---|---|
| Browse without a scan | ● | ● | ● | ● | ○ | ○ | ● (Phase 1) |
| Sidebar: places and drives | ● | ● | ● | ● | ○ | ○ | ● (Phase 1) |
| Tabs | ● | ● | ● | ● | ○ | ◐ (scan only) | ● (Phase 1) |
| Editable path and UNC | ● | ◐ | ● | ● | ◐ | ◐ | ● (Phase 1) |
| Folder sizes in the list | ○ | ◐ slow | ◐ | ◐ on demand | ● | ● | ● from the scan cache, on demand |
| Treemap | ○ | ○ | ○ | ○ | ● | ● | ● linked to the browser |
| Dual pane | ○ | ○ | ● | ● | ○ | ○ | ● (Phase 4) |
| Operation queue and conflicts | ◐ | ◐ | ◐ | ● | ○ | ◐ (one at a time) | ● (Phase 2) |
| Undo | ● | ● | ● | ● | ○ | ○ | ◐ safe cases (Phase 2) |
| Batch rename | ○ | ◐ | ● | ● | ○ | ○ | ● (Phase 4) |
| Search with a rich syntax | ◐ | ● | ● | ● | ◐ | ● filter language (scan only) | ● on live listings and the cache (Phase 4) |
| Image-sequence awareness | ○ | ○ | ○ | ◐ | ○ | ● | ● in the browser too (Phase 5) |
| Snapshots and change tracking | ○ | ○ | ○ | ○ | ○ | ● | ● |
| Duplicates | ○ | ○ | ○ | ● | ◐ | ● | ● with byte verification |
| Previews | ● | ● | ● | ● | ○ | ◐ (details pane) | ● (Phase 4) |

### B.3 What to adopt, where to do better, and what to leave out

**Essential.** Without these, dirmap can't be someone's main file manager:

- browsing without a scan, with the sidebar, path bar, tabs, Details view, keyboard and selection;
- the core operations (copy, move, rename, delete to trash, new folder) with an honest queue and
  conflict handling;
- previews, and search inside a folder.

**Where dirmap can be better.** This is what no single competitor combines:

- **Sizes everywhere.** A folder's size shows in the ordinary browser as soon as any scan has
  covered it, with no "calculate all sizes" step.
- **One step from browsing to analysis.** "Analyse this folder" keeps the place and the selection,
  and the treemap and browser stay linked.
- **Never freezes on network storage.** Listings stream in on workers and can always be
  cancelled. This answers the most common complaint about Explorer and Finder.
- **Sequence-aware operations and filters** for VFX, render and ML-dataset folders.
- **Snapshots and change tracking** as an everyday tool ("what grew this week").

**Not worth copying now:**

- scripting engines (Opus, XYplorer);
- plugin ecosystems (TC);
- FTP or cloud panes (later, behind the same listing interface);
- tags;
- full shell-extension hosting of Windows context menus. Opening the native menu can come later
  through `IContextMenu` on Windows.

---

## C. Proposed interface

Three workspaces, all equal and persistent, switched from the left of the toolbar or with
Ctrl/⌘+1, 2, 3. The last one used is restored at start. Every workspace shares one **current
location** and **selection** (see D.3), so switching never loses your place.

### C.1 File Manager (default)

```
┌───────────────────────────────────────────────────────────────────────────────────────────┐
│ [Files|Analysis]  ◀ ▶ ⏶ ⟳   🖴 C: › Users › rich › Projects           [🔍 Filter…] [⋯] ⚙ ❓ │  toolbar
│ [Projects ✖] [Downloads ✖] [➕]                                                            │  tabs
├──────────────┬────────────────────────────────────────────────────────────────────────────┤
│ ▾ Quick access│ Name ▲               Size      Type          Date modified                 │
│   🏠 Home      │ 📁 assets           2.1 GB*   Folder        12 Nov 2026 14:02             │
│   🖥 Desktop   │ 📁 src                 —      Folder        17 Nov 2026 09:15             │
│   📄 Documents │ 📄 Cargo.toml       1.2 KB    TOML file     17 Nov 2026 09:15             │
│   ⬇ Downloads │ …                                                                          │
│ ▾ Pinned      │                                                                            │
│   ⭐ renders   │                                                                            │
│ ▾ Drives      │                                                                            │
│   🖴 C: ▓▓▓░ 62%│                                                                           │
│   🖧 Z: (net)  │                                                                            │
│ ▾ Recent      │                                                                            │
├──────────────┴────────────────────────────────────────────────────────────────────────────┤
│ 214 items · 3 selected (4.2 MB)                 Scanning D:\ 41% [Show]                     │  status
└───────────────────────────────────────────────────────────────────────────────────────────┘
```

- `*` marks a folder size that comes from an earlier scan. A tooltip says when, and "—" means not
  scanned.
- The bottom status bar shows any running scan or operation, so neither ever needs its own window.
- Phase 2 and later add:
  - an optional right inspector (preview and properties, Phase 4);
  - an optional bottom operation queue (Phase 2);
  - the ⋯ overflow menu (view mode, hidden files, sort, Analyse this folder).

### C.2 Explorer + Analysis (Phase 3)

```
┌ toolbar + tabs (same as File Manager) ──────────────────────────────────────────────┐
├──────────┬────────────────────────────────────────────┬─────────────────────────────┤
│ sidebar  │ file browser (Details / tree)              │ File types │ Sizes │ …      │
│(collapsible)                                          │ (existing side tabs)        │
│          ├────────────────────────────────────────────┤                             │
│          │ treemap / sunburst (existing engine)       │                             │
└──────────┴────────────────────────────────────────────┴─────────────────────────────┘
```

- The treemap follows the browser's location by default. Its ⋯ menu offers Follow browser,
  Follow selection, Independent and Pin to this folder (brief §12).
- Selecting in either one highlights the item in the other. The map does not zoom unless asked
  ("Zoom to selection" or the follow mode).
- A folder that hasn't been analysed shows "Analyse this folder" in the map area, rather than an
  empty map.

### C.3 Storage Analysis (today's layout, unchanged)

The current window stays as it is: scan toolbar, file manager or tree panel, filter bar, map,
side tabs and status. The only addition is the workspace switcher at the left of its toolbar.
Later polish (Phase 3) makes the side panel collapsible and the map maximisable (double-click the
divider or press F11).

### C.4 Dual pane (Phase 4)

F3 (Dolphin's key) splits the browser area into two panes. Each has its own tabs, history, view,
sort, selection and filter. The **active pane** has an accent bar and receives the keyboard.

- F5 copies and F6 moves the selection to the other pane, after a confirmation that lists the
  source and destination.
- The treemap (in Explorer + Analysis) follows the active pane unless it's pinned.

### C.5 Preview inspector (Phase 4)

A right panel, toggled with Alt+P. Its parts:

- **Header:** icon or thumbnail, name, kind and size, with the share of its folder and drive when
  scanned.
- **Body:** a preview from a modular provider (image, text or code, PDF first page, audio
  waveform, archive listing, sequence filmstrip), loaded on a worker with a size cap and cached.
- **Footer:** dates, permissions or attributes, owner, link target, and actions.

### C.6 Operation queue (Phase 2)

A bottom drawer that opens itself when an operation runs for more than one second. It has one row
per operation with:

- the verb and the counts ("Copying 1,204 files (8.2 GB) to D:\Backup");
- a progress bar, the current file, the transfer rate and the time left;
- Pause, Cancel and Show errors;
- conflicts as an inline card: Replace, Skip, Keep both, Rename and Compare, with "do this for the
  rest". It shows the metadata for both files side by side.

When the queue finishes, the drawer closes after a few seconds, and a summary stays in the status
bar.

### C.7 Interaction rules shared by every view

- **Keyboard focus.** Shortcuts go to the panel that was clicked last, and that panel shows an
  accent outline. Ctrl+A in the browser never selects tree rows. Delete acts only on the focused
  panel's selection, always confirms permanent deletes, and never acts on something not visible.
- **Ctrl and ⌘.** egui's `Modifiers::COMMAND` maps to ⌘ on macOS and Ctrl elsewhere. Shortcuts
  are written through `format_shortcut`, so menus show the right symbol.
- **Destructive commands** sit at the bottom of menus, set apart and labelled with what they do
  ("Delete 3 items permanently…").

---

## D. Architecture

### D.1 Overview

```
            ┌───────────────────── UI (egui) ─────────────────────────┐
            │  workspace switcher · sidebar · file browser · tree ·   │
            │  treemap/sunburst · side tabs · queue · inspector       │
            └───────▲───────────────▲───────────────────▲─────────────┘
                    │ polls          │ reads             │ submits
     ┌──────────────┴──────┐ ┌──────┴────────────┐ ┌────┴──────────────┐
     │ Browsing service     │ │ Shared model      │ │ Operation service │
     │ fsys::listing (live  │ │ Location, History,│ │ actions.rs + queue│
     │ read_dir on workers) │ │ selection, places │ │ (run_op today)    │
     │ fsys::places         │ │ (paths on disk)   │ └────┬──────────────┘
     └──────────────────────┘ └──────▲────────────┘      │ invalidates
                                      │ path ⇄ index      ▼
                              ┌───────┴──────────────────────────────┐
                              │ Analysis service: scan.rs, Node tree, │
                              │ treemap/mapview/tilegpu, dupes, snaps │
                              └──────────────────────────────────────┘
```

### D.2 The browsing service (`src/fsys/`, new in Phase 1)

- **`fsys::listing`** reads one folder on its own worker thread and streams entries back in
  batches.
  - Batches are 512 entries or 40 ms, whichever comes first. A 100,000-file folder starts drawing
    after the first batch, not after the last.
  - Each `Entry` holds the name, folder flag, link flag, hidden flag, size, modified and created
    times, and the read-only flag.
  - The data comes from `DirEntry`. On Windows that costs nothing extra. On Unix it is one `lstat`
    per entry, plus a `stat` for links, to learn whether the target is a folder.
  - Dropping the listing (navigating away) cancels it. A worker stuck in the kernel on a dead
    share just ends later. The UI never waits.
  - The UI shows "Still waiting…" with Cancel after 2 s, so a slow share never looks like a hang.
- **`fsys::places`** finds the OS-specific places, using only cheap environment and config lookups:
  - Windows Known Folders (`SHGetKnownFolderPath`, which follows redirected folders such as
    Downloads);
  - XDG `user-dirs.dirs` on Linux;
  - `~/…` and `/Applications` on macOS.
- **`fsys::history`** is the navigation history (back and forward stacks of `Location`). It is
  shared by the new browser now and meant for the scan browser's tabs and the map in Phase 3.
- **Later phases:**
  - `fsys::watch`: a `notify` crate watcher on open folders (Phase 2);
  - `fsys::meta`: owner, permissions and attributes (Phase 4);
  - `fsys::thumbs`: a bounded LRU cache with a worker pool (Phase 4).

### D.3 Shared model: identity, location and selection

- **Identity is the path on disk** (`PathBuf`) everywhere outside the scan internals.
  - Scan `Vec<u32>` paths are an index cache, translated with `tree_path_of` / `full_path` at the
    boundary. They stay valid until the next rescan or `apply_updates`.
  - Paths are compared case-insensitively on Windows and macOS and exactly on Linux (`same_path`).
- **`Location`** is either `Computer` (the virtual "This PC" or "Computer" root) or `Dir(PathBuf)`.
- **Switching workspaces** hands over `(location, selected paths)`:
  - **Files → Analysis:** if the scan covers the folder, the analysis browser and the map go to it
    and select the item. If not, the analysis view stays put and the File Manager shows "Analyse
    this folder".
  - **Analysis → Files:** the File Manager opens the analysis browser's folder and selects the same
    item.
- **Phase 3 adds an event bus** (`SelectionChanged`, `LocationChanged` and `PathsChanged` in a
  per-frame `Vec<AppEvent>` that each panel reads). That avoids the per-panel `seen_*` comparisons
  in `browser_sync`.

### D.4 The analysis service

- The scan, `Node` tree, treemap, sunburst, duplicates, snapshots and sequences are **unchanged**.
- Phase 1 adds one read-only link: `App::scanned_size(path)` looks a folder up in the current scan
  to show its size in the browser.
- Phase 2 sends every completed operation through the existing `apply_updates`. A rename, move or
  delete then updates the scan tree in place, and the scan never shows deleted files as current.
- A file system watcher event under the scanned root marks that subtree **stale** (greyed size
  with a refresh hint) rather than rescanning.

### D.5 Concurrency

| Work | Where it runs | Priority and cancellation |
|---|---|---|
| Folder listing | One short-lived thread per listing | Dropped on navigation. Never queued behind a scan |
| Scan | Its own rayon pool (as now) | `Progress::cancel` |
| Drive and capacity query | One thread per refresh | Results replace the list. A slow drive delays only the list |
| File operations | Phase 1: `run_op` thread. Phase 2: queue worker(s) | Per-job cancel flag, checked per file and per 1 MB chunk |
| Hashing, thumbnails | Rayon pool / bounded workers | Lower priority than listings |

Browsing threads are separate from the scanner's rayon pool. A running scan uses the CPU cores
but never holds a listing up behind its queue.

### D.6 How the new browser coexists with the scan tree

- **The File Manager workspace** uses `app/files.rs`, which runs on live listings. It works with
  no scan at all.
- **The Storage Analysis workspace** keeps `app/browser.rs`, the scan-backed file manager, the
  folder tree and the map exactly as they are.
- **Shared pieces:**
  - `actions.rs` for every file command;
  - `menus::run_op` for background operations;
  - `natural_cmp` for name order;
  - `icons.rs`, `drives.rs` and the help topics.
- **What is not shared yet:**
  - The tab sets: the scan browser keeps its own. Phase 3 moves both onto `fsys::history` and
    one tab list, when the Explorer + Analysis workspace needs a single browser that can show
    either source.
  - The Phase 1 file browser deliberately has **no** delete, move or rename of its own. Those
    arrive in Phase 2 through the shared operation service, so the logic exists once.

### D.7 Docking and layout

- **`egui_dock` 0.21** supports egui 0.36, which is what dirmap uses. It provides tabs, splits,
  drag-to-dock and serialisable layouts.
- **`egui_tiles` 0.18** needs egui 0.37.
- **Recommendation:** fixed egui panels for Phase 1, then `egui_dock` from Phase 3, once the
  Explorer + Analysis layout needs user-arranged regions. Adding it means checking that the
  treemap's paint callback still works inside a dock tab. The paint callback is a normal
  `egui::PaintCallback`, so it should.

### D.8 Dependencies

**Phase 1 adds no crates.** One feature is added to `windows-sys`: `Win32_System_Com`, for
`CoTaskMemFree` after `SHGetKnownFolderPath`.

Candidates for later phases:

| Crate | Use | Status |
|---|---|---|
| `trash` 5.3 | Linux and macOS trash with restore info, for undo | Actively maintained |
| `notify` 8.2 | File system watchers | Ubiquitous |
| `egui_dock` 0.21 | Docking | Matches egui 0.36 |

The existing `actions::trash` stays the Windows path.

---

## E. Implementation plan

Each phase is one pull request made of small commits that build and test on their own. Rich
reviews between phases.

### Phase 1: file manager foundation (this PR)

| Step | Change | Test |
|---|---|---|
| 1 | `fsys::listing`: streaming, cancellable folder reads | Unit tests: batching, hidden files, links, errors, a 20,000-entry folder |
| 2 | `fsys::places`: Home, Desktop, Documents, Downloads, Pictures, Music, Videos, Applications; XDG parser | Unit tests for the `user-dirs.dirs` parser and fallbacks |
| 3 | `fsys::history`: back, forward and up over `Location` | Unit tests |
| 4 | `app/files.rs`: the File Manager workspace. Sidebar (Quick access, Pinned, Drives with capacity bars and network flags, Recent), toolbar (◀ ▶ ⏶ ⟳), breadcrumbs and editable path (Ctrl+L, UNC), tabs (new, close, middle-click, Ctrl+T/W, Ctrl+Tab), Details view (Name, Size, Type, Date modified; sortable; folders first; natural order; virtualised), selection (click, Ctrl, Shift, Ctrl+A, arrows, Home/End, Page keys, type-to-select), open (double-click or Enter), hidden-file toggle, filter-this-folder box, "This PC"/"Computer" view, context menu (Open, Open in New Tab, Pin, Copy Path, Show in system file manager, Open in Terminal, Analyse this folder) | GUI check under Xvfb, screenshots |
| 5 | Workspaces: switcher; Ctrl+1 and Ctrl+3; the last one persisted; File Manager by default (Storage Analysis when started with a path or `--compare`); location and selection handed over on switch; scan shortcuts only in Storage Analysis | GUI check |
| 6 | Folder sizes from the current scan in the browser | GUI check |
| 7 | Persistence: tabs, pinned, recent, sort, hidden files, workspace | Restart under Xvfb |
| 8 | Docs: a manual page, keyboard and mouse reference, settings reference | `cargo test` doc checks |
| 9 | Safety fix: `rename` case-only overwrite | Unit test |

**Acceptance:** dirmap opens in the File Manager. You can browse drives, mounts and home folders
with no scan. Storage Analysis behaves exactly as before. Scan speed is unchanged (benchmark
below).

### Phase 2: everyday file operations

- An operation queue (`fsys::ops`) with streamed copy: chunked, cancellable and pausable, with
  progress per byte and the timestamps preserved.
- Check-and-act in one step where the OS allows it.
- A conflict dialog.
- Verify cross-volume moves before deleting the source.
- Rename (inline F2), new folder or file, copy, cut and paste, move, duplicate, delete to trash and
  permanent delete, and drag and drop between tabs and the sidebar.
- `notify` watchers.
- An undo stack for rename, move, create and trash. Undoing a trash uses `trash` crate restore
  where available, and otherwise says it can't.
- Every operation feeds `apply_updates`, so the analysis stays correct.
- **Risks:** platform trash semantics, network latency during copy, and keeping a dropped
  connection from stopping the whole queue.

### Phase 3: Explorer + Analysis

- The third workspace.
- `egui_dock`.
- One tab model and event bus.
- Linked selection in both directions without unwanted zoom.
- Treemap follow and pin modes.
- On-demand folder scans.
- Stale marking from watcher events.
- Collapsible and maximisable panels.

### Phase 4: advanced file management

- Dual pane, an Icons and thumbnail grid, and Columns on live listings (reusing browser.rs
  drawing).
- The preview inspector.
- Batch rename.
- Search, using the existing pattern language on live recursive walks and on the scan cache.
- Saved searches.
- Saved layouts and session restore options.

### Phase 5: storage intelligence

- Sequence grouping in the browser.
- A byte-verified duplicate cleanup with "keep newest or oldest".
- Snapshot timelines.
- Cleanup suggestions (caches, old renders, model weights).
- Optional indexing.
- Fast NTFS MFT scanning (investigated).

### Phase 6: production quality

- Profiling at 1M, 3.5M and 10M entries.
- An accessibility pass (AccessKit labels).
- Crash-safe operation journal.
- Installers.
- Onboarding.
- UI regression screenshots in CI under Xvfb.

### Performance budgets (to hold from Phase 1)

| Metric | Target |
|---|---|
| Scan throughput | No regression. Phase 1 doesn't touch `scan.rs`. Measured below |
| First rows of a local 100,000-entry folder | < 100 ms warm |
| Frame time while a listing streams | < 16 ms (rows are virtualised; sorted once per batch) |
| Navigating on a dead network share | The UI never blocks. "Still waiting" with Cancel after 2 s |

---

## Phase 1: what was delivered

See the pull request description for the full list, screenshots and benchmark results. The
manual page [The File Manager](../src/views/file-manager.md) describes it for users.
