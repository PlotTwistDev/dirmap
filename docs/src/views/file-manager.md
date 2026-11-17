# The Files workspace

dirmap opens in the **Files** workspace: a file manager like Windows Explorer, Finder or Dolphin.
You can browse your computer straight away, with no scan. When you want to know where the space
goes, **Analyse This Folder** scans it and takes you to Storage Analysis.

![The Files workspace, with folder sizes from an earlier scan](../images/file-manager.png)

## Workspaces

The buttons at the left of the toolbar switch between the two workspaces. dirmap reopens in the
one you used last.

| Workspace | What it's for |
|---|---|
| **🗀 Files** (**Ctrl+1**, **⌘1** on a Mac) | Browse drives, folders and network shares, and open files. Folders are read from the disk as you open them, so there's nothing to wait for. |
| **▦ Storage Analysis** (**Ctrl+3**, **⌘3**) | The scan, with its [file manager](file-browser.md), [treemap](treemap.md), sunburst, file types, duplicates, snapshots and sequences. |

Switching keeps your place:

- **Files to Storage Analysis.** If the scan includes the folder you're in, Storage Analysis opens
  at that folder, with the same item selected.
- **Storage Analysis to Files.** Files shows the folder Storage Analysis is showing, with the same
  item selected.

Starting dirmap with a folder (`dirmap D:\Projects`) or a snapshot to compare (`--compare`) opens
Storage Analysis and scans it, as before.

A scan keeps running while you browse in Files. The status bar shows its progress, and **Show**
switches to it.

## The sidebar

| Group | What's in it |
|---|---|
| **Quick access** | Home, Desktop, Documents, Downloads, Pictures, Music and Videos (and Applications on a Mac). These are your real folders, including ones that have been moved, such as Documents on OneDrive. |
| **Pinned** | Folders you've pinned. Right-click any folder, place or drive and choose **Pin to Sidebar**. Right-click a pinned folder to unpin it or move it up or down. |
| **Drives** | **This PC** (Windows) or **Computer** (Linux and macOS), then each drive or mounted volume, network drives included. Each has a bar showing how full it is, red when over 90%. Hover for the free space. |
| **Recent** | The last ten folders you opened. |

Click an entry to go there. Middle-click opens it in a new tab. Drag the sidebar's edge to make
it wider.

## Moving around

- **◀ ▶** go Back and Forward through the folders you've opened in this tab. **Alt+←** and
  **Alt+→** do the same, as do the mouse's side buttons.
- **⏶** (or **Backspace**) goes up a level. Above a drive is the **Computer** view, which shows
  every drive with its space.
- **⟳** (or **F5**) reads the folder again.
- **The path.** Click any part of it to go there. Click the empty space after it, or press
  **Ctrl+L**, to type a path:
  - `D:` means the whole drive;
  - `~/Projects` is in your home folder;
  - `\\server\share` opens a Windows share.
- **Double-click** a folder to open it, or a file to open it with its usual app. **Enter** does
  the same for the selected item.

## Tabs

Each tab has its own folder and history. **Ctrl+T** opens a tab, **Ctrl+W** closes one,
**Ctrl+Shift+T** reopens the last one you closed, and **Ctrl+Tab** moves between them.
Middle-click a folder to open it in a new tab. Your tabs are there again the next time you
start dirmap.

## The list

| Column | What it shows |
|---|---|
| **Name** | The file or folder. Hidden items are drawn fainter. An arrow on the icon marks a link (a shortcut, symlink or junction). |
| **Size** | The file's size. A folder shows its total in *italics* when the current scan includes it. Hover for the number of files. Without a scan, folders show no size. |
| **Type** | Folder, or the file's kind, for example "EXR image" or "SAFETENSORS AI model". |
| **Date modified** | When the item was last changed. |

Click a heading to sort by it, and again to reverse. Folders always come first, and names
sort in natural order, so `shot2` comes before `shot10`.

To choose items:

- **Ctrl+click** adds or removes one item.
- **Shift+click** (or **Shift+↑**/**↓**) selects a range.
- **Ctrl+A** selects everything shown, and **Escape** selects nothing.
- **Typing** the start of a name jumps to it.

The bar under the list counts the items and adds up the size of what's selected.

- **Filter** (**Ctrl+F**) shows only the names containing what you type. `*` and `?` work as
  wildcards, for example `*.exr` or `shot_??.png`. **Escape** clears it.
- **Hidden** (**Ctrl+Shift+.**) shows hidden files and folders: names starting with a dot, and
  items marked hidden on Windows.

## Right-click menu

| Command | What it does |
|---|---|
| **Open** / **Open in New Tab** | Opens folders here or in a new tab, and files in their apps. |
| **Copy Path** (**Ctrl+Shift+C**) / **Copy Name** | Puts the full paths, or just the names, on the clipboard, one per line. |
| **Show in Explorer** (Finder, or the system file manager) | Opens the system's file manager with the item selected. |
| **Open in Terminal…** (Command Prompt or PowerShell on Windows) | Opens a terminal in the folder. |
| **Pin to Sidebar** / **Unpin from Sidebar** | Adds the folder to **Pinned**, or takes it off. |
| **Analyse This Folder…** | Scans the folder and switches to Storage Analysis. |

## Copying, moving and deleting

Rename, copy, move and delete aren't in the Files workspace yet. They're planned for the next
stage, with a progress window, conflict handling and Undo. Until then, use the
[file manager in Storage Analysis](file-browser.md), which has all of them.

## Slow and unreachable drives

Folders are read in the background, a batch at a time, so a folder of 100,000 files starts
showing at once, and the window never freezes. If a network share or a sleeping disk takes more
than two seconds to answer, the list says so and offers **Cancel**. You can use other tabs,
drives and Storage Analysis while you wait.

If a folder can't be opened, the list says why, for example because it doesn't exist or you
don't have permission, and offers **Go up**.

![The Computer view, with every drive and its space](../images/file-manager-computer.png)

## Related topics

- [The file manager in Storage Analysis](file-browser.md)
- [Scanning a drive or folder](../getting-started/scanning.md)
- [Keyboard shortcuts](../reference/keyboard.md)
- [External and network drives](../advanced/network-drives.md)
