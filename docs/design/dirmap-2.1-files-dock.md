# dirmap 2.1: Files redesign and the analysis dock

Status: proposal and plan, 17 November 2026. It follows on from `dirmap-2.0.md`, which covers
the Files workspace (Phase 1). This brief has two parts: make Files look finished, and bring
the treemap into Files as a dock under the list.

## 1. Audit

### What the screen does today

| Area | What's there | Problems |
|---|---|---|
| Toolbar | Workspace buttons, ◀ ▶ ⏶ ⟳, breadcrumbs, filter, Hidden, Help, ⚙ | Every control has the same weight. Navigation, location and utilities run together with no grouping. The breadcrumb box is the same grey as everything else. There are no analysis actions, so Analyse lives only in menus. |
| Tabs | egui selectable buttons with a ✖ after each | They read as buttons, not tabs. The active tab isn't joined to the page under it, there are no separators, and the close button always shows. No reordering, and no overflow handling (tabs run off the edge). |
| Sidebar | Collapsing headers, rows, drive bars | Default egui headers (large ▶ triangles). The current place is a flat fill. Section titles look like rows. The drive bars are fine. |
| List | Fixed 22 px rows, four columns, stripes | Column widths are fixed and can't be resized. The header has no dividers. The hover fill is almost the same as the stripes. Selection uses the egui accent at full strength. There's no density option. Empty folders say "This folder is empty" in small weak text. |
| Surfaces | One panel colour for everything | The sidebar, list, toolbar and status bar all share one grey, so the panels have no edges except 1 px lines. |
| Below the list | Unused space | In a short folder, most of the window is empty. |
| Status bar | Scan progress, notices, path under the pointer | A running scan takes the whole bar. Other background jobs (duplicates, file commands) aren't shown in Files. |
| Analysis | A separate workspace | Seeing where space goes means leaving the file manager. |

### Code structure that matters

- `src/app/files.rs` holds the Files workspace: `Files` state, `FTab` per tab, `files_toolbar`,
  `sidebar`, `files_tabs_row`, `details_view` and `files_footer`. Folders are listed by
  `fsys::listing` on worker threads.
- `src/app.rs::treemap_panel(ui)` draws the GPU treemap into whatever rectangle `ui` gives it.
  It reads `self.zoom` (the folder shown, as a path of child indices into the scan tree) and
  `self.selected`. `MapView::sync` already animates zooming into a child folder and back out
  to a parent, and re-lays out when the size changes (debounced 150 ms). It keeps its tiles,
  so a resize doesn't recreate GPU resources.
- `sunburst_panel(ui)` works the same way.
- There is one scan tree, `self.tree: Arc<Node>`, rooted at `self.scanned_path`.
  - `tree_path_of(path)` turns a path on disk into a tree path, and `full_path(tree path)`
    turns it back.
  - `apply_updates` swaps fresh subtrees into the tree, keeping the selection and the zoom.
- Scans run on worker threads (`Job`), with `Progress` counters and a live tree (`LiveDir`).

## 2. Visual design proposal

The aim is refined and quiet: a serious utility, not a dashboard. The model is Windows 11
Explorer and Finder for surfaces and selection, Dolphin and Directory Opus for density.

**Surfaces (three levels).**
- Window chrome (toolbar, tabs strip, status bar): the darkest in dark mode, the lightest grey
  in light mode.
- Sidebar: one step lighter (dark mode) or darker (light mode).
- Content (list, dock): the "paper", with the most contrast for text.

Panels are separated by colour first and hairlines second.

**Accent.** One restrained blue, used only for:
- selection;
- focus rings;
- the current place in the sidebar;
- the active tab's top edge;
- progress.

Selection is a 22% tint of the accent with normal-weight text. When the list doesn't have the
keyboard, it's a neutral tint.

**Shape.** Rows and buttons have 5 px corners, tabs 6 px on top. There are no shadows apart
from popups, and no gradients.

**Typography.**
- 13 px for the list and sidebar.
- 12 px for headers, status and secondary columns.
- Section titles in 11 px small caps, letter-spaced and weak.
- Sizes are right-aligned with tabular figures, and dates are in a weak colour.

