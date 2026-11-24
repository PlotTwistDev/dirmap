# dirmap 2.3: the Remote workspace

Status: assessment and plan, 20 November 2026; phase 1 implemented the same day (see the tracker). It follows `dirmap-2.2-file-manager.md` (phase 4
there added SFTP, FTP(S) and WebDAV). The brief replaces the sidebar's **Network & Remote**
section and its Connect to Server box with a **Remote** workspace: two panes (local and remote
side by side), a site manager, connection tabs, a transfer dock, and the analysis tools, all
built from the Files workspace rather than beside it.

The comparison in section 3 draws on the documented behaviour of WinSCP 6 (Commander and
Explorer interfaces, Site Manager, background queue, Synchronize), FileZilla 3 (Site Manager,
queue, directory comparison), Cyberduck 9 (bookmarks, transfer window, edit-in-place), File
Explorer (Windows 11) and Finder (macOS 15). It is not based on new measurements.

## 1. Summary of decisions

1. **One file manager, two desks.** Remote is the Files workspace with a different *desk*:
   its own panes, tabs, split and dock layout, swapped in when the user switches workspace.
   Every list, view, menu, shortcut, preview, drag and file operation is the same code. No
   second file manager is written.
2. **Locations stay paths.** `sftp://user@host:port/dir` already travels through `Location`,
   `Listing`, `transfer`, preview, thumbnails, compare and the scanner as a `PathBuf`. That
   stays. What's added is a **capability** query per location, so the UI greys out what a
   protocol can't do instead of failing after the click.
3. **Sites become profiles.** `SavedSite` (name, url, key) grows into a `SiteProfile` with a
   group, favourite flag, protocol options, auth method, start folder, last-used time and
   notes. Secrets stay in the system keychain only. Old saved sites are migrated on load.
4. **Transfers become a real queue.** The single `OpJob` + `queued` list is replaced, for
   copies and moves, by a queue with several workers, one row per item, speed and time left,
   retry, resume of partial files where the protocol allows, temporary-name uploads, and a
   history. It lives on `App`, so it keeps running in any workspace.
5. **SMB stays with the operating system.** Windows' own UNC paths and Credential Manager,
   macOS's mounter and Keychain, GVFS on Linux. dirmap never sees those passwords. The Remote
   workspace lists and opens them like the other servers.
6. **SSH terminal: the system's terminal first.** "Open SSH Terminal Here" starts the
   platform terminal running `ssh -t user@host 'cd <dir> && exec $SHELL -l'`, which uses the
   user's `~/.ssh/config`, agent and keys. An embedded terminal pane is possible later
   (section 6.7) but is a project of its own.

## 2. Audit: what dirmap has today

The audit started from the code on 18 November (phases 0 to 6 done). Nothing was assumed
missing.

### 2.1 The Network & Remote section

`app/netui.rs` draws the sidebar section and the Connect to Server box. The section lists:
network drives the drive list found, what the system remembers (Windows mapped drives through
`WNetEnumResource`, GVFS mounts), the user's saved UNC paths and mount points
(`FilesPrefs.servers`), and the saved SFTP, FTP and WebDAV servers (`FilesPrefs.sites`, a
name, URL and key file each). The Connect box has a kind picker: SMB through the system
(UNC path, drive letter, reconnect at sign-in) or one of dirmap's own protocols (host, port,
user, folder, key file, name). Connecting runs on a worker; questions from the connection
(new host key, changed host key, password, key passphrase) come back to the UI through
`remote::set_asker` and are answered in a small window.

### 2.2 Protocols: what works

| Protocol | State | Where | Notes |
|---|---|---|---|
| SMB / CIFS | ✅ through the OS | `netdrives.rs` | Connect, disconnect, map a letter (Windows), UNC copy. Browsing is ordinary local file access |
| SFTP | ✅ | `fsys/remote/sftp.rs` (russh, russh-sftp) | known_hosts check (the user's and dirmap's own), agent (Unix socket, Windows OpenSSH pipe, Pageant), key files, password, keychain. Live tests behind `cargo test live_ -- --ignored` |
| FTP | ✅ | `ftp.rs` (suppaftp) | MLSD with LIST fallback, pool of idle control connections |
| FTPS | ◐ | `ftp.rs` | Explicit TLS (AUTH TLS) with the platform verifier. Implicit FTPS (port 990) is not supported |
| WebDAV http / https | ✅ | `dav.rs` (ureq, roxmltree) | PROPFIND depth 1, Basic auth, platform TLS verifier |
| SCP | ○ | — | `scp://` is read as SFTP. Servers with SFTP disabled can't be used |

