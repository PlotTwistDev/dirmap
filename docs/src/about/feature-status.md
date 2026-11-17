# Feature status

This page says plainly what dirmap does today, what works only partly, and what isn't available
yet, so you don't go looking for something that isn't there. It's kept up to date with each
version.

## Available

| Feature | Notes |
|---|---|
| Files workspace | Browse without scanning: sidebar (Quick access, Favourites in groups, Drives with space, Network & Remote, Recent), Computer view, two panes side by side or stacked, Details, Icons, Columns or Gallery view per pane with pictures of images (EXR and HDR too), videos and PDFs, each folder remembering its view, sort and grouping, grouping by type, date or size, zip/7z/tar archives browsed like folders with Extract Here and Extract To, file watching, copies and moves with pause, stop and a queue, copy/cut/paste, drag and drop between folders, panes, tabs, path and sidebar (move on the same drive, copy across drives), rename, Recycle Bin/Trash and permanent delete, New folder and files from templates, Quick Look and a preview pane (images including EXR and HDR, text, video frames, PDF first pages, sound playback, archive contents), an address bar with suggestions, Tab completion, environment variables, `>` commands and `@` favourites, Windows mapped drives and SMB connect/disconnect (mounts on macOS and Linux), tabs restored at start, Back/Forward/Up, clickable and typed paths (including UNC), sortable Details list, filter box, hidden files, multi-select, type to select, right-click menu, Analyse This Folder, folder sizes from the last scan, resizable columns, row density, draggable tabs. See [The Files workspace](../views/file-manager.md). |
| Analysis dock | The treemap or sunburst of the current folder under the list, from the existing analysis. It has:<br>- Follow folder, Pinned and Follow selection;<br>- Analyse and Refresh in the background;<br>- the two most recent earlier analyses kept;<br>- a changed-since-analysis check;<br>- two-way selection, the tile menu, folder facts and a type legend.<br>See [The analysis dock](../views/file-manager.md#the-analysis-dock). |
| Drive list and folder scanning | Parallel, with live progress and a live file manager. Windows, Linux and macOS. |
| Directory tree | Sortable Size, % of parent, Files and Last change columns; Explorer icons on Windows. |
| Treemap | Cushion shading on the GPU, cursor-anchored smooth zoom, pan with inertia, drill in and out, Back/Forward, zoom to selection, labels, pointer lighting. |
| Sunburst | Six rings, zoom in and out, selection. |
| File manager | Finder/Explorer-style browsing above the map in List, Icons, Columns and Tree layouts: tabs, sizes and share of folder, sorting, path bar, Back/Forward, keyboard navigation, type to select, multi-select, rename, new folder, copy/cut/paste, drag and drop, filters, right-click menu; moves with the map. |
| Colour by type, age, and change since a snapshot | |
| File types list | Highlight, find all files of a type, copy, exclude. |
| Category, size and Custom name filters | Highlight or Show matches only; wildcards, exclusions, AND/OR, frame numbers, padding and ranges; presets and recent patterns. |
| Numbered sequence detection | Frame ranges, steps, missing frames. |
| Size histogram | Click a range to filter. |
| Snapshots and comparison | Save, compare, colour by change; from the command line too. |
| Duplicate finder | Size, then first 64 KB, then whole-file hash. |
| Right-click menu in the file manager | Open, Copy, Cut, Paste, Rename, New Folder, Copy Path, reveal, terminals, Refresh, Properties, SHA-256, exclude, Delete, Recycle Bin/Trash, Remove Empty Folders, Empty Folder, Move to, with keyboard shortcuts. |
| Settings | Appearance, scan exclusions, columns, treemap look and navigation, threads, administrator mode. |
| Run as administrator | Windows: ask, always or never, and restart on request. |
| Command-line report and snapshots | `--cli`, `--top`, `--save-snapshot`, `--compare`, `--all-fs`. |
| Built-in help | This Help Centre, with search, F1 context help and tooltips. |

## Partly available

| Feature | What's missing |
|---|---|
| Files workspace | Grouping is in the Details view only. Video and PDF pictures and sound need the system's own thumbnailers and players (on Linux: ffmpeg, pdftoppm, ffplay or similar); videos don't play inside dirmap, and seeking in sound is Windows-only. Archives are read-only (no adding to them), and RAR and password-protected archives aren't supported. Changes on network shares are noticed only if the server reports them. No Redo. Undo can't restore items deleted permanently, or ones the bin no longer has. No folder colours, tags, batch rename, folder compare or sync. No image-sequence previews yet. SFTP, FTP and WebDAV aren't built in: mount them with your system. Network & Remote on macOS and Linux needs the share to be mountable without a dirmap password prompt (the system asks). The analysis dock draws the treemap when a scan finishes, not while it runs (it shows the scan's progress meanwhile). |
| Right-click menus | Only in the file manager and the File types list. The treemap and sunburst have none (right-click zooms out there). WinDirStat's **Compress**, **Sparsify**, **Optimize VHD**, **Custom Cleanup**, the full **Windows Explorer** context submenu and **Group Unregistered Types** aren't available. |
| Keyboard use | The file manager can be used entirely from the keyboard, and there are shortcuts for file commands, the Custom filter, help and the treemap, but there's no keyboard way to move between the sidebar and the list (Tab moves between the two panes), and the Tree layout can't be moved through with the arrow keys. |
| Duplicate verification | Matches use a 64-bit hash of the whole contents, not a byte-by-byte comparison. There's no "delete all but one" action; copies are deleted one at a time. |
| Sunburst navigation | No Back/Forward history (the treemap and the file manager have it). |
| File manager | Copy and Cut work only inside dirmap (the paths go to the clipboard as text), and files can't be dragged to or from other apps. Changes made by other apps show after a Refresh. Real Finder icons on macOS aren't used yet (drawn icons instead). |
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
