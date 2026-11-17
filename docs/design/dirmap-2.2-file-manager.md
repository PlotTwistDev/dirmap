# dirmap 2.2: a file manager to use every day

Status: proposal, plan and progress record, 17 November 2026. It follows `dirmap-2.0.md` (the
Files workspace) and `dirmap-2.1-files-dock.md` (the redesign and the analysis dock). The brief
asks for the best practical features of the leading file managers and specialist tools,
without another visual redesign. Rich's priorities are split panes, dragging to Favourites,
network mounts and Quick Look first. The image sequence player comes later.

The research behind sections 2 and 3 draws on how these products work today: File Explorer
(Windows 11), Finder (macOS 15), Dolphin, GNOME Files, Directory Opus 13, Total Commander 11,
Double Commander, OneCommander, WinSCP 6, Cyberduck 9 and DJV 2/3. It is based on their
documented behaviour, not on new measurements.

## 1. Audit: what dirmap already has

The audit started from the code. Nothing was assumed missing.

| Area | In the code | Where | Gap |
|---|---|---|---|
| Browsing without a scan | Listings stream in batches on worker threads, with a slow-share notice and cancel | `fsys/listing.rs` | Local paths only, and no backend abstraction |
| Tabs and history | Per-tab Back and Forward, reopen a closed tab, drag to reorder | `fsys/history.rs`, `files.rs` | One pane only |
| Address bar | Breadcrumbs, Ctrl+L to type a path, `~` and `D:` | `files.rs::path_bar` | No suggestions, Tab completion, environment variables, crumb menus or drops |
| Sidebar | Quick access, Pinned (with move up and down), Drives with capacity bars, Recent | `files.rs::sidebar` | No drag to pin, labels, groups, or Network section |
| Details list | Sorting, resizable columns, density, hidden items, filter with `*` and `?`, scanned folder sizes | `files.rs::details_view` | No other views, grouping or per-folder preferences |
| File operations | Copy, cut, paste, move, rename, new folder, Recycle Bin or Trash, delete, SHA-256, properties, all in the background | `actions.rs`, `menus.rs` (`Ops`), `browser.rs` | **Only in Storage Analysis.** Files has open, reveal, terminal and copy path, but nothing that changes the disk |
| Drag and drop | Drag items onto folders and tabs to move them, Alt to copy, with a label by the pointer | `browser.rs` (`DragItems`, `drag_preview`) | Storage Analysis only |
| Other views | Icons grid, Finder-style columns, a file panel | `browser.rs` | Storage Analysis only, and they read the scan tree, not the disk |
| Preview | A large icon and facts about the file | `browser.rs::file_preview` | No image, text or media content |
| Sequences | Detects numbered frame sequences, with gaps, steps and padding | `sequences.rs` | Runs on the scan tree only, with no viewer |
| Duplicates | Size, then partial hash, then full hash, in the background | `dupes.rs` | Fine as is |
| Drives | Windows logical drives, including mapped network drives (marked), and Unix mounts flagged nfs, cifs or sshfs | `drives.rs` | Disconnected mapped drives are skipped, with no connect or disconnect |
| Analysis | Scan, treemap, sunburst, dock with follow modes and cached analyses | `app.rs`, `dock.rs`, `mapview.rs` | Follows the single pane |
| Settings | eframe storage key `files` (`FilesPrefs`) and `Settings` | `files.rs`, `settings.rs` | Fine; new prefs go in `FilesPrefs` with `serde(default)` |

**Conclusion.** The biggest single gap is that the Files workspace can't change anything on
disk, although the code to do it exists. Split panes, dragging between panes, New from
template and synchronisation all depend on that. Wiring `Ops` into Files comes first.

## 2. Feature gap analysis

Key: ● has it, ◐ partly, ○ no. "dirmap" is before this work.

