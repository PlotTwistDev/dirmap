# How sizes are calculated

## File size

dirmap uses each file's **size**: the number of bytes in it, the same figure Explorer shows as
"Size" and `ls -l` shows on Linux and macOS. It doesn't read the files to get it; the size comes
from the folder listing, which is why scanning is fast.

## File size versus space on disk

The space a file actually occupies on the drive ("Size on disk" in Explorer) can be different:

| Situation | Space on disk compared with the size dirmap shows |
|---|---|
| Ordinary files | Slightly **more**: space is handed out in clusters (usually 4 KB), so a 1-byte file still takes 4 KB. Millions of tiny files can add up to several GB of slack. |
| Very small files on NTFS | Can be **less**: files under about 700 bytes may live inside the file table itself. |
| Compressed files (NTFS compression, APFS, Btrfs) | **Less**. dirmap shows the uncompressed size. |
| Sparse files (some databases, virtual disks, downloads in progress) | Often much **less**: the unused parts aren't stored. dirmap shows the full size. |
| Cloud placeholders (OneDrive, iCloud, Dropbox "online-only" files) | Almost **nothing** until downloaded. dirmap shows the full size. |
| Hard links (the same file under several names) | dirmap counts the file once **for each name**; the disk stores it once. Windows uses hard links heavily in `C:\Windows\WinSxS`. |

dirmap currently has no "size on disk" mode, so treat its numbers as "how much data is here",
not "how much this would free". See [Feature status](../about/feature-status.md).

## Folder size

A folder's size is the total of every file inside it, at any depth. Folders themselves add
nothing. Files left out by [scan exclusions](../filtering/scan-filters.md) or that couldn't be
read aren't included.

## Percentages

**% of parent** in the folder list is the item's size divided by the size of the folder that
contains it. So a folder at 40% holds 40% of its parent folder, not 40% of the drive. The top
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
scale. Choose under **⚙ Settings > General > Sizes**. A 1 TB drive is about 931 GB in binary units.

Typed sizes follow the same setting: `500 MB` in a size box is 500 × 1,024 × 1,024 bytes in
binary mode.

## Related topics

- [Why is free space lower than expected?](../tutorials/missing-space.md)
- [Links, junctions and mounted drives](links-and-mounts.md)
- [Interpreting scan results](../getting-started/results.md)
