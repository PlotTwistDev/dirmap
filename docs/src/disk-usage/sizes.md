# How sizes are calculated

Filescape reads two sizes for every file, from the folder listing alone (it never opens the
files, which is why scanning is fast and why it never downloads cloud files):

- **Logical size**: the number of bytes in the file. What Explorer shows as "Size", Finder as
  the file's size, and `ls -l` on Linux and macOS.
- **Size on disk**: the space the file actually takes on this computer's drive. What Explorer
  shows as "Size on disk" and `du` counts on macOS and Linux.

## Size on disk and logical size

The **Size** switch above the map (**On disk** / **Logical**) picks which of the two the map,
the sunburst, the folder list, the File types list, the size histogram, the summary under the
map, the **Larger than** and **Smaller than** filters, percentages and the status bar use.
Switching is instant and doesn't rescan: both sizes were read by the scan. Your zoom, selection
and open folders stay where they are. Filescape remembers the choice for next time.

- **On disk** (the default) answers "what is filling my drive, and what would deleting it free?".
  Files in Google Drive, OneDrive, iCloud Drive, Dropbox and other cloud folders that are only
  online take (almost) no space here, so they no longer crowd the map.
- **Logical** answers "how much data is here?", wherever it's stored. Use it to see how big a
  cloud folder is in the cloud, or to compare with sizes other apps show.
- **Cloud only** appears when the scan has files that are online only or partly downloaded. It
  counts only the data that is in the cloud and not on this computer, so the map shows where that
  data is. While a scan is still running, the live view shows logical sizes in its place, and a
  snapshot comparison compares logical sizes.

Hovering a tile or selecting an item shows both sizes when they differ, for example
`1.9 MB on disk (33.2 GB logical) · some files only in the cloud`. The file manager's **Size**
column follows the switch too; its heading reads **On disk** while sizes on disk are shown.

The two sizes differ for these reasons:

| Situation | Size on disk compared with the logical size |
|---|---|
| Ordinary files | Slightly **more**: space is handed out in clusters (usually 4 KB), so a 1-byte file still takes 4 KB. Millions of tiny files can add up to several GB of slack. |
| Very small files on NTFS | Can be **less**: files under about 700 bytes may live inside the file table itself. Filescape counts them as one cluster. |
| Compressed files (NTFS compression, APFS, Btrfs) | **Less**. |
| Sparse files (some databases, virtual disks, downloads in progress) | Often much **less**: the unused parts aren't stored. |
| Cloud files that are only online (Google Drive, OneDrive, iCloud Drive, Dropbox, Box…) | Almost **nothing** until downloaded. See [Cloud files](cloud-files.md). |
| Hard links (the same file under several names) | Counted **once** on disk, at the first name the scan finds, as `du` does. The logical size counts every name. On Windows, hard links are counted once per name in both sizes. |
| APFS clones and other copy-on-write copies | Each copy is counted in full, as `du` and Finder do, although the copies share their blocks until one is changed. Deleting one copy frees less than its size on disk. |

Size on disk isn't always what deleting frees: shared blocks (clones), Time Machine local
snapshots and other filesystem snapshots can keep the space in use after a file is deleted.

### When size on disk isn't known

Analyses of folders on servers (SFTP, FTP, WebDAV) and snapshots saved by earlier versions of
Filescape have no sizes on disk. Their sizes on disk are shown as the logical sizes, and
**On disk** says so in its tooltip; the hover text says `size on disk unknown`. When Windows
can't report a cloud or compressed file's size on disk, the hover text says `size on disk
partly estimated`.

## Folder size

A folder's size is the total of every file inside it, at any depth, by the size chosen with the
switch. Folders themselves add nothing (so a folder full of empty subfolders can be slightly
smaller than `du` reports, which also counts the folders' own entries). Files left out by [scan exclusions](../filtering/scan-filters.md) or that couldn't be
read aren't included.

## Percentages

**% of parent** in the file manager is the item's size divided by the size of the folder that
contains it, both by the size the switch shows. So a folder at 40% holds 40% of its parent folder, not 40% of the drive. The top
item (what you scanned) is 100%.

The **%** column in the File types list is a share of the whole scan (or of the matching files
in **Show matches only**). The filter count, for example "1,204 files match · 82.1 GB (12.4% of
the scan)", is also a share of the whole scan.

## File counts

**Files** counts files only, at any depth below the folder; folders aren't counted. The status
bar shows the total for the scan.

## Dates

**Last change** is a file's modification time. For a folder it's the **newest** modification
time of any file inside it, so a folder full of old files shows a recent date if one file in it
changed today. Times are shown in your computer's time zone.

## Units: binary and decimal

By default sizes use **binary units**, like Windows Explorer: 1 KB = 1,024 bytes,
1 MB = 1,024 KB, 1 GB = 1,024 MB, labelled KB/MB/GB. Drive makers and macOS use **decimal
units** (1 kB = 1,000 bytes), which make the same number of bytes look about 7% bigger at GB
scale. Choose under **⚙ Settings > General > Size units**. A 1 TB drive is about 931 GB in binary units.

Typed sizes follow the same setting: `500 MB` in a size box is 500 × 1,024 × 1,024 bytes in
binary mode.

## Related topics

- [Cloud files](cloud-files.md)
- [Why is free space lower than expected?](../tutorials/missing-space.md)
- [Links, junctions and mounted drives](links-and-mounts.md)
- [Interpreting scan results](../getting-started/results.md)
