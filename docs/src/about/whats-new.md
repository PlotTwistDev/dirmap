# What's new

## Version 0.2.0

### File manager

- The folder list at the top is now a **file manager** like Finder or Windows Explorer, with the
  map still underneath. Choose **List**, **Icons**, **Columns** or **Tree**, each with every
  item's size and share of its folder. Sort by name, size, kind, items or date; click the path,
  use Back, Forward and Up, the arrow keys and type-to-select. Opening a folder zooms the map
  there, and zooming the map moves the file manager. See [The file manager](../views/file-browser.md).
- **Tabs**: **Ctrl+T**, **Ctrl+W**, **Ctrl+Tab**, middle-click a folder or **Open in New Tab**.
- **Select several items** with Ctrl- and Shift-click or **Ctrl+A**.
- **Rename** (**F2**), **New Folder** (**Ctrl+Shift+N**), **Copy**, **Cut** and **Paste**
  (**Ctrl+C**, **Ctrl+X**, **Ctrl+V**), and **drag and drop** onto folders, the path or tabs
  (hold **Alt** to copy).
- **Copy Path** is now **Ctrl+Shift+C**, since **Ctrl+C** copies the items.

### Help and documentation

- **Help Centre** built into dirmap: the complete manual, tutorials, keyboard and mouse
  references, filter syntax and troubleshooting, searchable and available offline. Open it from
  **❓ Help** in the toolbar.
- **F1** opens help for the part of the window under the pointer.
- More tooltips and **?** help links in the Settings window, the Custom filter and the side tabs.
- The status bar now says how many items couldn't be read after a scan.

### Custom filter and sequences

- **🔍 Custom** filter (**Ctrl+F**): wildcards, multiple patterns, exclusions with `!`, frame
  numbers with `#` and `{frame}`, padding and ranges, case-sensitive and match-all options,
  presets, recent patterns, examples and extension completion. See
  [Custom filter syntax](../filtering/syntax.md).
- **Highlight** or **Show matches only** for every filter, with a live match count.
- **Sequences** tab: numbered image and render sequences with frame ranges and missing frames.

### Right-click menus

- A WinDirStat-style right-click menu on the file manager, with keyboard shortcuts: Open, Copy
  Path, Select in Explorer, Command Prompt and PowerShell (Terminal elsewhere), Refresh Selected,
  Properties, Compute Hash (SHA-256), Add to Exclusion Filter, Delete Permanently, Delete to
  Recycle Bin, Remove Empty Folders, Empty Folder and Move to.
- A right-click menu on the File types list: Find all files of a type, Highlight, Copy
  Extension, Add to Exclusion Filter.
- Only changed items are read again after a file operation.

### Treemap

- Smooth zoom towards the pointer, panning with inertia, animated opening and closing of folders,
  Back and Forward (Alt+←/→ and the mouse's side buttons), Zoom to selection and Whole map.
- GPU drawing: only what's in view is worked out, so millions of files stay smooth.
- Interactive pointer lighting, a slight bevel, and tile labels that fade in.
- Navigation and effects settings under **⚙ Settings > Treemap**.

### Earlier in 0.2.0

- Windows: offer to run as administrator.
- Linux and macOS builds, a macOS app bundle and correct whole-disk scanning on macOS.
- Sunburst view, colour by age and change, size histogram, snapshots and comparison, duplicate
  finder.
- Wheel zoom, tile labels, hover information and kind and size filters.
- Live scan progress and live file manager, Explorer icons on Windows, the Settings window.
- Light theme fixes.

## Version 0.1

- First version: parallel scanning, drive list, file manager and cushion treemap.

## Related topics

- [Feature status](feature-status.md)
- [About dirmap](about.md)
