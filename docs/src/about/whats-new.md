# What's new

## Next version

### A redesigned Settings window

- Settings has a sidebar of pages with icons, and opens on the page you used last. Each page has
  a title, a line on what it's for, and its settings in grouped sections, each with a sentence on
  what it does. The window can be resized. See [Settings reference](../settings/reference.md).
- **Find a setting…** (or **Ctrl+F**) searches every setting by name or by everyday words such as
  "zoom", "threads", "hidden files" or "licence", and takes you to it.
- On/off settings are switches, choices are segmented buttons, and sliders show their value (drag
  or type it) with a reset button once changed.
- Pictures show what you're choosing: the themes, a folder list preview that follows the columns
  you pick, and a small treemap drawn with the current treemap settings, with the tile styles and
  presets as thumbnails. The Classic cushion settings only show when that style is chosen.
- **General** has a new **Interface size** (80% to 150%, which **Ctrl +** and **Ctrl −** also
  change and which is now remembered), an example for the size units, and an **Updates** section
  with your version, the last successful check, **Check now** and release notes.
- **Filtering** keeps rules as lists you add to, edit and remove from, with **Browse…** for
  folders, warnings for rules that can't work, common patterns to add with one click, a name
  tester, sizes in any unit and ages in days, weeks, months or years. A summary at the top says
  what's skipped and when a rescan is needed.
- **Advanced** has scanner thread profiles (Automatic, Low, Balanced, High, Network, Custom), a
  Permissions section that gives the right advice for Windows, macOS and Linux, and Diagnostics
  with the settings folder and system details to copy.
- **Licensing** shows your edition and state at a glance, a bar for the evaluation's days left,
  a **Paste** button for the licence key, and makes clear that a perpetual licence never expires
  even after its updates end.
- The footer has **Restore page defaults** and **Restore all…** (which asks first). **Apply** is
  greyed out until something changes, and **Cancel** or **Esc** undoes everything not applied.

### Update notices