| Capability | Explorer | Finder | Dolphin | Opus | WinSCP | dirmap |
|---|---|---|---|---|---|---|
| Breadcrumbs, editable path | ● | ◐ (Go to Folder) | ● | ● | ● | ● |
| Path autocomplete and Tab completion | ● | ● (Go to Folder) | ● | ● | ◐ | ○ |
| Environment variables in the path | ● | ○ | ◐ | ● | ○ | ○ |
| Drop onto breadcrumbs | ● | ● (path bar) | ● | ● | ○ | ○ |
| Favourites by dragging, with groups | ◐ (Quick access, no groups) | ● (no groups) | ● (Places groups) | ● | ◐ (bookmarks) | ◐ (pin by menu) |
| Dual pane | ○ | ○ | ● (F3) | ● | ● | ○ |
| Drag between panes and tabs, hover-open tabs | ● | ● | ● | ● | ● | ◐ |
| Copy, move, rename, delete in the browser | ● | ● | ● | ● | ● | ◐ (Analysis only) |
| New file from templates | ● (ShellNew) | ○ | ● (templates) | ● | ○ | ○ |
| Icons, thumbnails, columns, gallery | ● | ● | ● | ● | ◐ | ◐ (Analysis only) |
| Grouping | ● | ● | ● | ● | ○ | ○ |
| Preview pane | ● | ● | ● (info panel) | ● (viewer pane) | ○ | ◐ |
| Quick Look on Space | ○ | ● | ◐ | ● | ○ | ○ |
| Mapped drives, connect and disconnect | ● | ● (Connect to Server) | ● (KIO) | ● | ○ | ◐ (list only) |
| SFTP, FTP(S), WebDAV client | ○ | ◐ (WebDAV, SMB) | ● (KIO) | ● (plugin) | ● | ○ |
| Transfer queue, resume, conflict rules | ◐ | ◐ | ◐ | ● | ● | ○ |
| Folder compare and sync | ○ | ○ | ○ | ● | ● | ○ |
| Batch rename | ○ | ◐ | ● | ● | ○ | ○ |
| Folder colours and tags | ○ | ● (tags) | ● (tags, colours) | ● | ○ | ○ |
| Disk usage map in the browser | ○ | ○ | ◐ (Filelight) | ◐ | ○ | ● |
| Image sequence playback | ○ | ○ | ○ | ○ | ○ | ○ |

**What each does best and where it falls short.**

- **Explorer.** Familiar shortcuts, ShellNew templates and the Preview pane. It has no dual
  pane, and big network folders stall the window.
- **Finder.** Quick Look and column view are the reference. Dragging to the sidebar makes a
  favourite, and dropping on a sidebar item moves files into it. Typing a path is hidden
  behind ⇧⌘G.
- **Dolphin.** Split view (F3), Places groups, and a info panel. Network access through KIO.
- **Directory Opus.** The most complete: dual pane, folder formats, a viewer pane, sync,
  batch rename, labels. It is dense, with a steep learning curve, and Windows-only.
- **Total Commander and Double Commander.** Fast keyboard dual pane, with F5 and F6 to copy and
  move to the other pane. Their keys clash with OS conventions (F5 is refresh elsewhere).
- **OneCommander.** A modern look: columns plus dual pane, colour tags, a preview.
- **WinSCP and Cyberduck.** Saved sessions, host-key checks, a transfer queue with resume, and
  conflict rules (overwrite, newer, rename, skip). Credentials go to the OS keychain.
  Their local browsing is weak.
- **DJV.** RAM-cached image sequence playback with a timeline, a cache bar, loop and ping-pong,
  OpenColorIO, and A/B compare.

**Where dirmap can do better.** dirmap combines a modern dual-pane browser with the treemap.
Space and network status stay visible, and remote and slow storage never freezes the window.
The image sequence player is the professional feature nobody else ships in a file manager.

**What to leave out.**

- Shell-extension menus (unsafe and Windows-only).
- Running arbitrary shell commands from the address bar.
- Changing OS folder icons by default.
- FTP as a silent fallback for SFTP.

## 3. Prioritised feature matrix