Untested on a real device: the Windows SSH agent and Credential Manager, the macOS Keychain.

### 2.3 Placeholders and gaps

No UI placeholder pretends to work. The gaps are real missing features:

- **Saved servers** are a flat list with no groups, favourites, duplicate, search, import or
  export, and no "test connection". There is no quick-connect bar.
- **Insecure protocols**: the Connect box shows a line of text for FTP and http WebDAV, but
  nothing stops or confirms the connection.
- **One connection per server, used by one call at a time** for SFTP and WebDAV (the pool in
  `remote/mod.rs`). A long download blocks listing that server until it finishes; there are
  no parallel transfers to one server.
- **Transfers** run one at a time (`Ops.job`, others wait in `Ops.queued`) with a single
  progress bar in the status bar: no per-item rows, speed, time left, retry, history or
  resume. A failed download deletes the partial file; an upload writes straight to the final
  name, so a dropped connection leaves a truncated file on the server.
- **No latency, no connection log**, no "connected since" or idle timeout.
- **Remote permissions** (chmod), symlink targets and owner are read by SFTP but not shown or
  editable.
- **Editing a remote file** opens a downloaded temporary copy; saving it doesn't upload.
- **No SCP, implicit FTPS, S3 or SSH terminal.**

### 2.4 How Files works, and what Remote reuses

| Piece | Where | Remote-ready? |
|---|---|---|
| Panes (one or two), tabs, per-tab history, focus | `files.rs` (`Files`, `Pane`, `FTab`), `fsys/history.rs` | ✅ remote paths open in any tab |
| Listings streamed on workers, cancel, slow notice | `fsys/listing.rs` | ✅ (`remote::list`) |
| Details, Icons, Columns, Tree views; sort, filter, group, sequences | `files.rs`, `views.rs` | ✅ |
| Address bar with suggestions, breadcrumbs | `addr.rs` | ✅ parses `sftp://…` |
| Favourites, Recent, saved searches | `favs.rs`, `FilesPrefs` | ✅ remote folders can be favourites |
| Preview, Quick Look, thumbnails | `preview.rs`, `thumbs.rs` | ✅ (fetch to a capped temp file) |
| Copy, move, delete, rename, new folder, clashes, Undo | `transfer.rs`, `actions.rs`, `fileops.rs`, `menus.rs` | ✅ dispatched on `is_remote`; Undo of remote deletes isn't possible (no bin), and the UI says so |
| Drag and drop between panes, tabs, favourites | `dnd.rs` | ✅ |
| Compare Folders | `compare.rs` | ✅ lists both sides, remote too |
| Analysis dock, treemap | `dock.rs`, `scan::scan_remote` | ✅ Analyse on a remote folder scans it (recursive, cancellable) |
| Sequence player | `player.rs`, `seqplayer.rs` | ◐ needs frames local; remote clips play only after download |

**Is there a provider abstraction?** Yes, one level down: `remote::Backend` (stat, list,
mkdir, rmdir, rm, rename, get, put, touch, home, alive) with three implementations, and a
path-based dispatch (`remote::is_remote`) at about 70 call sites. It's not a trait over local
*and* remote, and it doesn't report capabilities. Section 4.2 keeps the dispatch (it works,
and making local files go through a trait object would touch the 1.3 M-file scan path) and
adds the capability layer the UI needs.

**Conclusion.** Most of the brief's "reuse the Files workspace" list already works on remote
folders today; it is hidden behind a sidebar section and a modal box. The work is mostly
(a) a workspace and layout that put it in front of the user, (b) a proper site manager,
(c) a proper transfer queue, and (d) protocol depth: concurrency, resume, permissions,
edit-and-upload, SCP, implicit FTPS, and later S3.

## 3. Comparison with established tools

Key: ● has it, ◐ partly, ○ no. "dirmap" is the state before this work.

