# Virtual disk files (VHDX, VDI, VMDK)

Virtual machines, WSL, Docker and some backup tools keep a whole disk inside one large file. dirmap
shows each of these as a single rectangle, often one of the biggest on the drive.

| Extension | Used by |
|---|---|
| `.vhdx`, `.vhd`, `.avhdx` | Hyper-V, WSL 2 (`ext4.vhdx`), Docker Desktop, Windows Backup |
| `.vdi` | VirtualBox |
| `.vmdk` | VMware, also VirtualBox |
| `.qcow2`, `.img` | QEMU/KVM, Android emulator |
| `.dmg` | macOS disk images |
| `.iso`, `.wim`, `.esd` | Installation media and Windows images |

All of these are in the **💿 Disk images** category, so **Show: 💿 Disk images** lights them all
up at once.

## Why virtual disks are often bigger than what's inside

Most virtual disks are **dynamically expanding**: the file grows as the virtual machine writes
data, but **doesn't shrink** when data is deleted inside it. A WSL distribution that once held
100 GB of build files keeps a 100 GB `ext4.vhdx` after you delete them. The size dirmap shows is
the real size of the file on your drive; the space used *inside* it can be much less.

To get the space back you have to compact the disk with the virtual machine's own tools (for
example `Optimize-VHD` in PowerShell, `diskpart`'s `compact vdisk`, or `VBoxManage modifymedium
--compact`). dirmap doesn't do this itself; see [Feature status](../about/feature-status.md).

**Snapshots and checkpoints** (`.avhdx` files, VirtualBox `Snapshots` folders, VMware
`-000001.vmdk` files) record changes since a snapshot was taken and can grow as large as the
disk itself.

## Finding them

1. Scan the drive.
2. Click **Show: 💿 Disk images**. Everything else is dimmed.
3. Hover over the bright rectangles to see their paths, or use the **File types** tab:
   right-click `.vhdx` and choose **🔍 Find all .vhdx files** for a list, largest first.

See [Investigate WSL virtual disks](../tutorials/wsl-disks.md) for a step-by-step walkthrough.

## Related topics

- [Investigate WSL virtual disks](../tutorials/wsl-disks.md)
- [Windows system storage](../advanced/windows-storage.md)
- [File categories](../filtering/categories.md)
