# Feature status

This page says plainly what Filescape does today, what works only partly, and what isn't available
yet, so you don't go looking for something that isn't there. It's kept up to date with each
version.

## Available

| Feature | Notes |
|---|---|
| Files workspace | Browse without scanning: sidebar (Quick access, Favourites in groups, Drives with space, Network & Remote, Recent), Computer view, two panes side by side or stacked, Details, Icons, Columns or Gallery view per pane with pictures of images (EXR and HDR too), videos and PDFs, each folder remembering its view, sort and grouping, grouping by type, date or size, zip/7z/tar archives browsed like folders with Extract Here and Extract To, file watching, copies and moves with pause, stop and a queue, copy/cut/paste, drag and drop between folders, panes, tabs, path and sidebar (move on the same drive, copy across drives), rename, Recycle Bin/Trash and permanent delete, New folder and files from templates, Quick Look and a preview pane (images including EXR and HDR, text, video frames, PDF first pages, sound playback, archive contents), an address bar with suggestions, Tab completion, environment variables, `>` commands and `@` favourites, Windows mapped drives and SMB connect/disconnect (mounts on macOS and Linux), image sequences shown as one row (range, missing frames, copied and moved as one) and played from memory with a timeline, loop and ping-pong, frame rates, exposure, sRGB, Rec.709 and Cineon log display, half and quarter size, and an A/B wipe (EXR, DPX at 8 to 16 bits, PNG, TIFF and the rest), SFTP, FTP(S) and WebDAV servers browsed and changed like drives (saved in the sidebar, SSH agent, key files or passwords kept only in the system keychain, server identity checks) and analysed, tabs restored at start, Back/Forward/Up, clickable and typed paths (including UNC), sortable Details list, filter box, hidden files, multi-select, type to select, right-click menu, Open With your own apps, batch rename with a preview (find and replace, regular expressions, case, numbering, extension), colours, tags and notes with a Tags sidebar, search through subfolders with saved searches, Compare Folders with Update, Both Ways and Mirror sync, Redo, F6 keyboard access to the sidebar, `.cube` LUTs in the sequence player, Analyse This Folder, folder sizes from the last scan, resizable columns, row density, draggable tabs. See [The Files workspace](../views/file-manager.md). |
| Analysis dock | The treemap or sunburst of the current folder under the list, from the existing analysis. It has:<br>- Follow folder, Pinned and Follow selection;<br>- Analyse and Refresh in the background;<br>- the two most recent earlier analyses kept;<br>- a changed-since-analysis check;<br>- two-way selection, the tile menu, folder facts and a type legend.<br>See [The analysis dock](../views/file-manager.md#the-analysis-dock). |
| Drive list and folder scanning | Parallel, with live progress and a live file manager. Windows, Linux and macOS. |
| Directory tree | Sortable Size, % of parent, Files and Last change columns; Explorer icons on Windows. |
| Treemap | Cushion shading on the GPU, cursor-anchored smooth zoom, pan with inertia, drill in and out, Back/Forward, zoom to selection, labels, pointer lighting. |
| Sunburst | Six rings, zoom in and out, selection. |
| File manager | Finder/Explorer-style browsing above the map in List, Icons, Columns and Tree layouts: tabs, sizes and share of folder, sorting, path bar, Back/Forward, keyboard navigation, type to select, multi-select, rename, new folder, copy/cut/paste, drag and drop, filters, right-click menu; moves with the map. |
| Colour by type, age, and change since a snapshot | |
| File types list | Highlight, find all files of a type, copy, exclude. |
| Category, size and Custom name filters | Highlight or Only matches; wildcards, exclusions, AND/OR, frame numbers, padding and ranges; presets and recent patterns. |
| Numbered sequence detection | Frame ranges, steps, missing frames. |
| Size histogram | Click a range to filter. |
| Snapshots and comparison | Save, compare, colour by change; from the command line too. |
| Duplicate finder | Size, then first 64 KB, then whole-file hash. |
| Right-click menu in the file manager | Open, Copy, Cut, Paste, Rename, New Folder, Copy Path, reveal, terminals, Refresh, Properties, SHA-256, exclude, Delete, Recycle Bin/Trash, Remove Empty Folders, Empty Folder, Move to, with keyboard shortcuts. |
| Settings | Appearance, scan exclusions, columns, treemap look and navigation, threads, administrator mode. |
| Licensing | A 15-day evaluation, then free and Professional features; perpetual and temporary (time-limited, non-renewable) licences, online or offline activation, licence details, certificate and details export, a weekly check. See [Licensing](../settings/licensing.md). |
| Run as administrator | Windows: ask, always or never, and restart on request. |
| Command-line report and snapshots | `--cli`, `--top`, `--save-snapshot`, `--compare`, `--all-fs`. |
| Built-in help | This Help Centre, with search, F1 context help and tooltips. |

## Partly available

| Feature | What's missing |
|---|---|
| Files workspace | Grouping is in the Details view only. Video and PDF pictures and sound need the system's own thumbnailers and players (on Linux: ffmpeg, pdftoppm, ffplay or similar); videos don't play inside Filescape, and seeking in sound is Windows-only. Archives are read-only (no adding to them), and RAR and password-protected archives aren't supported. Changes on network shares are noticed only if the server reports them. Undo can't restore items deleted permanently, or ones the bin no longer has. Colours, tags and notes are kept by Filescape, not in the files, so they're lost when another program renames or moves the items. Compare contents works on local folders only, and a subfolder search isn't updated when files change (press F5). The sequence player has no OpenColorIO configs (`.cube` LUTs only), no audio, no pixel aspect ratio, and ignores alpha; DPX is read from the first image element only, uncompressed. Sequences are grouped one folder at a time. On servers (SFTP, FTP, WebDAV): delete is always permanent, Undo can't restore what was deleted there, changes made by other programs aren't noticed (press F5), files opened in apps are copies whose changes aren't sent back, archives open as files, and certificates must be trusted by the system (no per-server exceptions). No SCP-only or S3 servers. Network & Remote on macOS and Linux needs the share to be mountable without a Filescape password prompt (the system asks). The analysis dock draws the treemap when a scan finishes, not while it runs (it shows the scan's progress meanwhile). |
| Right-click menus | Only in the file manager and the File types list. The treemap and sunburst have none (right-click zooms out there). WinDirStat's **Compress**, **Sparsify**, **Optimize VHD**, **Custom Cleanup**, the full **Windows Explorer** context submenu and **Group Unregistered Types** aren't available. |
| Keyboard use | The file manager can be used entirely from the keyboard, and there are shortcuts for file commands, the Custom filter, help and the treemap, but there's no keyboard way to move between the sidebar and the list (Tab moves between the two panes), and the Tree layout can't be moved through with the arrow keys. |
| Duplicate verification | Matches use a 64-bit hash of the whole contents, not a byte-by-byte comparison. There's no "delete all but one" action; copies are deleted one at a time. |
| Sunburst navigation | No Back/Forward history (the treemap and the file manager have it). |
| File manager | Copy and Cut work only inside Filescape (the paths go to the clipboard as text), and files can't be dragged to or from other apps. Changes made by other apps show after a Refresh. Real Finder icons on macOS aren't used yet (drawn icons instead). |
| Size accuracy | Sizes on disk and logical sizes for every file. On Windows, hard links are counted once per name. Files on mounted cloud drives (rclone, FUSE, Google Drive's `G:` drive) show what the drive reports, marked **Unknown**. Pinned cloud files are recognised on Windows only. Freeing up and keeping cloud files works on Windows and macOS with apps that keep files on demand, not on Linux or mounted cloud drives; macOS has no "always keep" setting Filescape can set. See [How sizes are calculated](../disk-usage/sizes.md) and [Cloud files](../disk-usage/cloud-files.md). |
| Licensing | Filescape Professional is sold at filescape.app; serials activate online or with offline request files. Licences for organisations (seat pools, deployment) are planned. |
| Administrator mode | A restart button on Windows only; on Linux use `sudo`, on macOS Full Disk Access. |

## Not available

These aren't in Filescape yet. Some may come later; none should be relied on.

- Counting hard-linked files once on **Windows** (macOS and Linux do, in sizes on disk).
- **Compressing**, **sparsifying** or **compacting virtual disks** from Filescape.
- **Selecting several items** at once, and bulk operations.
- **Exporting** results (CSV, report files) from the window. The command-line report is text only.
- **Automatic rescans** or watching drives for changes.
- **Fast NTFS scanning** by reading the master file table directly.
- **Comparing two different folders** side by side (snapshots compare the same folder over time).
- **Scanning several drives into one map.**
- A **translated** interface; Filescape is in English only.

## Related topics

- [What's new](whats-new.md)
- [How Filescape works](../advanced/architecture.md)
