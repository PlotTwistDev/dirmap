# Cloud files

Google Drive, OneDrive, iCloud Drive, Dropbox, Box and similar apps can keep files **online
only**: the file shows in its folder with its full size, but its contents stay in the cloud
until you open it. Such a file takes (almost) no space on your drive.

Filescape tells these files apart, shows where each file's data is, and never downloads them
unless you ask.

## What Filescape shows

With the **Size** switch above the map on **On disk** (the default), online-only files count as
the little space they really take, so a cloud folder shows what it uses on this computer. For
example, a Google Drive folder holding 33.2 GB of books that are online only shows as a couple
of megabytes, the same figure `du -sh` gives in Terminal.

Switch to **Logical** to see the full size of everything, wherever it's stored, or to **Cloud
only** to see where the data that isn't on this computer is. See
[Size on disk and logical size](sizes.md#size-on-disk-and-logical-size).

### Status icons

Files in cloud folders get an icon after their name in the file manager, the Storage Analysis
list and the tree. Hover it to see what it means and which service the file belongs to.

| Icon | State | Meaning |
|---|---|---|
| Blue cloud | **Online only** | None of the file is on this computer. Opening it downloads it. |
| Orange half-filled cloud | **Partly downloaded** | Some of the file is here (Windows). |
| Green tick in a circle | **Downloaded** | The whole file is here. Your cloud app may free it up again when space runs low. |
| Filled green circle | **Always kept on this device** | Downloaded and pinned, so the cloud app keeps it here (Windows reports this). |
| Grey cloud with a question mark | **Unknown** | On a mounted cloud drive, which doesn't say how much of each file is stored here. |

A folder shows a blue cloud when something inside it is only in the cloud, a tick when every
cloud file in it is downloaded, and a question mark when it holds files on a mounted drive.
Ordinary files outside cloud folders have no icon. **Status icons** on the
[Cloud storage](../settings/reference.md#cloud-storage) settings page turns them off.

Hovering a tile or selecting an item also gives both sizes and where its data is, for example
`1.9 MB on disk (33.2 GB logical) · some files only in the cloud`, and the preview panel's
**Stored** line says how many files inside a folder are only in the cloud.

### Storage breakdown

When a scan has cloud files, a bar under the map splits its data, by logical size, into:

- **Not in the cloud**: files on this computer outside any cloud folder.
- **Downloaded**: cloud files with their data here.
- **Only in the cloud**: data that isn't on this computer, from online-only files and the missing
  part of partly downloaded ones.
- **Unknown**: files on mounted cloud drives.

It names the services involved and says what share of the data is here; hover that figure for
how many cloud files aren't downloaded. The breakdown always covers the whole scan, whatever the filters. Click a part
to show only those files on the map (Professional); click it again to show everything.

### Stored filter and columns

With cloud files in the scan, the filter bar gets a **Stored** row: **Online only**, **Partly
downloaded**, **Downloaded**, **Always kept on this device**, **Unknown** and **Not in the cloud**.
Pick one or more to show only those files; it combines with the type, size and Custom filters.
Like the other filters, choosing is part of Filescape Professional.

The Storage Analysis list then also shows the other size (**Logical** next to **On disk**, or the
other way round) and a **Stored** column. **Stored column** in Settings turns them off.

## Which services Filescape recognises

Filescape doesn't talk to any cloud service. It looks where the services keep their folders, and
reads what each file's own metadata says:

| System | Where Filescape looks | Services |
|---|---|---|
| macOS | `~/Library/CloudStorage` (one folder per account) and `~/Library/Mobile Documents` | Google Drive, OneDrive, SharePoint (shared libraries), Dropbox, Box and other File Provider apps; iCloud Drive |
| Windows | The sync roots Windows itself keeps, OneDrive's folders, and drives labelled as cloud drives | OneDrive, SharePoint, iCloud, Dropbox, Box, Nextcloud, Google Drive (mirror folder and the `G:` drive), pCloud and other Cloud Files apps |
| Linux | The mount table, GNOME Online Accounts, and the usual folders in your home folder | rclone and other FUSE drives (Google Drive, OneDrive, S3, Google Cloud Storage, Azure), WebDAV and Nextcloud mounts, pCloud, Dropbox, Nextcloud, ownCloud, MEGA, Synology Drive |

The **Cloud folders on this computer** section of the Cloud storage settings page lists what was
found. Services keep files on your computer in one of three ways, which decides what Filescape
can tell:

- **Files on demand** (macOS File Provider, Windows Cloud Files): each file says whether its data
  is here, so online-only, partly downloaded and pinned files are shown exactly. On macOS, such
  files carry a "dataless" mark and report no blocks on disk; on Windows they carry attributes
  saying where their data is, and Filescape asks Windows for the space they use, which doesn't
  download them.
- **Synced folders** (Dropbox, Nextcloud, MEGA and others on Linux): everything in the folder is
  on this computer, so every file shows as downloaded.
- **Mounted drives** (rclone, FUSE, WebDAV, Google Drive's `G:` drive on Windows, pCloud): files
  don't say how much of them is cached here, so they show as **Unknown**, and their size on disk is
  whatever the drive reports, which can be the full size. They're never counted as nothing.

Pinned files ("Always keep on this device") are shown on Windows. macOS doesn't report pinning in
a way that can be read without opening each file, so pinned files there show as downloaded.

### Mounted cloud drives

A cloud drive mounted inside a folder you scan (for example an rclone mount in your home folder)
is left out by default, because reading it asks the service about every folder in it, which can
be slow and count against the service's limits. Turn on **Scan mounted cloud drives** in
Settings to include them, or scan the mounted drive itself, which always works. On the command
line, `--cloud-mounts` does the same for one run.

## Filescape doesn't download your files

Scanning, browsing, sorting, filtering and switching sizes only read what the folder listing says,
so they never download cloud files. Features that have to read a file's contents leave online-only
files alone unless you say otherwise:

| Feature | With online-only files |
|---|---|
| Thumbnails (Icons view) | Not made; the type icon is shown. |
| Preview pane and Quick Look | Shows **This file is only in the cloud** and the size it would download, with a **Download and preview** button. |
| [Duplicates](../analysis/duplicates.md) | Left out of the comparison; the panel says how many files were skipped and how big they are. |
| [Folder comparison](../views/file-manager.md#comparing-and-syncing-folders) with **Compare contents** | Compared by size and date only. |
| Sequence player | Asks first, saying how many frames are only in the cloud and how much playing them would download. |

Turning off **Ask before downloading** in Settings lets the preview and the sequence player
download without asking; thumbnails, scans and duplicate searches still never read online-only
files.

Anything you open yourself (double-click, **Open with**, copying to another folder) is handled by
the cloud app as usual, which downloads it.

## On the command line

`filescape --cli PATH` prints both totals and a **Cloud** line with the services found and how many
cloud files are downloaded, online only, partly downloaded or unknown. `--logical` ranks by
logical size and `--cloud-only` by data only in the cloud.

## Related topics

- [How sizes are calculated](sizes.md)
- [Cloud storage settings](../settings/reference.md#cloud-storage)
- [Why is free space lower than expected?](../tutorials/missing-space.md)