- Filescape checks for a newer version each time it starts and, when there is one, says so at
  the top of the window, with a download link and whether your licence covers it. See
  [Updates](../settings/licensing.md#updates). Turn it off in **Settings > General**.
- **❓ Help > Contact Filescape…** (and **Contact Us…** in **About Filescape**) opens the contact
  form on filescape.app. See [Still stuck? Contact us](../troubleshooting/index.md#still-stuck-contact-us).

### A new look for the treemap

- The treemap has a new **Modern** tile style, the same look as the map on filescape.app: soft
  pastel tiles, names and sizes written on the tiles, and the folders on screen outlined with dark
  name tags. The cushion look is still there as **Classic cushion** under
  **Settings > Treemap > Tile style**.
- Filescape has its own icon, the tile-grid logo from the website, in the window, the taskbar and
  the Dock.

### Files workspace

- Filescape now opens as a **file manager**. Browse your drives, folders and network shares straight
  away, with no scan. It has:
  - a sidebar with Home, Desktop, Documents, Downloads and the rest, favourites, drives with
    their space, and recent folders;
  - tabs that are restored next time;
  - Back, Forward and Up;
  - a path you can click or type in (**Ctrl+L**), including `\\server\share` paths;
  - a sortable Details list with a filter box and type-to-select;
  - Shift and Ctrl selection.

  See [The Files workspace](../views/file-manager.md).
- **A full file manager.** Copy, cut and paste (**Ctrl+C**, **Ctrl+X**, **Ctrl+V**), drag and
  drop, **F2** to rename and **Delete** to send to the Recycle Bin or Trash now work in Files.
  Drops move items on the same drive and copy them to another, as in Explorer and Finder, and the
  label by the pointer says which. See
  [Copying, moving and deleting](../views/file-manager.md#copying-moving-and-deleting).
- **Undo and name clashes.** **Ctrl+Z** undoes copies, moves, renames, new items and Move to
  Trash, putting deleted items back from the bin. When a copy or move meets a name that's taken,
  Filescape asks: Replace, Keep Both, Skip, or Merge for folders. See
  [Undo](../views/file-manager.md#undo).
- **Servers: SFTP, FTP and WebDAV.** Browse, copy to and from, rename and delete on servers as if
  they were drives, and analyse their folders. **Connect to Server…** saves them in the sidebar.
  SFTP checks the server's identity and signs in with your SSH agent, key files or a password;
  passwords are only ever kept in the system keychain, and only if you ask. FTP and plain-http
  WebDAV are marked as not encrypted. See
  [Servers: SFTP, FTP and WebDAV](../views/file-manager.md#servers-sftp-ftp-and-webdav).
- **Image sequences and a sequence player.** Numbered frames (`shot.1001.exr`, `shot.1002.exr`…)
  show as one row with their range and any missing frames, and copy, move, delete and drag as one
  item. Double-click one to play it from memory, with a timeline, loop, ping-pong, frame rates
  from 23.976 to 60, exposure and colour transforms (sRGB, Rec.709, Cineon log). Select two and
  press **Enter** to compare them with a wipe. EXR, DPX, PNG, TIFF and the other image types all
  play. See [Image sequences](../views/file-manager.md#image-sequences).
- **Two panes** (**Ctrl+\\**): two folders side by side or one above the other, each with its own
  tabs, sort and filter. Drag between them, or press **Shift+F5** to copy and **Shift+F6** to move
  to the other one. See [Two panes](../views/file-manager.md#two-panes).
- **Favourites.** Drag a folder onto the sidebar to bookmark it; nothing is moved. Reorder them by
  dragging, rename them, and sort them into groups. Pinned folders become favourites. See
  [Favourites](../views/file-manager.md#favourites).
- **Quick Look and a preview pane.** Press **Space** for a large preview, or **Alt+P** for a
  preview pane, with zoom and pan. Images (including OpenEXR and HDR) and text files. See
  [Quick Look and the preview pane](../views/file-manager.md#quick-look-and-the-preview-pane).
- **Four views.** Details, Icons, Columns (the folders above this one at the left, as in Finder)
  and Gallery (a large preview over a strip of pictures), from the buttons at the bottom right of
  the list. Icons and Gallery show pictures of images, videos and PDFs. Each folder remembers its
  own view, sort and grouping.
- **Groups.** **View > Group By** puts the Details list under headings by type, date or size.
- **Rename many items at once.** Select several items, or a sequence, and press **F2**: find and
  replace (with regular expressions), change case, add numbers or change the extension, with every
  new name shown before anything changes. See
  [Renaming many items](../views/file-manager.md#renaming-many-items).
- **Compare and sync folders.** With two panes, **View > Compare Folders…** shows what's only on
  one side, newer on one side or different, then copies one way, both ways or mirrors the left
  onto the right. Replaced and deleted items go to the bin, and Undo reverses a sync. See
  [Comparing and syncing folders](../views/file-manager.md#comparing-and-syncing-folders).
- **Search subfolders.** **🗐** beside the filter searches the folder and every folder below it,
  and **☆** keeps the search in the sidebar. See
  [Searching subfolders](../views/file-manager.md#searching-subfolders).
- **Colours, tags and notes.** Give items a colour, tags and a note from the right-click menu.
  The sidebar's **Tags** shows everything with a tag. See
  [Colours, tags and notes](../views/file-manager.md#colours-tags-and-notes).
- **Open With your own apps.** Add the programs you use for each type of file (Nuke, RV, a text
  editor) and open items in them from the right-click menu. See
  [Open With](../views/file-manager.md#open-with).
- **Redo** (**Ctrl+Y** or **Ctrl+Shift+Z**) after an Undo, and **F6** to move the keyboard to the
  sidebar and back.
- **LUTs in the sequence player.** **LUT…** shows frames through a `.cube` file. See
  [Playing a sequence](../views/file-manager.md#playing-a-sequence).
- **Archives.** Open zip, 7z and tar archives like folders, preview what's in them, and take
  things out with **Extract Here**, **Extract To…**, copy and paste or a drag.
- **Pause and queue.** Copies and moves show their progress in the status bar, can be paused,
  resumed and stopped, and ones started while another runs wait their turn.
- **Changes made by other programs** show up in the list by themselves, without **F5**.
- **Video, PDF and sound previews.** A frame from a video and a PDF's first page from the
  system's thumbnailers, and sound files play in the preview.
- **A smarter address bar.** Suggestions as you type, **Tab** completion, `%VAR%` and `$VAR`,
  `>` for commands and `@` for favourites. Each part of the path takes drops and has a menu of
  the folders inside it. See [Typing a path](../views/file-manager.md#typing-a-path).
- **New menu.** New folders and small valid starter files (text, Markdown, JSON, XML, CSV,
  scripts), plus your own templates: put any file or folder in the templates folder and New
  copies it. See [New folders and files](../views/file-manager.md#new-folders-and-files).
- **Network & Remote.** Mapped drives (including offline ones), network mounts, saved servers and
  **Connect to Server…** for SMB shares. The system asks for passwords; Filescape never sees them.
  See [Network & Remote](../views/file-manager.md#network--remote).
- **The analysis dock** (**Ctrl+J**) shows the treemap or sunburst of the folder you're in, under
  the list. Once a folder or drive is analysed, every folder inside it shows its treemap
  straight away as you browse, with no rescanning.
  - **Analyse Folder** scans in the background while you keep browsing.
  - **Refresh Analysis** rescans just one folder inside a bigger analysis.
  - The dock can follow the open folder, follow the selected folder, or stay pinned to one
    folder.
  - Selecting in the list outlines the item on the map, and clicking a tile selects it in the
    list. Tiles have the same right-click menu as the list.
  - A line above the map shows the folder's size, file and folder counts, its largest file and
    subfolder, and whether it has changed since it was analysed. The legend under the map
    highlights a file type when you click it.

  See [The analysis dock](../views/file-manager.md#the-analysis-dock).
- **Storage Analysis** is still the Filescape you know, with everything in it. **Ctrl+1** and
  **Ctrl+3** switch between the two workspaces, and each keeps your place.
- **A new look.** It has:
  - calmer panel colours in both themes, and one accent colour;
  - grouped toolbar buttons;
  - proper tabs, which you can drag to reorder;
  - resizable columns;
  - Comfortable or Compact rows (**View** menu);
  - a status bar that shows background work.
- Folder sizes from the last scan show in the Files list.
- Big folders and slow network drives never freeze the window: folders are read in the
  background, and a share that takes a while says so and can be cancelled.

### Linux distributions in the sidebar

- On Windows, your **WSL** distributions are listed under **Linux** in the sidebar, as in
  Explorer. Click one to browse its files.

### Windows' own right-click menu

- On Windows, **Show More Options** at the bottom of an item's right-click menu (or **Shift+F10**,
  or **Shift+right-click**) opens the menu Explorer shows, with what other apps add to it:
  7-Zip or PeaZip, Notepad++, PowerToys, antivirus scanners, **Send to** and the rest. See
  [Right-click menu](../views/file-manager.md#right-click-menu).

### Show in Files

- **Show** beside the filter in Files keeps only some kinds of file (Audio, Video, Images,
  Documents…), and with **🗐** gathers them from every subfolder. See
  [Showing only some kinds of file](../views/file-manager.md#showing-only-some-kinds-of-file).
- **The preview pane counts a selection** of several items by kind, with sizes. Click a kind to
  preview one of them.
- **Crashes are saved** to `crash.log` in Filescape's settings folder, and on Windows a message says
  what happened, so a crash can be reported.

### Fixes

- **Menus have icons**, in a column of their own so the names line up.
- **File names on map tiles are easier to read.** They sit on a dark backing and get bigger as
  you zoom in.
- **`pagefile.sys`, `swapfile.sys` and `hiberfil.sys` aren't counted as Programs** any more.
  They're Windows' memory saved to disk, and now count as Other.
- **Previews of UTF-16 text files**, such as PowerShell output, show the text instead of "No
  preview". **PDFs on Windows** show their first page without needing a PDF app's thumbnails.
- **The File Manager toolbar no longer overlaps itself on a narrow window.** Back, Forward, Up,
  the path and the filter now sit on the pane's own bar above the tabs, and on a narrow window
  the toolbar's buttons show only their symbols.
- **Resizing the window on Windows** no longer makes the panels shake. Filescape now draws with
  OpenGL there. `DIRMAP_RENDERER=wgpu` brings back DirectX 12.
- **Rename** no longer replaces a different file whose name differs only in capital letters
  (`readme` → `README`) on Linux and on case-sensitive Mac disks.

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

- **Help Centre** built into Filescape: the complete manual, tutorials, keyboard and mouse
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
- [About Filescape](about.md)
