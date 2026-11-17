# The Files workspace

dirmap opens in the **Files** workspace: a file manager like Windows Explorer, Finder or Dolphin.
You can browse your computer straight away, with no scan. When you want to know where the space
goes, open the [analysis dock](#the-analysis-dock) under the list: it shows the treemap of the
folder you're in, and follows you as you browse.

![The Files workspace, with folder sizes from an earlier scan](../images/file-manager.png)

## Workspaces

The buttons at the left of the toolbar switch between the two workspaces. dirmap reopens in the
one you used last.

| Workspace | What it's for |
|---|---|
| **🗀 Files** (**Ctrl+1**, **⌘1** on a Mac) | Browse drives, folders and network shares, and open files. Folders are read from the disk as you open them, so there's nothing to wait for. |
| **⊞ Storage Analysis** (**Ctrl+3**, **⌘3**) | The scan, with its [file manager](file-browser.md), [treemap](treemap.md), sunburst, file types, duplicates, snapshots and sequences. |

Switching keeps your place:

- **Files to Storage Analysis.** If the scan includes the folder you're in, Storage Analysis opens
  at that folder, with the same item selected.
- **Storage Analysis to Files.** Files shows the folder Storage Analysis is showing, with the same
  item selected.

Starting dirmap with a folder (`dirmap D:\Projects`) or a snapshot to compare (`--compare`) opens
Storage Analysis and scans it, as before.

A scan keeps running while you browse in Files. The right of the status bar shows it, and any
other work running in the background (finding duplicates, applying a filter, file commands);
click it for details.

## The sidebar

| Group | What's in it |
|---|---|
| **Quick access** | Home, Desktop, Documents, Downloads, Pictures, Music and Videos (and Applications on a Mac). These are your real folders, including ones that have been moved, such as Documents on OneDrive. |
| **Favourites** | Folders you've bookmarked, in as many groups as you like. See [Favourites](#favourites). |
| **Drives** | **This PC** (Windows) or **Computer** (Linux and macOS), then each local drive or mounted volume. Each has a bar showing how full it is, red when over 90%. Hover for the free space. |
| **Network & Remote** | Network drives and shares, saved servers, and **Connect to Server…**. See [Network & Remote](#network--remote). |
| **Recent** | The last ten folders you opened. |

Click an entry to go there. Middle-click opens it in a new tab. Click a group's title to fold it
away. Drag the sidebar's edge to make it wider. Every entry also takes dropped files: drop items
on a drive or a favourite to copy or move them there.

## Favourites

Drag a folder from the list, a tab or the path onto **Favourites** to bookmark it. A line shows
where it will go, and the label by the pointer says *a bookmark: nothing is moved*. The folder
stays where it is. You can drop several folders at once. **Add to Favourites** on any folder's
right-click menu does the same.

- **Reorder** a favourite by dragging it up or down, or into another group.
- **Drop files onto the middle of a favourite** to copy or move them into that folder, as with
  any other folder. Near the top or bottom edge of a row, a drop adds a bookmark instead.
- **Right-click a favourite** for **Rename…** (the name shown in the sidebar; the folder isn't
  renamed), **Remove from Favourites**, Open in New Tab, Copy Path and Analyse.
- **Right-click a group's title** for **New Group**, **Rename Group…**, **Move Group Up/Down** and
  **Remove Group**. Click the title to fold the group away.
- A favourite whose folder has gone (a drive that isn't plugged in, a renamed folder) stays in
  the list; opening it says the folder can't be found.

Folders pinned in earlier versions of dirmap appear in the first group.

## Moving around

- **◀ ▶** go Back and Forward through the folders you've opened in this tab. **Alt+←** and
  **Alt+→** do the same, as do the mouse's side buttons.
- **⏶** (or **Backspace**) goes up a level. Above a drive is the **Computer** view, which shows
  every drive with its space.
- **⟳** (or **F5**) reads the folder again.
- **The path.** Click any part of it to go there, or the small arrow after a part for the
  folders inside it. Drop files on a part to move or copy them there. Click the empty space
  after it, or press **Ctrl+L** (also **Alt+D** or **F4** on Windows and Linux), to
  [type a path](#typing-a-path).
- **Double-click** a folder to open it, or a file to open it with its usual app. **Enter** does
  the same for the selected item.

## Typing a path

While you type, a list under the path suggests folders that match, then recent folders and
favourites. **↑**/**↓** choose one, **Tab** completes the name, **Enter** goes there and
**Escape** puts the path back.

| You type | You get |
|---|---|
| `D:` | The whole drive. |
| `~/Projects` | A folder in your home folder. |
| `%USERPROFILE%\Downloads`, `$HOME/Downloads`, `${HOME}` | Environment variables are filled in. |
| `\\server\share` | A Windows share, opened directly. |
| `smb://server/share` | The same share on Windows. On a Mac or Linux, connect to it first with **Connect to Server…**. |
| `file:///home/me` | A file URL. |
| `>` then a word | A command, for example `>split`, `>icons`, `>new folder`, `>hidden`, `>connect` or `>preview`. The list shows each command with its shortcut. |
| `@` then a name | Your favourites whose names match. |

The path box never runs programs or shell commands. SFTP, FTP and WebDAV addresses aren't built
in yet: the box says so instead of trying.

## The toolbar

From left to right:

- the workspaces;
- **⏴ ⏵ ⏶ ⟳**: Back, Forward, Up and Refresh;
- **✚ New**, the [New menu](#new-folders-and-files);
- the path;
- **⚡ Analyse Folder**, which becomes **⟳ Refresh Analysis** when the folder is already
  analysed, or **■ Cancel Scan** while a scan runs (see [the analysis dock](#the-analysis-dock));
- **⊞ Analysis**, which shows or hides the dock;
- **👁** shows or hides the [preview pane](#quick-look-and-the-preview-pane), and **⬌** splits
  the view into [two panes](#two-panes);
- the filter, the **View** menu, Help and Settings.

The **View** menu has:

- **Hidden Items**, which shows hidden files and folders;
- **Comfortable** or **Compact** rows;
- **Reset Column Widths**;
- **Panes**: **One Pane**, **Side by Side** or **One Above the Other**, then **Swap Panes** and
  **Same Folder in Other Pane**;
- **☰ Details** or **⊞ Icons** for the pane you're in, and in Icons, **Sort By** and the icon
  **Size** (see [The Icons view](#the-icons-view));
- **Preview Pane**;
- **Analysis Dock**.

## Tabs

Each tab has its own folder and history. **Ctrl+T** opens a tab, **Ctrl+W** closes one,
**Ctrl+Shift+T** reopens the last one you closed, and **Ctrl+Tab** moves between them.
Middle-click a folder to open it in a new tab. Drag a tab to move it along the strip, and
right-click one for **Close Other Tabs**. When there are more tabs than fit, they get narrower,
then the strip scrolls. Your tabs are there again the next time you start dirmap.

## Two panes

Press **Ctrl+\\** (**⌘\\** on a Mac), click **⬌**, or choose **View > Panes > Side by Side** to show
two folders at once. **One Above the Other** stacks them instead.

- Each pane has its own tabs, history, sort order, filter and selection.
- The pane you're working in has a thin coloured line along its top, and the toolbar, the path
  and the keyboard work on it. Click in a pane, or press **Tab**, to switch.
- Drag items from one pane onto the other, onto a folder in it, or onto one of its tabs.
- **Shift+F5** copies the selected items to the other pane's folder, and **Shift+F6** moves them.
  The right-click menu has **Copy to Other Pane** and **Move to Other Pane** too.
- **Ctrl+U** swaps the panes. Drag the line between them to share the room, and double-click it
  to make them even.
- Press **Ctrl+\\** again to go back to one pane. dirmap keeps the other pane's tabs for next
  time, and remembers the split when you quit.

## The list

| Column | What it shows |
|---|---|
| **Name** | The file or folder. Hidden items are drawn fainter. An arrow on the icon marks a link (a shortcut, symlink or junction). |
| **Size** | The file's size. A folder shows its total in *italics* when the current scan includes it. Hover for the number of files. Without a scan, folders show no size. |
| **Type** | Folder, or the file's kind, for example "EXR image" or "SAFETENSORS AI model". |
| **Date modified** | When the item was last changed. |

Click a heading to sort by it, and again to reverse. Folders always come first, and names
sort in natural order, so `shot2` comes before `shot10`. Drag the divider at a column's left edge
to resize it, and double-click the divider to put the column back to its usual width. dirmap
remembers the widths.

To choose items:

- **Ctrl+click** adds or removes one item.
- **Shift+click** (or **Shift+↑**/**↓**) selects a range.
- **Ctrl+A** selects everything shown, and **Escape** selects nothing.
- **Typing** the start of a name jumps to it.

The bar under the list counts the items and adds up the size of what's selected.

- **Filter** (**Ctrl+F**) shows only the names containing what you type. `*` and `?` work as
  wildcards, for example `*.exr` or `shot_??.png`. **Escape** clears it.
- **View > Hidden Items** (**Ctrl+Shift+.**) shows hidden files and folders: names starting with
  a dot, and items marked hidden on Windows.

## The Icons view

Click **⊞** at the bottom right of the list (or choose **View > ⊞ Icons**) to see the folder as
big icons. Images show a small picture of themselves, so you can find a render or a photo at a
glance. **☰** goes back to Details. Each pane remembers its own choice.

![The Icons view, with pictures of images and the preview pane](../images/file-manager-icons.png)

- **Size.** Hold **Ctrl** (**⌘** on a Mac) and turn the mouse wheel over the icons, or pinch on a
  trackpad, or use **View > Size**.
- **Sorting.** There are no column headings, so use **View > Sort By**: Name, Size, Type or Date
  modified, and **Reverse Order**.
- **Keys.** **←** and **→** move along a row, **↑** and **↓** to the row above or below. Everything
  else works as in Details: Shift and Ctrl to select several, **Enter** to open, **Space** for
  Quick Look, typing a name to jump to it.
- Clicking, dragging, dropping onto folders and the right-click menu work as in Details.

Pictures are made for the same image types as the [preview](#quick-look-and-the-preview-pane),
including OpenEXR and HDR, in the background and only for the icons on screen. Scrolling quickly
through thousands of images never freezes the window, and dirmap keeps the pictures within a
fixed amount of memory, dropping the ones you haven't seen for a while. A file that can't be
read shows its type's icon instead.

## Right-click menu

| Command | What it does |
|---|---|
| **Open** / **Open in New Tab** / **Open in Other Pane** | Opens folders here, in a new tab or in the other pane, and files in their apps. |
| **Quick Look** (**Space**) | A [large preview](#quick-look-and-the-preview-pane) of the file. |
| **Copy** / **Cut** / **Paste** (**Ctrl+C**, **Ctrl+X**, **Ctrl+V**) | See [Copying, moving and deleting](#copying-moving-and-deleting). |
| **Copy to Other Pane** / **Move to Other Pane** (**Shift+F5**, **Shift+F6**) | With two panes. |
| **Rename…** (**F2**) | Renames the item. Only the name is selected at first, not the extension. |
| **Move to Trash…** (**Delete (to Recycle Bin)…** on Windows; **Delete** key) / **Delete Permanently…** (**Shift+Delete**) | Asks first, saying how much is involved. |
| **Copy Path** (**Ctrl+Shift+C**) / **Copy Name** | Puts the full paths, or just the names, on the clipboard, one per line. On a mapped network drive, **Copy Network (UNC) Path** gives the `\\server\share\…` form. |
| **Add to Favourites** | Bookmarks the folder in [Favourites](#favourites). |
| **Show in Explorer** (Finder, or the system file manager) | Opens the system's file manager with the item selected. |
| **Open in Terminal…** (Command Prompt or PowerShell on Windows) | Opens a terminal in the folder. |
| **Properties** (**Alt+Enter**) | Size, number of files and dates. |
| **Analyse This Folder** | Scans the folder and shows it in the [analysis dock](#the-analysis-dock). You stay in Files. |

Right-click empty space in the list for **New**, **Paste**, **Refresh**, Open in New Tab, Add to
Favourites and Analyse.

## New folders and files

**✚ New** on the toolbar (or on the right-click menu of empty space) makes something new in the
folder you're in, then opens the Rename box so you can name it straight away. It never replaces
a file that's already there: the new one is called `New folder 2`, `README 2.md` and so on.

| Item | What you get |
|---|---|
| **Folder** (**Ctrl+Shift+N**) | An empty folder. |
| **Text Document**, **Markdown**, **JSON**, **XML**, **CSV** | A small valid file of that kind, for example `{}` for JSON, never an empty file pretending to be something it isn't. Hover an item to see what it will hold. |
| **Script** | A shell or Python script on a Mac or Linux (marked as runnable), or PowerShell, batch or Python on Windows. |
| Your templates | Anything you put in the templates folder, file or folder, copied as it is. |
| **Open Templates Folder** | Opens the folder in a new tab, making it first. |

Put any file or folder in the templates folder (a `.blend` scene, a project folder with its
subfolders, a document with your letterhead) and it appears in the New menu. The folder is:

- Windows: `%APPDATA%\dirmap\Templates`;
- macOS: `~/Library/Application Support/dirmap/Templates`;
- Linux: `~/.local/share/dirmap/Templates`, plus your desktop's own Templates folder.

dirmap doesn't make Word, Excel, Photoshop or other formats it can't write correctly. Save one
from the app into the templates folder and New copies it.

## Quick Look and the preview pane

Select a file and press **Space** for Quick Look: a large preview over the window. **↑**/**↓**
move to the next file while it stays open, and **Space** or **Escape** closes it. **Open** opens
the file in its usual app.

**👁** on the toolbar, or **Alt+P**, shows the preview pane at the right of the window. It shows
the selected item with its details: kind, size, dates and, for images, the dimensions and pixel
format. Drag its edge to make it wider.

| Kind | What you see |
|---|---|
| Images | PNG, JPEG, GIF, WebP, TIFF, BMP, TGA, ICO, QOI, PNM, and HDR and OpenEXR (shown with a standard display curve). Transparent areas show a checkerboard. |
| Text | Text and source files, the first 256 KB. |
| Other files | The file's details, and **Open**. |

On an image, scroll to zoom towards the pointer, drag to pan, and double-click to switch between
fitting the image and showing it at 1:1.

Previews are made in the background, so a big file never freezes the window. To keep memory in
check, dirmap shrinks large images to 2048 pixels across for display, and doesn't preview images
over 256 MB.

## The analysis dock

The analysis dock shows the treemap (or the sunburst) of the folder you're in, under the list.
You can see where the space goes while you browse, without switching to Storage Analysis.

- **Open it** with **⊞ Analysis** on the toolbar, **View > Analysis Dock**, or **Ctrl+J** (**⌘J**
  on a Mac).
- **Resize it** by dragging the line above it. The list and the dock each keep at least a few
  rows.
- **🗖** makes the dock as tall as possible, and **🗗** puts it back.
- **⛶** opens the same folder in the full Storage Analysis workspace.
- **✖** closes it.

dirmap remembers whether the dock is open, and its height.

![The analysis dock under the list](../images/file-manager-dock.png)

### Analysing a folder

The dock never scans on its own. If the folder hasn't been analysed, it says so and offers
**⚡ Analyse This Folder**. The scan runs in the background, and the dock shows its progress:

- files and folders found;
- the bytes counted and the time taken;
- the folder being read;
- any it couldn't read.

You can keep browsing, in this tab or others, and **■ Cancel Scan** stops it. The percentage is
shown only for whole drives, where the used space gives a real total.

When it's done, the treemap appears. From then on, every folder inside the analysed one shows
its treemap straight away, with no rescanning. If you analysed a whole drive, that's every
folder on it.

If a folder is already analysed, the button says **⟳ Refresh Analysis**. That scans only this
folder again and puts the result into the existing analysis, so a whole-drive analysis is kept.

dirmap also keeps the last two analyses you replaced (if each is under 2 million files). Going
back to a folder one of them covers shows it again at once.

### What the map shows as you browse

Choose with the menu on the dock's toolbar:

| Mode | The map shows |
|---|---|
| **Follow folder** (the usual) | The folder open in the list. Switching tabs switches the map too. |
| **Pinned** | The folder that was on the map when you chose Pinned, wherever you browse. **📌** shows which. |
| **Follow selection** | The folder selected in the list, without opening it. With a file selected, the open folder, with the file outlined. |

Moving between a folder and its subfolders zooms the map in or out, so you can see where you
went.

### The list and the map together

- **Selecting in the list** outlines the item on the map. With several selected, each one is
  outlined.
- **Clicking a tile** selects it, and selects the item in the list that holds it.
- **Double-clicking a tile** opens its folder in the list.
- **Right-clicking a tile** gives the same menu as the list, plus **Open in List** or **Show in
  List**.
- **⌖** zooms to the selection, and **⟲** goes back to the whole folder.

The wheel, dragging, and the **+**, **−** and **Home** keys work as in
[Storage Analysis](treemap-navigation.md) while the pointer is over the map.

### The information line and the legend

The line above the map shows:

- **Analysed**, **Changed since analysis**, **Analysed with filters**, **Scanning** or **Not
  analysed**;
- the folder's size, files and folders;
- its largest subfolder and largest file;
- when it was analysed.

**Changed since analysis** means the list doesn't match the analysis: something analysed has
gone, a file's size has changed, or something was modified after the analysis. **⟳ Refresh
Analysis** brings it up to date.

Under the map, the legend lists the file types taking the most space in the folder, in their map
colours. Click one to highlight those files on the map, and click it again to stop. **Colour**
on the dock's toolbar switches to colouring by age, or by change since a snapshot.

## Copying, moving and deleting

- **Copy and paste.** Select items, press **Ctrl+C** (or **Ctrl+X** to move them), open another
  folder and press **Ctrl+V**. The clipboard is shared with Storage Analysis.
- **Drag and drop.** Drag items onto a folder in the list, a tab, a part of the path, the other
  pane, a drive or a favourite. Hold the items over a tab for a moment to switch to it. Within the
  same drive a drop moves the items, and onto another drive it copies them, as in Explorer and
  Finder. Hold **Ctrl** or **Alt** while you let go to copy, or **Shift** to move (on a Mac,
  **⌥** copies and **⌘** moves). The label by the pointer always says which will happen, and a
  folder can't be dropped inside itself.
- **Rename** with **F2**, and **Delete** to send items to the Recycle Bin or Trash. dirmap asks
  first.

Copies and moves run in the background with a progress bar in the status bar, and the open
folders are read again when they finish. One file operation runs at a time.

## Slow and unreachable drives

Folders are read in the background, a batch at a time, so a folder of 100,000 files starts
showing at once, and the window never freezes. If a network share or a sleeping disk takes more
than two seconds to answer, the list says so and offers **Cancel**. You can use other tabs,
drives and Storage Analysis while you wait.

If a folder can't be opened, the list says why, for example because it doesn't exist or you
don't have permission, and offers **Go up**.

![The Computer view, with every drive and its space](../images/file-manager-computer.png)

## Network & Remote

The **Network & Remote** group in the sidebar lists:

- **network drives** that are connected, with how full they are;
- **connections the system remembers**: mapped drives on Windows, including ones that are offline
  (marked *offline*; click to reconnect), and network mounts made by your desktop on Linux;
- **saved servers** you added.

Right-click a network drive for **Disconnect**, and a saved server for **Remove from This List**.

**Connect to Server…** opens a box for a share address:

- **Windows:** type `\\server\share` (or `smb://server/share`), choose a drive letter or leave it
  empty for the last free one, and tick **Reconnect when I sign in** to keep it mapped.
- **macOS:** type `smb://server/share`. It's mounted in `/Volumes`.
- **Linux:** type `smb://server/share`. It's mounted through GVFS, as your desktop's file
  manager does.

If the share needs a password, the system asks for it and can keep it in Credential Manager, the
Keychain or your keyring. dirmap never sees or stores passwords. Connecting happens in the
background, so a server that doesn't answer shows an error instead of freezing the window.

SFTP, FTP and WebDAV connections aren't built in yet. You can mount them with your system and
open the folder in dirmap.

## Related topics

- [The file manager in Storage Analysis](file-browser.md)
- [Scanning a drive or folder](../getting-started/scanning.md)
- [Keyboard shortcuts](../reference/keyboard.md)
- [External and network drives](../advanced/network-drives.md)