**Toolbar groups**, left to right:
1. Workspace switch.
2. Navigation (◀ ▶ ⏶ ⟳) in a segmented group.
3. Location: breadcrumbs in a rounded field with a folder icon, which you can click to type.
4. Analysis: Analyse (context-sensitive: Analyse Folder, Refresh Analysis, or Cancel while
   scanning) and the Dock toggle.
5. Utility, on the right: filter, Hidden, Help, Settings.

Every button has a tooltip with its shortcut. File actions (copy, move, rename, delete) join
the toolbar in the file-operations stage, when they work. No placeholders.

**Tabs.**
- The active tab shares the content colour and has an accent line on top.
- Inactive tabs blend with the chrome and lighten on hover.
- Each tab has a folder icon and a name truncated at 22 characters.
- ✖ shows on the active and hovered tabs.
- Drag a tab to reorder it.
- Tabs shrink to fit, and scroll when there are too many.

**List.**
- Comfortable rows are 26 px and compact rows are 22 px, set in Settings and on the toolbar's
  View menu.
- The header has thin dividers you can drag to resize columns. Widths are remembered.
- The sort arrow sits next to the title.
- Hover is a faint neutral fill, distinct from the stripes.
- Empty folders show a large faint folder icon with one line of text.

**Light and dark.** Both palettes are defined explicitly, with contrast of at least 4.5:1 for
body text and 3:1 for weak text. The System setting follows the OS.

## 3. Layout with the dock

```
┌ Files | Storage Analysis ┃ ◀ ▶ ⏶ ⟳ ┃ 🗀 C: › Users › rise ┃ ⚡Analyse ▾  ▦ Dock ┃ 🔍 filter  Hidden ? ⚙ ┐
├──────────────┬ [🗀 rise ✖] [🗀 Projects] [+] ───────────────────────────────────────────────────┤
│ QUICK ACCESS │ Name                       │    Size │ Type          │ Date modified            │
│  Home        │ 🗀 Documents               │ 12.1 GB │ Folder        │ 2026-11-17 09:12         │
│  Desktop     │ 🗀 Downloads               │  4.0 GB │ Folder        │ …                        │
│ DRIVES       │  …                                                                           │
│  C: ▬▬▬▬▭    ├══════════════════════════════ drag ══════════════════════════════════════════┤
│  D: ▬▬▭▭▭    │ ▦ Treemap ◔ Sunburst ┃ Follow folder ▾ ┃ Colour: Type ▾ ┃ ⌖ Zoom to sel ⟲ Whole ┃ ⤢ ⛶ ✖ │
│ RECENT       │ ● Complete · 18.4 GB · 41,203 files · 3,118 folders · largest: Documents 12.1 GB    │
│              │ ┌───────────────────────────── GPU treemap of C:\Users\rise ──────────────────────┐ │
│              │ │                                                                                │ │
│              │ └────────────────────────────────────────────────────────────────────────────────┘ │
│              │ ■ mp4 ■ exr ■ zip ■ psd ■ …   (click to highlight)                                  │
├──────────────┴──────────────────────────────────────────────────────────────────────────────┤
│ 41 items · 2 selected (1.2 GB)                                      ◌ Scanning D:\ 31% ▾      │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
```

- The dock is a bottom panel inside the Files content area, under the tabs. It never overlays
  the list.
- The splitter enforces a 120 px minimum for both the list and the dock.
- ⛶ maximises the dock, to all but 120 px of the list. ⤢ opens the full Storage Analysis
  workspace at the same folder.
- Height, visibility, follow mode and map kind are remembered (`FilesPrefs`).
- The list keeps its scroll position, selection and sort when the dock opens or resizes,
  because it only gets a shorter rectangle.

## 4. Reusing the treemap and the scan

There is one analysis model and one renderer. The dock calls the same `treemap_panel` and
`sunburst_panel` as Storage Analysis, with a smaller rectangle. The same `MapView` is used,
because only one workspace is on screen at a time. Tiles are cached by folder and size, and
the existing 150 ms resize debounce means dragging the splitter doesn't rebuild GPU resources
on every frame.

**Resolving the current folder.**
1. `tree_path_of(dir)`: strip `scanned_path` from the folder, then walk the names down the tree.
2. If the folder resolves, set `self.zoom` to that tree path. `MapView::sync` animates parent to
   child and child to parent with its existing drill transitions.
3. If it doesn't resolve, the dock says so ("Not in the current analysis") and offers
   **Analyse This Folder**.

The dock never shows a treemap of an unrelated folder.