| Tier | Features |
|---|---|
| **Essential** (daily use) | File operations in Files (copy, cut, paste, rename, delete, New) · drag and drop everywhere with visible Move or Copy · Favourites by drag, with labels and groups · split panes with independent tabs · address bar suggestions, Tab completion, variables and crumb drops · Quick Look and preview pane for images and text · Network & Remote section with mapped drives and network mounts · Icons view |
| **Advanced** | Thumbnails and gallery · column view in Files · grouping · per-folder view memory · folder compare and preview of sync · batch rename · New from user templates · command palette (`>`) · SFTP connection manager with keychain and host keys · transfer queue with resume and conflict rules · copy UNC or relative path · multi-file properties · search with scope (filter, live search, analysed data) |
| **Specialist** | Image sequence grouping and RAM player with cache, timeline and OCIO · EXR and DPX decode · A/B compare · folder colours, tags and notes · application launcher and Open With favourites · FTP(S), WebDAV · remote analysis · saved searches |

## 4. UX integration: where each feature lives

The 2.1 layout stays. New features go into places that already exist, or appear only when
they are used.

- **Toolbar.** Unchanged groups. A split-view toggle joins the View group (Ctrl+\\). The New
  menu (✚ New) sits beside the navigation group, because creating things is a primary action.
  The address bar is the one path field. It always works on the **focused pane**.
- **Address bar.**
  - Breadcrumbs as now. Each crumb has a menu (Open in New Tab, Open in Other Pane, Copy
    Path, Add to Favourites, Analyse) and accepts drops.
  - Click, Ctrl+L, Alt+D or F4 turns it into a text field with a suggestion list:
    sub-folders of what's typed, then recent and favourite folders.
  - Tab completes, and Up and Down choose a suggestion.
  - `%VAR%`, `$VAR` and `~` expand.
  - Typing `>` lists dirmap commands, and `@` searches favourites.
    Both are optional, and the field's hint names them.
- **Sidebar.**
  - Sections in order: Quick access, **Favourites** (user groups), Drives, **Network &
    Remote**, Recent.
  - Dropping *between* rows, or on a group title, adds a bookmark. A line shows where it
    goes, and the label by the pointer says "Add to Favourites. Nothing is moved".
  - Dropping *on* a folder row moves or copies into that folder, as in Finder.
- **Centre.**
  - One or two panes. Each pane has its own tabs, sort, filter and selection.
  - The focused pane has the accent line on its active tab. The other pane's selection is
    shown dimmed.
  - In split view each pane's footer shows its folder.
- **Right inspector.** The preview pane (Alt+P) shows the item under the cursor: preview,
  facts and Open. Quick Look (Space) is a floating version of the same preview.
- **Lower dock.** As in 2.1. It follows the focused pane.
- **Status bar.** Notices, the running file operation (with its label), and the activity chip.
- **Drag rules, everywhere.**
  - Same drive: move. Another drive or a network or remote location: copy.
  - Ctrl (⌥ on macOS) forces copy, and Shift (⌘ on macOS) forces move, as in Explorer and
    Finder.
  - The label by the pointer always names the action and the destination, so it is never a
    guess.
  - Hovering a tab for 0.6 s while dragging switches to it.
- **Keyboard.**
  - OS conventions stay: F2 rename, Del to bin, Shift+Del delete, F5 refresh, Ctrl+C, X and V,
    Ctrl+Shift+N new folder, Space for Quick Look.
  - Dual-pane keys that don't clash: Tab switches pane, Shift+F5 copies to the other pane,
    Shift+F6 moves there, Ctrl+U swaps panes.

## 5. Technical architecture

### 5.1 Panes, tabs and focus

`Files` now holds `panes: Vec<Pane>` (one or two) and `focus`. A `Pane` holds its tabs, active
tab, sort, filter and selection state, which used to sit on `Files` itself. Every existing
accessor (`here`, `tab`, `chosen_paths`, `selection_key`) answers for the focused pane. The
dock, Storage Analysis hand-off and status bar keep working unchanged. Drawing functions take
a pane index.

### 5.2 One file-operation service for both workspaces

