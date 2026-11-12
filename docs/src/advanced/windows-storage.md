# Windows system storage

Scanning `C:\` shows several large items that belong to Windows. This page explains the common
ones, how dirmap counts them, and what (if anything) is safe to do about them.

**Never delete Windows files from dirmap.** Use **Settings > System > Storage** (Storage Sense,
Cleanup recommendations) or **Disk Cleanup** instead; they know what's safe.

| Item | What it is | Notes |
|---|---|---|
| `pagefile.sys` | Virtual memory | Size is set in **System Properties > Advanced > Performance > Virtual memory**. |
| `hiberfil.sys` | Hibernation and Fast Startup | `powercfg /hibernate off` removes it (and disables Fast Startup). |
| `swapfile.sys` | Memory for Store apps | Usually small. |
| `C:\Windows\WinSxS` | Component store: copies of system files for updates and repairs | Mostly **hard links** to files in `System32`, so dirmap (and Explorer) count the same files twice. Its real extra size is much smaller. `Dism /Online /Cleanup-Image /AnalyzeComponentStore` reports the true figure. |
| `C:\Windows\Installer` | Cached installers needed to repair or uninstall programs | Don't delete; uninstall programs instead. |
| `C:\Windows\SoftwareDistribution` | Downloaded Windows updates | Cleaned by Disk Cleanup > Windows Update Cleanup. |
| `System Volume Information` | Restore points and shadow copies | Usually unreadable even as administrator, so not counted. Managed under **System Protection**. |
| `$Recycle.Bin` | Every user's Recycle Bin | Other users' bins need administrator rights to read. |
| `C:\ProgramData` | Data shared by all users' programs | Often holds large caches (for example Windows Defender, NVIDIA, Docker). |
| `C:\Users\<you>\AppData` | Programs' per-user data and caches | Hidden by Windows, included by dirmap. `AppData\Local\Temp` is safe to clear when programs are closed. |
| `C:\Windows.old` | The previous Windows after an upgrade | Removed by **Settings > System > Storage > Temporary files > Previous Windows installation(s)**. |

## Hidden and system files

dirmap includes hidden and system files by default; Explorer hides them. That's why a folder can
look bigger in dirmap. **⚙ Settings > Advanced > Skip hidden files and folders** leaves them out.

## Running as administrator

Many of these folders are partly unreadable without administrator rights, so a scan of `C:\`
without them misses a lot. See [Running as administrator](../settings/administrator.md).

## OneDrive

Files that are **online-only** in OneDrive show their full size in dirmap but take almost no
space on the drive. Right-click them in Explorer and choose **Free up space** to make files
online-only, or **Always keep on this device** to download them.

## WSL, Docker and virtual machines

See [Investigate WSL virtual disks](../tutorials/wsl-disks.md).

## Related topics

- [Why is free space lower than expected?](../tutorials/missing-space.md)
- [Hidden, system and unreadable files](../disk-usage/hidden-and-unreadable.md)
- [Links, junctions and mounted drives](../disk-usage/links-and-mounts.md)