**Analyse Current Folder.**
- If the folder is inside the current analysis, it becomes **Refresh Analysis**. That scans
  only this folder, then swaps the result into the existing tree with `apply_updates`, so a
  whole-drive analysis is kept and not thrown away.
- Otherwise it starts a normal scan of the folder.

In both cases you stay in Files. The dock shows progress and the counters (files, folders,
bytes, time, errors, the folder being read). The percentage is shown only for whole drives,
where the used space gives a real total, as today. Browsing carries on, because listings are
independent threads.

**Earlier analyses.** When a new scan replaces the tree, the previous tree is kept in a small
cache: two entries, and only if each is under 2 million files. Opening a folder covered by a
cached analysis switches to it at once, with the time it was made. No rescanning.

**Staleness.** The folder on screen is compared with the analysis: its names, its file sizes,
and newer modification times. If they differ, the dock marks the analysis **Changed since
analysis** and offers Refresh. This check is cheap, needs no extra disk access (the listing
already has the data), and is honest about what's been checked.

## 5. Keeping the treemap in step

| Mode | The treemap shows | On a tab switch |
|---|---|---|
| **Follow folder** (default) | The active tab's folder | Follows the new tab |
| **Pinned** | The folder it was pinned at, until unpinned | Unchanged |
| **Follow selection** | The selected folder, or the current folder with the selected file outlined | Follows the new tab's selection |

**List to map.** The selected items are outlined on the map; up to 500 are drawn, as in
Storage Analysis today. The map only moves if the item is too small to see, and then it frames
it with the existing `frame_path`.

**Map to list.**
- Clicking a tile selects the matching item in the list. If the tile is deeper than the
  current folder, the containing item is selected and the tile itself is outlined, so nothing
  scrolls away under you.
- Double-clicking a folder tile opens that folder in the list (in Follow mode the map drills
  in with it).
- Right-clicking a tile shows the same item menu as the list, through the same `files_action`
  and `actions` code. There are no separate implementations.

The **info strip** shows:
- the status: Not analysed, Scanning, Complete, Changed since analysis, Cancelled or Error;
- the total size, files and folders of the folder shown;
- its largest file and largest subfolder;
- when it was analysed.

The **legend** shows the top file types of the folder shown, in their treemap colours.
Clicking a type highlights it, using the existing `highlight_ext`.

## 6. Stages and acceptance criteria

| Stage | Delivers | Accepted when |
|---|---|---|
| 1. Design system | The palette for both themes, toolbar groups, new tabs (drag to reorder, overflow), the list (density, resizable columns, header, hover/selection, empty state), the sidebar and the status bar | Both themes pass a contrast check; every control works; no frame takes over 2 ms at 100k items; settings persist |
| 2. Dock | The bottom dock with show/hide, splitter, maximise/restore and Open in Storage Analysis; the treemap or sunburst of the current folder; Analyse/Refresh/Cancel with progress; the merge refresh | It opens, closes and resizes with no layout jump and no lost scroll or selection; a scan never blocks browsing; Refresh of a subfolder keeps the drive analysis |
| 3. Follow modes | Follow folder, Pinned and Follow selection; tab awareness; the not-analysed state; the cache of earlier analyses; staleness | Navigating 5 levels into a whole-drive analysis updates the map with animated drills and no rescans; outside the analysis it says so; tabs follow the rules above |
| 4. Unified interaction | Two-way selection; the shared context menu; the info strip; the legend with highlight; the activity indicator (scan, duplicates, file commands) with a popover | Selecting in either view selects in the other; menu actions behave the same from both; the strip only shows facts the data supports |
| 5. Polish | Profiling, the help pages, screenshots, Windows and macOS cross-checks, the scan benchmark | Tests pass; the scan time is unchanged; the manual matches the app |

### Deliberately later

- **Progressive treemap tiles while scanning.** The renderer works from a finished `Node`
  tree. Drawing from the live tree means periodic snapshots of a structure that's still
  changing. The dock shows live counters and the largest folders found so far instead, and
  draws the map the moment the scan finishes. This is worth a stage of its own.
- **File operations in the Files toolbar** (copy, move, rename, delete, undo). These are
  Phase 2 of the 2.0 plan. They'll appear in the toolbar and on tiles once that backend is
  wired into Files.
- **Thumbnail view and thumbnail generation.**
