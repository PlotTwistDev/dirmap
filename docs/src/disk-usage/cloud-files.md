# Cloud files

Google Drive, OneDrive, iCloud Drive, Dropbox, Box and similar apps can keep files **online
only**: the file shows in its folder with its full size, but its contents stay in the cloud
until you open it. Such a file takes (almost) no space on your drive.

## What Filescape shows

With the **Size** switch above the map on **On disk** (the default), online-only files count as
the little space they really take, so a cloud folder shows what it uses on this computer. For
example, a Google Drive folder holding 33.2 GB of books that are online only shows as a couple
of megabytes, the same figure `du -sh` gives in Terminal.

Switch to **Logical** to see the full size of everything, wherever it's stored. See
[Size on disk and logical size](sizes.md#size-on-disk-and-logical-size).

Hover a tile or select an item for both sizes and where its data is:

| Shown | Meaning |
|---|---|
| `online only: opening it downloads it` | None of the file is on this computer. |
| `partly downloaded` | Some of the file is here (Windows). |
| `some files only in the cloud` | A folder holding online-only or partly downloaded files. The preview panel says how many and how much isn't downloaded. |

How Filescape knows, without asking the cloud app:

- **macOS**: files managed by macOS's File Provider (iCloud Drive and the current Google Drive,
  OneDrive, Dropbox and Box apps, usually under `~/Library/CloudStorage`) carry a "dataless" mark
  while they're online only, and report no blocks on disk.
- **Windows**: files managed by Windows' Cloud Files (OneDrive, Dropbox, Box, iCloud for
  Windows and others) carry attributes saying whether their data is here; Filescape then asks
  Windows for the space they use, which doesn't download them.
- **Linux**, and cloud drives mounted by other means (rclone, FUSE drives, Google Drive's `G:`
  drive on Windows): Filescape shows what the drive reports. Some of these report the full size
  of files that aren't downloaded; recognising them is planned.

## Filescape doesn't download your files

Scanning, browsing, sorting and switching sizes only read what the folder listing says, so they
never download cloud files. Features that have to read a file's contents leave online-only files
alone unless you say otherwise:

| Feature | With online-only files |
|---|---|
| Thumbnails (Icons view) | Not made; the type icon is shown. |
| Preview pane and Quick Look | Shows **This file is only in the cloud** and the size it would download, with a **Download and preview** button. |
| [Duplicates](../analysis/duplicates.md) | Left out of the comparison; the panel says how many files were skipped and how big they are. |
| [Folder comparison](../views/file-manager.md#comparing-and-syncing-folders) with **Compare contents** | Compared by size and date only. |
| Sequence player | Asks first, saying how many frames are only in the cloud and how much playing them would download. |

Anything you open yourself (double-click, **Open with**, copying to another folder) is handled by
the cloud app as usual, which downloads it.

## Related topics

- [How sizes are calculated](sizes.md)
- [Why is free space lower than expected?](../tutorials/missing-space.md)
