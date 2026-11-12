# Feature status

This page says plainly what dirmap does today, what works only partly, and what isn't available
yet, so you don't go looking for something that isn't there. It's kept up to date with each
version.

## Available

| Feature | Notes |
|---|---|
| Drive list and folder scanning | Parallel, with live progress and a live folder list. Windows, Linux and macOS. |
| Directory tree | Sortable Size, % of parent, Files and Last change columns; Explorer icons on Windows. |
| Treemap | Cushion shading on the GPU, cursor-anchored smooth zoom, pan with inertia, drill in and out, Back/Forward, zoom to selection, labels, pointer lighting. |
| Sunburst | Six rings, zoom in and out, selection. |
| Colour by type, age, and change since a snapshot | |
| File types list | Highlight, find all files of a type, copy, exclude. |
| Category, size and Custom name filters | Highlight or Show matches only; wildcards, exclusions, AND/OR, frame numbers, padding and ranges; presets and recent patterns. |
| Numbered sequence detection | Frame ranges, steps, missing frames. |
| Size histogram | Click a range to filter. |
| Snapshots and comparison | Save, compare, colour by change; from the command line too. |
| Duplicate finder | Size, then first 64 KB, then whole-file hash. |
| Right-click menu on the folder list | Open, Copy Path, reveal, terminals, Refresh, Properties, SHA-256, exclude, Delete, Recycle Bin/Trash, Remove Empty Folders, Empty Folder, Move to, with keyboard shortcuts. |
| Settings | Appearance, scan exclusions, columns, treemap look and navigation, threads, administrator mode. |
| Run as administrator | Windows: ask, always or never, and restart on request. |
| Command-line report and snapshots | `--cli`, `--top`, `--save-snapshot`, `--compare`, `--all-fs`. |
| Built-in help | This Help Centre, with search, F1 context help and tooltips. |

## Partly available

| Feature | What's missing |
|---|---|
| Right-click menus | Only in the folder list and the File types list. The treemap and sunburst have none (right-click zooms out there). WinDirStat's **Compress**, **Sparsify**, **Optimize VHD**, **Custom Cleanup**, the full **Windows Explorer** context submenu and **Group Unregistered Types** aren't available. |
| Keyboard use | Shortcuts exist for file commands, the Custom filter, help and the treemap, but the folder list can't be moved through with the arrow keys, and there's no keyboard way to move between panels. |
| Duplicate verification | Matches use a 64-bit hash of the whole contents, not a byte-by-byte comparison. There's no "delete all but one" action; copies are deleted one at a time. |
| Sunburst navigation | No Back/Forward history (the treemap has it). |
| Size accuracy | File sizes only. Hard links are counted once per name; compressed, sparse and cloud-only files show their full size. See [How sizes are calculated](../disk-usage/sizes.md). |
| Administrator mode | A restart button on Windows only; on Linux use `sudo`, on macOS Full Disk Access. |

## Not available

These aren't in dirmap yet. Some may come later; none should be relied on.

- A **size on disk** (allocated space) mode, and counting hard-linked files once.
- **Compressing**, **sparsifying** or **compacting virtual disks** from dirmap.
- **Selecting several items** at once, and bulk operations.
- **Exporting** results (CSV, report files) from the window. The command-line report is text only.
- **Automatic rescans** or watching drives for changes.
- **Fast NTFS scanning** by reading the master file table directly.
- **Comparing two different folders** side by side (snapshots compare the same folder over time).
- **Scanning several drives into one map.**
- A **translated** interface; dirmap is in English only.

## Related topics

- [What's new](whats-new.md)
- [How dirmap works](../advanced/architecture.md)
