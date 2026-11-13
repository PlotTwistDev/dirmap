# Troubleshooting

Find the problem below, or search this help for words from an error message.

## A drive or folder can't be scanned

- **The drive isn't in the list.** CD/DVD drives and drives that aren't ready (an empty card
  reader, a disconnected network share) aren't listed. Type the drive's path (such as `E:\` or
  `/media/you/disk`) in the folder box and press **Enter**. Press **🖴 Drives…** again to refresh
  the list after connecting a drive.
- **The scan finishes immediately with nothing in it.** The path doesn't exist or can't be read.
  Check the spelling; on Windows use `D:\`, not `D:` followed by a folder name without a
  backslash. A path can be pasted from Explorer's address bar.
- **A network drive is slow or stalls.** See [External and network drives](../advanced/network-drives.md).
- **macOS: folders under your home or Library are empty.** Give dirmap **Full Disk Access** in
  **System Settings > Privacy & Security** and restart it.

## Access denied errors

dirmap doesn't stop for folders it can't read: it counts them as **unreadable** and carries on.
The progress line and the status bar say how many there were. To read them, run dirmap as
administrator on Windows (**⚙ Settings > Advanced > 🛡 Restart as administrator now**), give it
Full Disk Access on macOS, or start it with `sudo` on Linux. See
[Hidden, system and unreadable files](../disk-usage/hidden-and-unreadable.md).

Right-click commands can also be refused (for example deleting a file that's in use, or a
protected system file). The reason appears in red in the status bar.

## Scan results appear incomplete

Check, in order:

1. **Unreadable items.** Does the status bar say **N items couldn't be read**? See above.
2. **Scan exclusions.** Is anything listed under **⚙ Settings > Filtering**? Those folders and
   files are never counted. See [Scan exclusions](../filtering/scan-filters.md).
3. **Skip hidden files** in **⚙ Settings > Advanced**: hidden and system files aren't counted
   while it's on.
4. **Stay on the scanned drive** (Linux and macOS): other drives mounted inside the folder are
   skipped.
5. **Links** are never followed, so a folder reached through a link or junction appears empty.
   Scan the real folder instead. See [Links, junctions and mounted drives](../disk-usage/links-and-mounts.md).
6. **A filter is on.** Filters hide files from the map without changing the totals. Look for
   **Show matches only** or a highlighted **Show:** button, and press **Clear filters**.
7. **The scan was cancelled** or files changed afterwards. Press **⟳ Scan** to scan again, or select
   a folder and press **F5** to refresh just that folder.

## Files cannot be found

- dirmap's filters match **file names**, not folders or paths: `Renders*` finds files whose names
  start with "Renders", not files in a folder called Renders. Open the folder on the map instead.
- Check the matching options in the Custom filter's **⏷** menu: **Case-sensitive** or **Match all
  terms (AND)** may be on.
- A file created after the scan isn't in it. Select its folder and press **F5**.
- The file may be in an excluded or unreadable folder (see above).

## Unexpected disk usage

dirmap's total and Windows' (or the drive's) used space measure different things. See
[Why is free space lower than expected?](../tutorials/missing-space.md) for a step-by-step check.

## Differences between file sizes and Windows' disk usage

dirmap shows each file's **size**, like the **Size** in Explorer's Properties, not **Size on disk**.
They differ for compressed files, sparse files, cloud placeholders and hard links, and by up to
one cluster (usually 4 KB) per file. Folder totals in dirmap also include hidden and system files
by default, which Explorer may hide. See [How sizes are calculated](../disk-usage/sizes.md).

## Zooming or rendering is slow

- **New detail takes a moment after a big zoom.** That's normal on very large scans: tiles are
  worked out in the background and appear when ready. The map stays usable meanwhile.
- **Turn off effects** in **⚙ Settings > Treemap**: **Interactive lighting**, **Block depth
  effect** and **File names on tiles** all cost a little time per frame.
- **Turn off Smooth zoom** if animations feel sluggish on a slow computer.
- **Remote desktop and virtual machines** may have no GPU acceleration. dirmap falls back to
  OpenGL automatically; if the window is blank, garbled or very slow, start dirmap with the
  environment variable `DIRMAP_RENDERER=glow` set (see [Command line](../reference/command-line.md)).
- **Update your graphics driver** if the window flickers or shows corrupted colours.

## Large scans consume excessive memory

dirmap keeps the whole scan in memory: roughly 150–250 MB per million files. To use less:

- Scan a smaller folder rather than the whole drive.
- Add large folders you don't care about (backups, caches) to **Folders to skip**.
- Close a snapshot comparison (**Stop comparing**) and switch **Show matches only** back to
  **Highlight**: each holds an extra copy of the tree.
- Close and reopen dirmap after very large scans to return all memory to the system.

See [Scanning millions of files](../advanced/millions-of-files.md).

## Filtering returns no results

- Look at the orange **⚠** message under the Custom box: the pattern may be invalid, in which case
  the previous pattern is still in effect.
- Patterns match whole names: `beauty` matches only files named exactly `beauty` or ending in
  `.beauty`. Use `*beauty*` to find names containing it.
- `####` needs **exactly** four digits, and a frame symbol always takes the whole number:
  `render.####.exr` won't match `render.00001.exr`. Try `{frame}`.
- Several filters must **all** match: a **Show:** category, size limits and the Custom pattern are
  combined. Press **Clear filters** and add them back one at a time.
- **Case-sensitive** or **Match all terms (AND)** may be on in the **⏷** menu.

See [Custom filter syntax](../filtering/syntax.md).

## A keyboard shortcut does nothing

- Shortcuts for files (**Delete**, **Enter**, **Ctrl+C**…) act on the **selected** item. Click
  something first.
- They don't work while the cursor is in a text box (the folder box, the Custom filter, a size
  box). Click on the map or the file manager first.
- Treemap keys (**+**, **−**, **Home**, **Alt+←**, **Alt+→**) only work when the treemap is shown.
- See [Keyboard shortcuts](../reference/keyboard.md) for the full list.

## Color by: Change is greyed out

It needs a snapshot to compare with. Open the **Changes** tab and press **📂 Compare with
snapshot…**. See [Comparing with a snapshot](../analysis/snapshots.md).

## Delete or Move doesn't work

- The top item of the scan can't be deleted or moved from dirmap; open its parent folder instead.
- Commands are greyed out while another file operation is running.
- **Linux:** moving to the Trash needs `gio`, `kioclient` or `trash-cli`.
- Files in use by a program can't be deleted on Windows; close the program and try again.

## Settings aren't remembered

Settings are saved when dirmap closes normally. If it's ended forcibly (Task Manager, a crash),
changes since it started may be lost. Check that the settings folder is writable; see
[Where settings are stored](../settings/reference.md#where-settings-are-stored).

## macOS says dirmap can't be opened

dirmap isn't notarised by Apple. Right-click it and choose **Open**, or run
`xattr -dr com.apple.quarantine dirmap.app`. See [Installation and first launch](../getting-started/installation.md).

## Related topics

- [Frequently asked questions](faq.md)
- [Feature status](../about/feature-status.md)
