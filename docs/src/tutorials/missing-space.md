# Why is free space lower than expected?

**Goal:** work out why a drive is fuller than the files you know about, or why dirmap's total
doesn't match the drive's used space.

## 1. Compare dirmap's total with the drive's used space

Open **🖴 Drives…**: the **Used** column is what the drive reports. Compare it with the total in the
status bar after scanning the whole drive. Make sure both use the same units (**⚙ Settings >
General > Sizes**): a "1 TB" drive is 931 GB in binary units.

## 2. If dirmap's total is lower: something wasn't counted

Likely causes, most common first:

| Cause | How to check | What to do |
|---|---|---|
| Folders dirmap couldn't read | The status bar says **N items couldn't be read** | Run as administrator (Windows), give Full Disk Access (macOS) or use `sudo` (Linux). See [Running as administrator](../settings/administrator.md). |
| System restore points and shadow copies (Windows) | Not visible to programs | **System Properties > System Protection > Configure** shows and limits their space. |
| Recycle Bin of other users | `$Recycle.Bin` is unreadable without administrator rights | Run as administrator, or empty the Recycle Bin. |
| Scan exclusions or skipped hidden files | **⚙ Settings > Filtering** and **Advanced** | Clear them and rescan. |
| File system overhead | Cluster slack on millions of small files, the file table itself | Normal; usually a few percent. |
| Files deleted but still open | A program is still writing to a deleted file (common on Linux servers) | Restart the program. |
| Other drives' space (macOS) | APFS volumes share one disk's space | Scan `/` to cover system and data volumes. |

## 3. If dirmap's total is higher: something was counted more than once or isn't really on disk

| Cause | How to check |
|---|---|
| Hard links, especially `C:\Windows\WinSxS` | Very large WinSxS or backup folders. See [Windows system storage](../advanced/windows-storage.md). |
| Compressed or sparse files | Explorer's Properties shows a smaller **Size on disk**. |
| Cloud "online-only" files (OneDrive, iCloud, Dropbox) | They show their full size but use almost no space. |
| Drives mounted inside the folder (Linux, macOS) | Only if **Stay on the scanned drive** is off. |

## 4. Find what's using the space

1. Look at the largest folders in the folder list, and the largest tiles on the map.
2. Check the usual suspects:
   - Virtual disks: **Show: 💿 Disk images** (WSL, Docker, virtual machines). See
     [Investigate WSL virtual disks](wsl-disks.md).
   - `pagefile.sys`, `hiberfil.sys` and `swapfile.sys` at the top of `C:\`.
   - Caches and temporary files: `AppData\Local\Temp`, browser caches, `~/.cache`.
   - Downloads, old installers (`*.iso, *.msi, *.exe` in a Custom filter) and duplicate files.
   - Large logs and crash dumps: `*.log, *.dmp`.
3. **Color by: Age** to spot data that hasn't been touched in years.
4. Save a snapshot now, and compare with a new scan next week to see what keeps growing. See
   [Comparing with a snapshot](../analysis/snapshots.md).

## Related topics

- [How sizes are calculated](../disk-usage/sizes.md)
- [Windows system storage](../advanced/windows-storage.md)
- [Troubleshooting](../troubleshooting/index.md)