`menus.rs::Ops` already runs copy, move, rename, trash, delete and new folder on a worker
thread, then re-reads the paths it touched. Files now calls the same functions
(`transfer`, `paste_into`, `start_rename`, `Confirm`). When a job finishes, every Files tab
whose folder holds an affected path is listed again, keeping its selection, and the result is
selected. The scan tree is updated through `apply_updates` as before, so the dock stays
correct.

The single-job limit stays for now. A queue with progress, pause and conflict rules comes with
remote transfers (Phase 4), because that is where it is needed most.

### 5.3 Locations and backends (Phase 4 design)

`Location` gains `Remote { profile, path }`. Each backend implements one trait:

```rust
trait Backend: Send + Sync {
    fn caps(&self) -> Caps;            // rename_atomic, move_within, permissions, watch, hash, …
    fn list(&self, dir: &RPath, sink: Sender<Batch>, cancel: &AtomicBool) -> Result<()>;
    fn stat(&self, p: &RPath) -> Result<Meta>;
    fn read(&self, p: &RPath, at: u64) -> Result<Box<dyn Read>>;   // previews, downloads
    fn write(&self, p: &RPath, resume_at: u64) -> Result<Box<dyn Write>>;
    fn rename(&self, from: &RPath, to: &RPath) -> Result<()>;
    fn remove(&self, p: &RPath, recursive: bool) -> Result<()>;
    fn mkdir(&self, p: &RPath) -> Result<()>;
}
```

The UI asks `caps()` before offering a command, so remote locations never promise an atomic
move or a watcher they can't do. Local stays on `std::fs` (the fast path that already works).

- **SFTP.** Uses `russh` and `russh-sftp` (pure Rust, no OpenSSL). Host keys are checked
  against `~/.ssh/known_hosts` and dirmap's own list. An unknown key asks, a changed key
  refuses with the fingerprint shown, and nothing is ever trusted silently.
- **Credentials.** Go through the `keyring` crate: Windows Credential Manager, macOS Keychain,
  Secret Service. Profiles in `FilesPrefs` hold only a keyring reference, never a secret.
- **SMB and NFS.** Use the OS mount (UNC paths on Windows, `/Volumes` on macOS, gvfs or
  `/mnt` on Linux), so they are local paths with a "network" flag.
- **FTP.** Labelled insecure, and offered only when chosen by name.

### 5.4 Network mounts

`drives.rs` already flags network drives. The new Network & Remote section lists those, plus
saved server locations (`\\server\share`, `smb://…`). Connect to Server works per platform:

- **Windows.** `WNetAddConnection2W` with `CONNECT_INTERACTIVE`, so Windows asks for the
  password and stores it in Credential Manager. dirmap never sees it.
  `WNetCancelConnection2W` disconnects.
- **macOS.** `open smb://…` hands the URL to the system mounter, which uses the Keychain.
- **Linux.** `gio mount smb://…` (GVFS) where it is available.

Every call runs on a worker thread with its result reported in the status bar, so a dead
server never freezes the window.

### 5.5 Previews

`preview.rs` holds a small provider list: `Image`, `Text`, then `Facts`, the fallback.

- **Decoding.** Runs on a worker thread with hard limits:
  - files over 256 MB aren't decoded;
  - `image::Limits` caps the pixels at 16k × 16k and the allocation at 512 MB;
  - text reads at most 256 KB, and NUL bytes mark a file as binary.
- **Cache.** Decoded images are scaled to at most 2048 px and cached as egui textures, with a
  small LRU of 8.
- **Formats.** Formats come from `image` features: PNG, JPEG, GIF, WebP, TIFF, BMP, and
  EXR and HDR.
- **Audio and video.** Need a decoder (ffmpeg, or the platform frameworks). This is assessed
  in Phase 3b, with the decoder in a child process so a bad file can't take dirmap down.

### 5.6 Image sequences and RAM player (Phase 5 design)

`sequences.rs` is generalised from scan nodes to names, so the Files list can group frames
with no scan.

The player has three parts:

- a decode pool (a thread per core, bounded);
- a frame cache with a byte budget (default 25% of RAM, user-set), with read-ahead in the play
  direction and eviction furthest-from-playhead first;