| Capability | WinSCP | FileZilla | Cyberduck | Explorer | Finder | dirmap |
|---|---|---|---|---|---|---|
| Local and remote side by side | ● (Commander) | ● | ○ (browser + transfers window) | ○ | ○ | ◐ (split panes, not by default) |
| Site manager with folders | ● | ● | ● (bookmarks) | ○ | ◐ (favourite servers) | ○ (flat list) |
| Quick connect bar | ◐ (login dialog) | ● | ● | ◐ (address bar) | ● (⌘K) | ◐ (address bar) |
| Tabs per connection | ● | ● | ● (windows) | ● | ● | ● |
| Background queue, parallel | ● | ● | ● | ◐ | ◐ | ◐ (serial) |
| Resume partial transfers | ● | ● | ● | ○ | ○ | ○ |
| Speed, time left, per-file rows | ● | ● | ● | ◐ | ◐ | ◐ (one bar) |
| Clash rules: overwrite, newer, size differs, rename, skip | ● | ● | ● | ◐ | ◐ | ◐ (Replace, Keep Both, Skip, Merge) |
| Compare folders | ● | ● | ○ | ○ | ○ | ● |
| Synchronise with preview | ● | ○ | ● | ○ | ○ | ○ |
| Edit remote file, upload on save | ● | ● | ● | ○ | ○ | ○ |
| Permissions (chmod) | ● | ● | ● | ○ | ○ | ○ |
| SSH terminal | ◐ (PuTTY launch, console) | ○ | ◐ (open in Terminal) | ○ | ○ | ○ |
| Thumbnails, previews, image sequences, treemap of a remote folder | ○ | ○ | ◐ (Quick Look) | ◐ | ◐ | ● |

What to take from each:

- **WinSCP Commander**: local left, remote right; F5 copy, F6 move to the *other* pane;
  Synchronize with a checklist of proposed changes before anything runs; "Keep remote
  directory up to date". Its density is what the brief asks to avoid.
- **FileZilla**: the quick-connect bar (host, user, password, port, Connect) above the panes,
  and a queue with Queued, Failed and Successful tabs. The connection log is useful when
  something goes wrong, but noisy as a default.
- **Cyberduck**: bookmarks as the home screen, and edit-in-place that watches the temporary
  file and uploads on save. The cleanest approachable model.
- **Explorer and Finder**: Connect to Server as a small dialog, favourite servers, and the
  idea that a server is just another place in the sidebar. That's what dirmap does today and
  should keep in the Files workspace.

## 4. Architecture

### 4.1 Workspaces and desks

`Workspace` gains a third value, `Remote`, on Ctrl/⌘+2 (unused today, between Files on 1 and
Storage Analysis on 3), and in the switcher between the two.

A **desk** is the part of the Files state that is about layout, not about the user's things:

```
struct Desk {
    panes: Vec<Pane>,        // tabs, history, sort, view, filter per pane
    focus: usize,
    closed: Vec<Location>,   // Ctrl+Shift+T
    layout: DeskPrefs,       // split, ratio, second pane, preview, sidebar shown, dock layout
}
```

`Files` keeps one active desk in its existing fields and one stashed desk. Switching between
Files and Remote swaps them (`std::mem::swap`, a few fields), so every function in
`files.rs`, `views.rs`, `dnd.rs`, `addr.rs` and the rest works on Remote with no change.
Favourites, sites, marks, apps, recent folders, searches and the player's settings are
shared and never swapped. The two desks are saved separately in `FilesPrefs`
(`remote_desk: DeskPrefs`).

Remote's defaults: two panes, left on the last local folder (or Home), right on the last
remote folder (or the Sites page, 4.4); details view; dock on the Transfers tab.

Where Remote differs from Files, it's by a check of `workspace == Remote` in a few places:
the sidebar's sections, the toolbar's connection bar, the dock's default tab, and F5/F6
meaning "copy/move to the other pane" (as in Commander) when there are two panes.

*Why not a second `Files`?* Every method is on `App` and reads `self.files`; two instances
would mean passing one around in 4,000 lines that another thread is changing now. *Why not
a generic `Provider` trait for local and remote?* The local path is the fast path for the
3.5 M-file scan and for Windows shell calls; routing it through a trait object buys nothing
the path dispatch doesn't already give.

### 4.2 Capabilities

```
pub struct Caps { rename, mkdir, delete, chmod, symlinks, server_copy, resume_get,
                  resume_put, checksum: Option<HashKind>, trash, mtime_set, ... }
pub fn caps(path: &Path) -> Caps   // local: everything the OS has; remote: by scheme + server
```

