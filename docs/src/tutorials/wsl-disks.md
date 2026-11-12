# Investigate WSL virtual disks

**Goal:** find out how much space WSL (Windows Subsystem for Linux), Docker Desktop and other
virtual machines use on a Windows drive.

WSL 2 keeps each Linux distribution's files in one virtual disk file called `ext4.vhdx`. Docker
Desktop keeps its images in a similar file. These files grow as you use them but **don't shrink**
when you delete files inside Linux, so they are a common cause of disappearing space.

## Steps

1. Scan your system drive (usually `C:\`), ideally as administrator so nothing is left out.
2. Click **Show: 💿 Disk images** in the filter bar. Every virtual disk lights up on the map.
3. Hover over each bright rectangle to see its path and size. Typical locations:
   - `C:\Users\<you>\AppData\Local\Packages\<distribution>\LocalState\ext4.vhdx` (WSL distributions
     from the Microsoft Store)
   - `C:\Users\<you>\AppData\Local\wsl\<id>\ext4.vhdx` (newer WSL installs)
   - `C:\Users\<you>\AppData\Local\Docker\wsl\` (Docker Desktop)
   - `C:\ProgramData\Microsoft\Windows\Virtual Hard Disks\` (Hyper-V)
4. For a list, open the **File types** tab, right-click **.vhdx** and choose
   **🔍 Find all .vhdx files**.

## What to do about a large ext4.vhdx

dirmap shows the size of the virtual disk file on your Windows drive. To see what's using space
*inside* it, run a disk usage tool inside Linux (for example `du -sh /*`), or scan the
distribution's files from Windows through the `\\wsl$\<distribution>` path (this works as a
network path and is slower).

After deleting files inside Linux, the `.vhdx` stays the same size until it's compacted. A common
way is, in PowerShell as administrator:

```text
wsl --shutdown
Optimize-VHD -Path "<path to ext4.vhdx>" -Mode Full
```

`Optimize-VHD` needs the Hyper-V PowerShell module; otherwise `diskpart` can do the same with
`select vdisk file="<path>"` and `compact vdisk`. Recent WSL versions can also shrink disks
automatically when sparse mode is turned on (`wsl --manage <distribution> --set-sparse true`).
Rescan afterwards (select the file and press **F5**) to see the new size.

dirmap doesn't compact virtual disks itself; see [Feature status](../about/feature-status.md).

## Related topics

- [Virtual disk files (VHDX, VDI, VMDK)](../disk-usage/virtual-disks.md)
- [Windows system storage](../advanced/windows-storage.md)
- [Running as administrator](../settings/administrator.md)