- a GPU upload of one frame per display refresh.

Playback runs at the target rate only from cached frames. If decode can't keep up, the
counter says so (actual fps) instead of pretending. EXR uses the `exr` crate (half and float).
The display transform is sRGB or Rec.709 by default. OpenColorIO is optional, through a
config file, because it is a C++ library.

### 5.7 Other services (Phase 6)

- **Favourites.** `FavGroup { name, items: [Fav { label, path }] }` in `FilesPrefs`. Old
  `pinned` folders move into the first group on load.
- **Folder colours, tags and notes.** dirmap-only metadata in a small file keyed by path. A
  rename or move made through dirmap updates the key. A folder renamed outside dirmap loses
  its colour, and the UI says the colour is dirmap-only.
- **Applications.** Structured `{ exe, args: Vec<String>, cwd }`. The file path is passed as
  one argument, never through a shell string.
- **Templates.** Built-in, with visible contents: text, Markdown, JSON, XML, CSV, shell,
  PowerShell, Python. Users add their own by putting files in a `Templates` folder that
  dirmap shows. Templates are copied, so a `.blend` template is a real `.blend` file the user
  supplied, never an empty file with that extension.
- **Compare and sync.** Compare lists only-left, only-right, size and date differences, and
  optional hash checks. Sync shows the plan first. One-way mirror deletes only after a separate
  confirmation that lists what will be deleted. Two-way never deletes on an ambiguous
  difference.

## 6. Implementation plan

| Stage | Delivers | Accepted when |
|---|---|---|
| **1. File operations in Files** | Copy, cut and paste, rename (F2), Recycle Bin or Trash (Del), delete (Shift+Del), new folder, drag items onto folders and tabs, shared Move or Copy rules | Every command works on local disks, results get selected, other tabs refresh, the dock stays right, and nothing blocks the window |
| **2. Favourites** | Groups, drag to add with an insertion line, drag to reorder, rename label, remove, new group, move between groups, drop files onto a favourite to move or copy into it, drag a favourite to a pane or tab to open it | Dragging never moves a folder unless dropped *on* a folder, the label always says which, and favourites survive restart |
| **3. Split panes** | Single or side by side or stacked, resizable divider, independent tabs, sort, filter and selection, Tab to switch, copy or move to the other pane, swap, same folder in the other pane, the dock follows the focused pane | Two folders browse independently, drag between panes copies or moves as labelled, and preferences persist |
| **4. Address bar** | Suggestions, Tab completion, environment and home expansion, crumb menus and drops, Alt+D and F4, `>` commands and `@` favourites | Typing a partial path completes without blocking on slow shares |
| **5. Quick Look and preview pane** | Space preview, arrows to move, Esc to close, Alt+P pane, image zoom, fit and 1:1, checkerboard, dimensions, text preview | 100 MB images don't freeze the window, and malformed files show an error, not a crash |
| **6. New menu** | New Folder, Text, Markdown, JSON, XML, CSV, Script, templates folder | Created files are valid for their type, and the name is selected for renaming |
| **7. Network & Remote section** | Network drives and mounts with status, saved server locations, Connect to Server and disconnect (per OS) | An unreachable share shows an error without freezing |
| Later: Phase 3b | Icons and thumbnails, gallery and columns in Files, grouping, per-folder views, audio and video | |
| Later: Phase 4 | SFTP backend, keychain, host keys, transfer queue, remote analysis | |
| Later: Phase 5 | Sequence grouping, RAM player, EXR and DPX, colour | |
| Later: Phase 6 | Colours, tags, launcher, batch rename, compare and sync, search scopes | |

Stages 1 to 7 are this round, ending with a review with Rich before Phase 3b.

### Validation for every stage

- `cargo test`, with clippy clean.
- Windows and macOS cross-checks.
- The scan benchmark, unchanged.
- Under Xvfb: the workflow for each stage, with screenshots for the manual.
- The Help Centre pages and the keyboard and mouse reference are updated with the code.
