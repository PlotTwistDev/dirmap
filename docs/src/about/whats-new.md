# What's new

## Next version

### The AI Models workspace

- **🔬 Lab** shows what a model would write next and how sure it is: each token coloured by its
  probability, the ten likeliest alternatives, and what temperature, top-k, top-p and min-p do to
  them. Pick a different token to see where the text goes instead. **Inside** shows how much each
  of the model's layers changes each word, and what each layer would predict if the model stopped
  there. **Speed** tests how fast a
  model reads and writes on this computer, and keeps the results to compare. Free. See
  [The Lab](../views/model-lab.md).
- **💬 Ask AI** chats with a model already on this computer, run by Filescape itself: no Ollama,
  LM Studio or internet connection needed, and nothing you ask leaves the computer. It runs GGUF
  models, on the graphics chip of a Mac with Apple silicon and on the processor elsewhere. Ask AI
  is free. See [Ask AI](../views/ask-ai.md).
- **Ask AI About This…** in the right-click menus of Files and Storage Analysis asks about a file or
  folder: its size, dates and what it holds. Reading what's inside files is a tick box for each
  question. See [Asking about files and folders](../views/ask-ai.md#asking-about-files-and-folders).
- No model yet? **Get a small model…** in Ask AI downloads Qwen3 1.7B (1.7 GB) from Qwen's own
  page, after showing you its size, licence and the memory it needs. Filescape checks the file
  before using it, and a stopped download carries on where it left off.

- A new **✨ AI Models** workspace (**Ctrl+4**) lists the AI models on this computer: what Ollama,
  LM Studio and Hugging Face have downloaded, model files in Downloads and in folders you add, and
  those in the Storage Analysis scan. Each shows its size, format, family, parameters, precision
  and context length, read from the file without loading it. See
  [The AI Models workspace](../views/ai-models.md).
- Files that are byte-for-byte copies of another model are marked, as are Ollama downloads no
  installed model uses.
- `filescape --cli --models PATH` prints the same list.
- Double-click a model to inspect it: its architecture, layers, heads, vocabulary, licence and
  parameters, each tagged with where it comes from (the file, Filescape's own count, or the
  publisher's notes), how its numbers are stored, and every tensor and metadata entry, searchable
  and sortable. PyTorch checkpoints are never opened, since they can run code. See
  [Inspecting a model](../views/ai-models.md#inspecting-a-model).
- A model's **Health** finds missing parts, files cut short by a stopped download, damaged
  files, unsafe formats and size labels that don't add up, and can verify Ollama and Hugging Face
  downloads against their checksums.
- **Will it run?** estimates the memory a model needs at any context length and says whether it
  runs on this computer's graphics chip, split, on the processor, or not at all.
- Click a tensor to see a histogram and statistics of its numbers, and **Check the numbers** in
  Health samples every tensor for broken values or missing data.
- A model's **Tokens** show its vocabulary: which languages and kinds of text it covers, its
  special tokens and chat template, and every token, searchable.
- **Compare with…** puts two models side by side: their facts, where their space goes, how their
  numbers are stored, and which tensors and metadata differ, so you can tell two compressions of
  one model from two different models.
- A model's **Structure** shows its layers in order and draws the model as a map of its layers
  and tensors, coloured by what each part does or by how its numbers are stored.

### See what's in files on the map

- Zoom in on the treemap and tiles big enough show what's in the file: pictures, video frames,
  a PDF's first page, a sound's waveform, and the first lines of text, code and Word or
  OpenDocument documents. Rest the pointer on a video tile and it plays there, silently.
  They load in the background and never download cloud files. See
  [Previews on tiles](../views/treemap.md#previews-on-tiles).
- Storage Analysis has a preview pane beside the map: click a tile to see its preview and details
  (bytes, size on disk, share of the analysis, how a cloud file is stored, and a folder's largest
  items). **Alt+P** shows or hides it. Videos play right in the preview panes, and on macOS
  sound now pauses at once and can jump to any point. Word and OpenDocument files now preview as text in both
  workspaces.
- Markdown files preview formatted, with headings, lists, tables and links. **Raw** shows the
  text as written, and the preview panes remember which one you picked.
- Right-click any part of the path above the file list, or a tab, and choose **Copy Path**.
- SVG drawings now preview as pictures, on map tiles, in icon views and in the preview panes,
  sharp at any zoom, with **Source** to see the markup.
- Code previews in syntax colours, for about 200 languages, and the details say which language.
- Fonts preview as a specimen drawn in the font, with its family, style and glyph count; on
  tiles and in icon views they show a large "Aa".
- Sound previews show the album cover and the waveform, which fills in as it plays and jumps
  where you click, and the details list the title, artist, album, length and format.
- Camera RAW photos (CR2, CR3, NEF, ARW, DNG, RAF, ORF, RW2 and more) preview on tiles, in icon
  views and in the panes, from the preview the camera stores in the file, with the camera, lens
  and exposure in the details.
- HEIC (iPhone) and AVIF pictures preview through the system's own decoders.
- Photoshop documents (PSD and PSB) preview from the flattened picture saved with them.
- Spreadsheets (Excel and OpenDocument) preview as a table of their cells, sheet by sheet, and
  their first rows show on map tiles. Formulas and macros are never run.
- Word, PowerPoint and OpenDocument files preview formatted: headings, bold and italic, lists and
  tables, and each slide's title and points. Icon views show the picture of the first page or
  slide saved with them.
- EPUB books preview as formatted text, chapter by chapter, with the title and author in the
  details and the cover in icon views.
- JPEG XL pictures (.jxl) preview everywhere pictures do.
- SQLite databases preview as their tables, read-only, and CSV and TSV files as a table with
  their column names.
- Archives preview as a tree of folders, biggest first, with sizes, how much compression saved,
  and warnings about password-protected files, unsafe names and archives that unpack to a huge size.
- Pictures can be looked at channel by channel, HDR and OpenEXR pictures have an exposure
  slider, and multilayer OpenEXR files show each layer (render pass).
- Videos have a filmstrip under the picture (click a frame to play from there), and a video that
  starts black or with a title card is shown by a more telling frame.
- 3D models (STL, OBJ, PLY, glTF/GLB and 3MF) preview as a shaded model to turn about, and show
  in Icons view.
- Thumbnails are kept on disk between runs, so folders you've seen show their pictures at once.
  A new **Previews & thumbnails** page in Settings has presets, the cache's size and **Clear**,
  and switches for filmstrips, video pictures and 3D and MIDI thumbnails.
- Emails (.eml, .emlx) preview with the sender, recipients, date, text and a list of
  attachments. HTML mail is shown as plain text, so nothing in it is loaded (no tracking
  pictures). Mailboxes (.mbox) list their messages, contact cards (.vcf) show the contact or a
  list of contacts, and calendars (.ics) show the event or a list of events and to-dos.
- Map files preview as a drawing of their shapes: GPS tracks (.gpx) with the distance, climb,
  height, time and average speed; GeoJSON with its features and properties; and KML and KMZ
  (Google Earth) with their placemarks. No map is drawn behind them, since that would mean
  fetching it from the internet.
- DXF drawings preview as the drawing itself (lines, curves, circles and blocks, in their
  colours), with the AutoCAD version, units, layers and overall size. DWG, STEP and IGES files
  show what they say about themselves: the program that made them, the units, the author and the
  parts.
- Pages, Numbers and Keynote documents, and files from Sketch, Affinity, CorelDRAW, SketchUp,
  Blender, Cinema 4D and 3ds Max, preview from the picture the app saved inside them, with the
  version that saved them. Maya and Houdini scenes show their version, units and what's in them.
- Files from the Craft Apps (PhotoCraft, VectorCraft, EffectCraft, FigCraft, DesignCraft,
  TextureCraft, DeckCraft, AniCraft, MaterialCraft, FilmCraft and SoundCraft) preview from the
  picture or PDF the app saved inside them, with their pages, layers, size, length and the version
  that saved them. Filescape reads them itself, without the apps.
- Music projects, and FilmCraft, SoundCraft and EffectCraft projects, list the footage and sound
  files they use: where each one is (this drive, another drive, a network drive or the cloud), which
  are missing or only in the cloud, and their size together. Click one to show it in Files.
- After Effects projects (.aep, .aepx) show the version that saved them, their compositions and
  the footage files they use.
- LightCraft library folders preview like a file: their photos and albums, what the catalogue,
  thumbnail cache and imported photos take, and the photos used from other folders. PhotoCraft
  documents saved as a folder preview like the single-file ones.
- 12-bit and arithmetic-coded JPEGs now show, as do DICOM scans compressed with 12-bit JPEG.
- Photoshop documents list their layers: groups, hidden layers, text layers, smart objects and
  adjustments, and the names at the top of the stack.
- Krita, OpenRaster and Figma files show the picture saved inside them, with their layers. GIMP
  and Paint.NET pictures are drawn from their layers.
- FBX, COLLADA (.dae) and Alembic (.abc) models show as shaded clay to turn about, like other 3D
  models. AutoCAD DWG drawings are drawn, converted to DXF in a separate process first.
- Music projects from Ableton Live (.als), REAPER (.rpp), FL Studio (.flp) and Cubase or Nuendo
  (.cpr, .npr) show the version that saved them, the tempo, tracks, plugins and the sound files
  they use, marking any that aren't on this computer.
- DICOM medical scans and FITS astronomy images preview as pictures, with what they show
  and how they were taken. Patient names, IDs and birth dates are never shown.
- Disk images preview without being mounted: an ISO shows the files on the disc, virtual
  machine disks show their size and type, and Windows images list their editions.
- Disc images that use UDF (DVDs, Blu-ray discs and Windows install discs) list their files
  too.
- JPEG 2000 pictures (.jp2, .j2k) preview, and DICOM scans compressed with JPEG 2000,
  JPEG-LS or lossless JPEG now show their picture.
- Logic Pro projects preview with Logic's picture of the project window, tempo, key and
  sound files.
- **Help › Supported Formats** lists every kind of file Filescape previews, what the preview
  shows and what reads it.
- MIDI files preview as a piano roll, with the length, tempo, tracks and instruments in the
  details.
- Quick Look works in Storage Analysis too: click a tile and press **Space**. **Full screen** (or
  **F**) fills the window with the preview, in both workspaces.
- PDFs preview every page, drawn by Filescape itself: scroll through them in the preview panes,
  with the title, page count and paper size in the details. Thumbnails and map tiles of PDFs are
  drawn the same way, so they no longer need a PDF tool installed on Linux.
- Files with a missing or wrong extension, such as a download saved as `file`, now preview by
  what they really are when Filescape can tell from their first bytes.
- **Help > About Filescape > Third-party licences** lists the open-source software Filescape is
  built with and the full text of each licence.

### What system files and folders are

- Filescape explains the files and folders you didn't make and whose names say little:
  `pagefile.sys`, `hiberfil.sys`, `WinSxS`, `System Volume Information`, `.Spotlight-V100`, the
  sleep image, `/proc`, the systemd journal, snap and Flatpak, and about 170 more on Windows,
  macOS, Linux and removable drives. It says what each one is, whether it's safe to delete, and
  how to make it smaller safely. The guide is built in and works offline.
- An info icon after the name in the file manager, the Storage Analysis list and the tree (hover
  it), a *What is this?* box in the preview pane, a note in the status bar over the map, and
  **What Is This?** in the right-click menus. See
  [What system files and folders are](../disk-usage/system-files.md).
- Free in every edition.

### Freeing up cloud space

- **Free Up Space** and **Always Keep on This Device** (Windows), or **Remove Download** and
  **Download Now** (macOS), in the right-click menu of cloud files and folders in the file
  manager, the Storage Analysis list and the tree. Filescape asks the system, which asks your
  cloud app: nothing is deleted, files stay in the cloud, and files with changes not yet
  uploaded are left alone. Keeping files that aren't downloaded asks first, saying how much would
  download. See [Freeing up space](../disk-usage/cloud-files.md#freeing-up-space-and-keeping-files-here).
- A **Reclaimable** size on the **Size** switch maps the space freeing up would give back, and
  the storage breakdown says how much that is. Click it to see where.
- On the command line, `--free-up PATH` and `--keep PATH` do the same (with Filescape Professional), and `--reclaimable` ranks a
  report by reclaimable space.
- One file at a time is free; folders, several items at once and the Reclaimable view are part of
  Filescape Professional.

### Cloud storage awareness

- Filescape recognises cloud folders and drives: Google Drive, OneDrive, SharePoint, iCloud
  Drive, Dropbox, Box, Nextcloud, ownCloud, pCloud, MEGA, Synology Drive, and drives mounted with
  rclone or other FUSE tools. Nothing is installed and no provider is contacted: Filescape looks
  where these services keep their folders. See [Cloud files](../disk-usage/cloud-files.md).
- A status icon after the name of each cloud file says whether it's **online only**, **partly
  downloaded**, **downloaded**, **always kept on this device** or **unknown**; hover it for what
  that means and which service it belongs to. Shown in the file manager, the Storage Analysis
  list and the tree.
- A **storage breakdown** under the map splits a scan into files not in the cloud, downloaded
  cloud files and data only in the cloud, with the share that is on this computer.
- A third size, **Cloud only**, on the **Size** switch maps where the data that isn't on this
  computer is.
- **Stored** filter chips show only online-only, downloaded, pinned or local files (Professional),
  and the Storage Analysis list gains **Stored** and other-size columns for cloud scans.
- A new **Cloud storage** page in Settings, with the default size, the icons and breakdown, asking
  before downloads, and whether to walk into mounted cloud drives (off by default).
- On Windows, OneDrive and other cloud placeholders no longer show as links in the file manager.

### Size on disk, and cloud files that stay in the cloud

- Filescape now reads two sizes for every file: its **size on disk** (what it really takes on
  this computer, as `du` and Explorer's "Size on disk" count it) and its **logical size**. The new
  **Size** switch above the map changes between them at once, without rescanning; everything
  follows it: the map, sunburst, folder list, File types, size histogram, summary, filters and
  the file manager's Size column. **On disk** is the default.
- Cloud folders (Google Drive, OneDrive, iCloud Drive, Dropbox, Box…) now show what they use
  on this computer: files that are only online take almost no space, instead of their full size.
  Hover an item to see both sizes and whether it's online only. See
  [Cloud files](../disk-usage/cloud-files.md).
- Filescape no longer downloads online-only files behind your back: no thumbnails for them,
  the preview asks first, **Find duplicates** skips them, **Compare contents** compares them by
  size and date, and the sequence player asks before playing frames that aren't downloaded.
- On macOS and Linux, files with several names (hard links) count once in sizes on disk.
- Snapshots now record both sizes. Snapshots saved by earlier versions still open; they're
  compared by logical size.
- The command-line report (`--cli`) prints both totals and counts online-only files; add
  `--logical` to rank by logical size.

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

- Under the Storage Analysis map, a summary shows the scanned folder, its size and file count,
  and the file types taking the most space, each in its map colour. It follows the filters, so
  with **Larger than** or **Smaller than** set you see how much space the matching files take
  and of which types. Click a type to highlight it. See
  [The summary under the map](../views/treemap.md#the-summary-under-the-map).
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