Each backend reports what the server actually offered (an SFTP server's extensions such as
`copy-data` and `posix-rename`, FTP's FEAT list with MLSD, REST STREAM and MFMT, a DAV
server's `DAV:` header and whether it allows MOVE and COPY). Menus and toolbar buttons use
it to disable items with a reason on hover ("This FTP server can't set modified times").
Delete on a server says "can't be undone" in the confirmation, as it does now.

### 4.3 Connections

- **A session per site** (`remote::pool`) stays the source of truth for "connected", but
  each session becomes a small set of channels: one for browsing (listings, stats, small
  reads) and up to *N* for transfers (SFTP: extra SFTP channels on the same SSH connection;
  FTP: extra control+data connections, already pooled; WebDAV: ureq's agent pool). Browsing
  never waits behind a transfer.
- **Keepalive and idle close**: a ping every 30 s while a pane shows the site; close after
  a configurable idle time (default 10 minutes) with nothing shown or queued.
- **Latency**: the round-trip of the last few small requests, shown in the status chip.
- **Connection log**: a ring buffer per site of what was sent and received at the level of
  "LIST /home/rich (184 items, 120 ms)", "Signed in with key id_ed25519". Passwords,
  passphrases, keys and tokens never reach it; a test asserts that.
- **Timeouts and retries**: per-profile connect timeout (default 15 s) and retry count
  (default 2, with backoff) for dropped connections. Listing errors show in the pane with
  Retry and the log link instead of a bare message.

### 4.4 Site manager

```
struct SiteProfile {
    id: u64, name: String, group: String,          // "Render Farm/Nodes" (slash = subfolder)
    scheme: Scheme, host: String, port: Option<u16>, user: String,
    start_dir: String, local_dir: Option<PathBuf>, // where the left pane opens
    auth: Auth,                                    // Password | KeyFile(path) | Agent | System (SMB)
    favourite: bool, color: Option<Color32>, notes: String,
    options: ProtoOptions,                         // passive/active, implicit TLS, encoding,
                                                   // keepalive, timeout, transfer channels, sync browse
    last_used: i64,
}
```

- Stored in `FilesPrefs.sites` (migrated from `SavedSite`). Passwords and passphrases are
  only in the keychain under the profile's account, as today.
- **Site Manager window**: a tree of groups and sites on the left with search, favourites
  first; the form on the right; buttons New Site, New Group, Duplicate, Rename, Delete,
  Test (connect, list the start folder, report time and server type), Connect, Connect in
  New Tab.
- **Import and export**: dirmap's own JSON (no secrets, ever; a tick to include *nothing*
  more is not offered). Import from WinSCP's `WinSCP.ini` and FileZilla's `sitemanager.xml`
  (host, port, user, folder, group; their stored passwords are ignored).
- **Quick connect bar**: one field (`sftp://rich@render01/mnt/shots` or just `render01`),
  protocol picker, Connect. A quick connection isn't saved unless the user picks Save as
  Site afterwards.
- **SMB profiles** keep the system's sign-in: a profile is a UNC path (and drive letter on
  Windows). "Test" asks the system to connect.

### 4.5 Transfer manager

A `Transfers` service on `App` (new `src/transfers.rs`, with UI in `app/transfersui.rs`):

- **Jobs and items.** A drop or F5 makes a *job* (sources, destination, copy or move, clash
  rule). Expanding folders happens on a worker and adds *items* (one per file, folders as
  mkdir items) as it goes, so a 200,000-file tree starts transferring at once and the UI
  never holds more than the visible rows plus counters per job.
- **Workers.** A pool of *N* (default 3, setting 1 to 8) per destination site, 1 for
  local-to-local on the same disk. Each item streams in chunks (4 MB, the size `transfer.rs`
  already uses) through `Backend::get_range` / `put_at`; nothing loads a whole file.
- **Resume.** A download writes `name.dirmap-part`; on retry it continues from the part's
  length if the server supports offset reads (SFTP always; FTP with REST; WebDAV with Range).
  Uploads go to `.name.dirmap-part` and are renamed over the target when complete (atomic
  where the server's rename is), so an interrupted upload never leaves a truncated file
  under the real name; FTP servers without rename-over get delete-then-rename, flagged in
  the item's log.
- **Clash rules** for the whole job: Ask (today's box), Overwrite, Overwrite if newer,
  Overwrite if size differs, Skip, Keep Both (rename). Ask's "do this for the rest" stays.
- **Verification** (optional, per job): compare SHA-256 where the server can compute it
  (SFTP `check-file` extension, or `sha256sum` over SSH exec when the profile allows it),
  otherwise compare sizes and set times.
- **Progress**: per item and per job: bytes, speed (5 s moving average), time left, state
  (waiting, running, paused, done, failed, skipped). Pause, Resume, Retry, Cancel per item,
  per job and for all.
- **History**: finished jobs with their failures stay in a list for the session and in a
  small log file (`transfers.log`, names and sizes, no credentials).
- **Background**: the queue runs while the user is in Files or Storage Analysis; the
  toolbar shows a small "↓ 3 · 42 MB/s" chip that opens the dock's Transfers tab.
- **Undo** still records local-side steps (new files, moves) as today.

Local copies and moves keep using today's `transfer::transfer_with` path inside a job until
the queue has proved itself; then they move onto it too, so one queue shows everything.

### 4.6 Remote operations

- Rename, new folder, delete (recursive, with the count first), copy within one server
  (server-side when `Caps.server_copy`, otherwise download-and-upload through the queue).
- **Properties** on a remote item: size, times, owner and group (SFTP), permissions with an
  rwx grid and octal field (SFTP and FTP with SITE CHMOD), symlink target.
- **Edit**: "Edit" downloads to a per-site cache folder, opens it in the chosen app, watches
  the file (the `notify` watcher already in `fsys/watch.rs`), and uploads it through the
  temp-name path on each save, with a toast "Uploaded notes.txt to render01". If the
  server's copy changed since the download, it asks before overwriting.
- **Open**: same download-to-cache, with a size check above a setting (default 500 MB).
- **Hidden files** follow Show Hidden (dot-files on servers). Symlinks show their target in
  the tooltip and Properties; following a link to a folder stays on the server, and `..`
  is resolved on the server, never on the local disk (paths are always joined in
  `remote::join`, which rejects `..` and absolute names from server listings).
- **Refresh**: F5 in the Files workspace already re-lists; in Remote, Ctrl+R refreshes (F5
  is Copy, as in Commander). Listings are cached per site for back and forward with a 30 s
  freshness window, and dropped on any change made through dirmap.

### 4.7 Compare, synchronise, analyse

- **Compare panes** (Ctrl+= ): the existing Compare Folders, opened on the two panes'
  folders, marks rows in both panes (only here, only there, newer, size differs) instead of
  a separate window, as an option.
- **Synchronise** (new `app/sync.rs`): direction (left→right, right→left, both), what counts
  as different (size, time with tolerance, checksum), delete extras (off by default), a
  preview list grouped by action with ticks, then one transfer job. Nothing runs before the
  preview is confirmed.
- **Analyse a remote folder**: today's Analyse already scans remote folders recursively.
  It gets a confirmation with the folder named ("This lists every folder under
  /mnt/shots on render01 and can take a while"), progress, cancel, and a cache of the
  result per site and folder so the treemap and folder sizes reappear without a rescan.
  Size of a selection before a transfer comes from the same walk (`bytes_in`) and is shown
  in the job's row while it counts.
- Sequences, filters and grouping already work on remote listings.

### 4.8 Security

- Host keys: today's checks stay (refuse a changed key, ask on a new one, remember in
  dirmap's own known_hosts if the user says Always).
- TLS: platform verifier for FTPS and https. A certificate that fails gets a details window
  (issuer, validity, fingerprint) and a per-site "trust this certificate" that pins the
  fingerprint, never a global "ignore errors".
- **Unencrypted protocols**: FTP and http WebDAV show an open-padlock badge on the site,
  tab and status chip, and the first connection to a site asks "Passwords and files will be
  sent unencrypted" with Connect Anyway and Remember for this site.
- No passwords in prefs, logs or crash reports (the crash.log the other thread is adding
  must use `Site::short()`, never the URL with credentials).
- Paths from servers are names, never joined as paths: `/`, `\`, `..` and NUL in a listed
  name are rejected (a malicious server can't make a download write outside the target).
- Downloads never follow a server symlink outside the job's folder.

### 4.9 SSH terminal

- **Phase 2:** "Open SSH Terminal Here" (pane menu, folder menu, toolbar, Ctrl+Shift+T is
  taken so Ctrl+Alt+T) for SFTP sites: Windows Terminal or PowerShell running `ssh.exe`
  (built into Windows 10+), Terminal.app on macOS, the user's terminal on Linux; the command
  is `ssh -t -p PORT [-i KEY] user@host "cd '<dir>' && exec \$SHELL -l"`. It uses the same
  agent and keys as the user's own ssh. The host key may need accepting again in ssh's own
  known_hosts; dirmap writes its accepted keys to its own file, not to the user's.
- **Later (option):** an embedded terminal tab in the dock, using russh's exec/shell
  channel on the existing connection and a terminal emulator crate (`alacritty_terminal`'s
  grid, drawn with egui). This needs input handling, colours, resize, selection and
  scrollback, so it's sized as its own phase, not folded into this one.

### 4.10 More protocols (investigated)

| Protocol | Value | Recommendation |
|---|---|---|
| SCP | Servers with the SFTP subsystem off (some NAS, embedded boxes) | Phase 4: listing through `ls -la` over exec, transfers through `scp -f/-t` on russh. Read-mostly; flagged as limited |
| Implicit FTPS (990) | Older servers | Phase 3, small change in `ftp.rs` |
| S3-compatible (AWS, MinIO, Wasabi, Backblaze B2, Cloudflare R2) | High for VFX archives and render outputs | Worth doing after Phase 5, as a `Backend` with multipart upload; ask before starting |
| NFS | Mounted by the OS already; drives list flags nfs mounts | No own client |
| SSH-only browsing (no SFTP) | Same as SCP | Covered by SCP |
| Cloud drives (OneDrive, Google Drive, Dropbox) | Through their own sync clients, already local folders | No |

## 5. Interface

```
┌ 🗀 Files │ ⇄ Remote │ ⊞ Storage ┐  [⚡ sftp ▾ render01        user  •••  22  Connect]  ● render01 12 ms  ↓3 42 MB/s
├──────────────┬──────────────────────────────────┬──────────────────────────────────┤
│ SITES  🔍    │ ◀ ▶ ▲ ⟳  C: › Users › rich › shots │ ◀ ▶ ▲ ⟳  render01 › mnt › shots   │
│ ★ render01   │ [shots] [Desktop] +              │ [render01] [nas01: /proj] +       │
│ ▸ Render Farm│ Name          Size   Modified    │ Name          Size   Modified  ⇅ │
│ ▸ NAS        │ ...                              │ ...                               │
│ ▸ Clients    │                                  │                                   │
│ SESSIONS     │                                  │                                   │
│ ● render01   │                                  │                                   │
│ RECENT       │                                  │                                   │
│ FAVOURITES   │                                  │                                   │
├──────────────┴──────────────────────────────────┴──────────────────────────────────┤
│ Transfers (3) │ History │ Log │ Preview │ Analysis                         ▾        │
│ ↑ plate_v003.1001-1240.exr  1.2 GB of 4.8 GB  38 MB/s  1:35 left   ⏸ ✕            │
└────────────────────────────────────────────────────────────────────────────────────┘
```

- **Toolbar**: the workspace switch, the quick-connect bar (collapses to a ⚡ button when
  narrow), the focused pane's connection chip (padlock, name, latency, Disconnect), the
  transfer chip. Back, forward, up and refresh stay in each pane's bar, as in Files today.
- **Sidebar**: Sites (favourites first, then groups, with search), Sessions (connected now,
  with Disconnect), Recent servers, then the shared Favourites and Drives. Hide with the
  existing sidebar toggle.
- **Panes**: Files' panes, tabs and bars. A pane's tab strip shows a site colour stripe and
  padlock on remote tabs. Maximise either pane by double-clicking its bar (existing split
  keys work too). Swap with the existing swap command.
- **Dock**: the existing dock, with Transfers, History and Log tabs added in Remote (and
  reachable from the transfer chip in any workspace); Preview and Analysis as in Files.
- **Files workspace**: keeps Network & Remote as a compact sidebar section (connected
  sessions and favourite sites, "Open in Remote"), so nobody loses a way in. The Connect to
  Server box becomes the quick-connect bar's popup and the Site Manager.

Colours, type, icons and spacing come from `style.rs`; no new visual language.

## 6. Implementation plan

Each phase ends with a Windows build on Rich's PC and a review, as before.

### Phase 1: Foundation
1. `Workspace::Remote`, switcher entry, Ctrl/⌘+2, desk swap (4.1), saved separately.
2. `SiteProfile` with migration from `SavedSite` and `servers`; Site Manager window (tree,
   search, form, New/Duplicate/Rename/Delete, groups, favourites, Test, Connect).
3. Quick-connect bar and connection chip; insecure-protocol confirmation.
4. Remote sidebar: Sites, Sessions, Recent servers.
5. `remote::caps` skeleton, browsing channel separate from transfer channels.
6. Connection log (ring buffer, redaction test).

### Phase 2: Core file management
1. Remote defaults (two panes, local left, remote right), F5/F6 to the other pane, Ctrl+R.
2. Capability-aware menus and toolbar.
3. Properties with permissions, owner, symlink target; chmod (SFTP, FTP).
4. Open SSH Terminal Here (4.9).
5. Listing cache for back/forward; error pane with Retry.

### Phase 3: Transfers
1. `transfers.rs` queue, jobs, items, worker pool, clash rules.
2. Range reads and offset writes in the three backends; resume; temp-name uploads.
3. Dock tabs: Transfers, History, Log; toolbar chip.
4. Retries with backoff on dropped connections; implicit FTPS.
5. Verification option.

Phase 3 tests: `live_servers` (resume from a part, a stale part started over, stop leaves
nothing, upload continued from a part and renamed over an existing file, on SFTP and FTP) and
`live_retry_and_resume` (through a proxy that cuts the first connections part-way:
`python3 docs/design/tools/flaky-proxy.py 2223 2222 2500000 3`, then
`DIRMAP_TEST_FLAKY=sftp://tester@127.0.0.1:2223/home/tester cargo test live_retry -- --ignored`).

### Phase 4: Advanced
1. Compare panes in place; Synchronise with preview.
2. Edit with upload on save.
3. Remote analysis confirmation, cache and transfer-size counts.
4. Import from WinSCP and FileZilla; export.
5. SCP backend.

### Phase 5: Refinement
1. Performance: listings of 100,000 entries on a slow link, worker limits, memory.
2. Keyboard and drag review against WinSCP Commander habits.
3. Test servers: OpenSSH (Linux), Windows OpenSSH, Synology DSM (SFTP, FTP, WebDAV),
   vsftpd with TLS, Nextcloud WebDAV, Samba.
4. Security review (4.8) and a credential-leak test over logs and saved prefs.
5. Help Centre: a "Remote workspace" chapter, settings rows, keyboard reference.

### Validation for every stage
- `cargo test` (help docs, filters, path round-trips, redaction), live tests against
  OpenSSH, vsftpd and a WebDAV server in the container
  (`cargo test live_ -- --ignored --test-threads=1`: the tests share the global asker and the
  listing cache, so they must run one at a time). FTPS and https WebDAV with a test CA:
  `SSL_CERT_FILE=<ca + system bundle>` for the test process only.
- GUI checks under Xvfb with screenshots.
- `cargo check` for Windows and macOS targets; the scan benchmark doesn't regress.
- Rich's Windows PC: SMB, mapped drives, Credential Manager, Windows OpenSSH agent.

## 7. Tracker

| # | Item | Phase | State |
|---|---|---|---|
| 1 | Remote workspace, Ctrl+2, desk swap | 1 | ✅ `app/remoteui.rs` (`use_desk`, `finish_save`, `FilesPrefs.remote_desk`) |
| 2 | Site profiles, migration | 1 | ✅ `SavedSite` grew group, favourite, notes, local folder, insecure choice, last used (old prefs load unchanged) |
| 3 | Site Manager window | 1 | ✅ list by group with search, form, Save, Test, Connect, New, Duplicate, Delete, rename group |
| 4 | Quick connect, connection chip, insecure warning | 1 | ✅ `Ask::Insecure` in `remote::open`, so every way of connecting asks |
| 5 | Remote sidebar (Sites, Sessions, Recent) | 1 | ✅ Sites (favourites, groups, search), Other Connections, Recent Servers (phase 5: `FilesPrefs.recent_servers`, one folder per server, last 8, `remoteui::note_recent`) |
| 6 | Capabilities, browse channel apart from transfers | 1 | ✅ `remote::caps` (phase 2); connection pool keyed by (site, lane), browsing on lane 0, each running transfer on its own lane (`remote::set_lane`) |
| 7 | Connection log with redaction | 1 | ✅ `remote::log_of`, `latency`; unsafe listed names refused (`safe_name`, `local_name`) |
| 8 | Two-pane defaults, F5/F6 to other pane | 2 | ✅ `remoteui::remote_keys` (Shift+F5/F6 still work) |
| 9 | Capability-aware menus | 2 | ✅ `remote::caps`; `RowMenu.server/ssh` drop Explorer, system menu, bin, UNC on servers |
| 10 | Remote properties, chmod | 2 | ✅ `RemoteProps` window; `Backend::chmod/readlink` (SFTP setstat, FTP SITE CHMOD); recursive option |
| 11 | SSH terminal (system) | 2 | ✅ `remote::ssh_args`, `remoteui::launch_ssh` (ssh.exe console, Terminal.app, Linux terminals); Ctrl+Alt+T |
| 12 | Listing cache, error pane | 2 | ✅ Try Again on errors (phase 2); listing cache (phase 5): `remote::listed` keeps each folder's list 60 s (32 folders, 400k names), every change through the `Tracked` backend wrapper drops that folder, its parent and anything under it, a listing begun before a change isn't kept, Refresh/Try Again call `remote::forget`; Compare and transfers use `list_fresh`/`listed_fresh` |
| 13 | Transfer queue, workers, clash rules | 3 | ✅ `app/transfers.rs`: anything to or from a server joins the queue from `App::transfer`; 1–8 at once (default 2); per-job pause, stop, retry; clash questions go through the usual clash window one at a time; Undo per job. Local copies keep the one-at-a-time `Ops` job. The queue isn't kept across restarts: asking for the same copy again continues the part files |
| 14 | Resume, temp-name uploads | 3 | ✅ `Backend::get_from/put_at` (SFTP seek, FTP REST and APPE, WebDAV Range); downloads to `.name.dirmap-part` beside the target, uploads to `.name.dirmap-part` in the server folder, renamed into place when complete (old target moved aside first where the server won't rename over it); parts continued only when their time matches the source |
| 15 | Transfers, History, Log dock tabs; chip | 3 | ✅ as a Transfers panel along the bottom (running, waiting, then the last 100 finished with results) and a status-bar chip with count and speed; the per-site log stays in the Connection Log window |
| 16 | Retries; implicit FTPS | 3 | ✅ 3 retries with 1/2/4 s back-off on dropped connections, reconnecting and continuing; implicit FTPS on port 990 |
| 17 | Verification | 3 | 🔨 size checked after every file (and the part kept out of place if it differs); checksums not yet: SFTP has no standard hash request and FTP's HASH is rare, so it needs a read-back pass, planned with Synchronise in phase 4 |
| 18 | Compare in panes; Synchronise | 4 | ✅ Compare Folders (already able to read servers) gained Ignore times, click-to-leave-out rows and Select in Panes (selects differences in both panes, instead of colouring rows, which would need the list's row painter); a sync with a server runs as one queue job (`compare::sync_items`, `App::queue_sync`), Retry skips files already there; `transfer_with` keeps totals a caller preset |
| 19 | Edit and upload on save | 4 | ✅ `remote::edit`: opening a server file (double-click, Open With) fetches it to its own folder, a 1 s poll sends each save once it settles (part-file upload on its own lane), `Ask::Overwrite` if the server's copy changed; listed in the Transfers panel with Stop Sending |
| 20 | Remote analysis cache, transfer sizes | 4 | 🔨 sizes before a transfer show while the queue counts; browsing is covered by the listing cache (row 12); a remembered Storage Analysis scan of a server isn't built (each Analyse reads the server again) |
| 21 | Import/export profiles | 4 | ✅ `app/siteimport.rs`: FileZilla sitemanager.xml (folders as groups), WinSCP.ini and the WinSCP registry key (via `reg query`), export as FileZilla XML; never passwords |
| 22 | SCP | 4 | ○ deferred: every server with SCP that dirmap has met also offers SFTP (OpenSSH has defaulted scp to SFTP since 9.0), so SCP-only is rare; revisit if Rich has such a box |
| 23 | Performance, server matrix, security review | 5 | 🔨 SFTP keeps 16 READDIRs in flight on a second channel (`Sftp::read_dir_ahead`): 100k items 87 s → 15 s at 40 ms/2 MB/s, 1.4 s → 0.25 s locally. Live tests pass on OpenSSH, vsftpd (plain and explicit TLS), WsgiDAV (http and https); an untrusted FTPS certificate is refused with a plain message. Security: per-user 0700 temp folder (`remote::temp_dir`), uploads skip links leading outside the folder sent (`copy_tree_in`, and in sync), `user:pw@host` passwords go to the session store and out of the address; live test checks the log, GUI check the saved settings. Not done: Windows OpenSSH, Synology, Nextcloud, Samba (Rich's network), keyboard/drag review against WinSCP |
| 24 | Help Centre chapter | 5 | ✅ `docs/src/views/remote.md` (Recent Servers, Listings and Refresh, password in an address, link and temp-folder safety; screenshot remote-recent.png) |
| 25 | Embedded SSH terminal | later | option |
| 26 | S3-compatible storage | later | option |
