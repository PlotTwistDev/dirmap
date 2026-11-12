# Links, junctions and mounted drives

## Symbolic links and junctions are never followed

A **symbolic link** (or a Windows **junction**) is a signpost to another folder or file. If
dirmap followed them, the same files could be counted twice, or a link pointing to a folder
above it could make a scan go round in circles. So dirmap **never follows links**: the link
itself is skipped, and the files it points to are counted only where they really are (if
that place is inside what you scanned).

Examples:

- `C:\Users\All Users` and `C:\Documents and Settings` are junctions; they are not counted.
- On macOS, `/tmp` is a link to `/private/tmp`; its files are counted under `/private/tmp`.
- A `node_modules` folder full of links (as made by pnpm) counts only the real files.

## Hard links

A **hard link** is a second name for the same file, not a signpost. Nothing on disk tells the
two names apart, so dirmap counts the file under **each** name. This mainly matters for
`C:\Windows\WinSxS`, which shares most of its files with `C:\Windows\System32` through hard links,
and for some backup tools (such as Time Machine snapshots and `rsync --link-dest` backups). The
total for those folders can be much larger than the space they really use. See
[Windows system storage](../advanced/windows-storage.md).

## Other drives mounted inside a folder

On **Linux and macOS**, other drives often appear inside the folder tree, for example a USB
disk at `/media/you/USB` or a network share at `/mnt/nas`. By default dirmap **stays on the
drive you scan** and skips them, so scanning `/` doesn't scan every connected disk. To include
them, turn off **⚙ Settings > Advanced > Stay on the scanned drive** and rescan, or start dirmap
with `--all-fs`. System folders such as `/proc` and `/sys` are separate file systems and are
skipped for the same reason.

On **macOS**, scanning `/` covers both the read-only system volume and the data volume (they
appear as one disk in Finder) without counting anything twice.

On **Windows**, a drive mounted in a folder (instead of a letter) is reached through a
junction-like mount point, which is never followed. Scan its own path to include it.

## Related topics

- [How sizes are calculated](sizes.md)
- [External and network drives](../advanced/network-drives.md)
- [Settings reference](../settings/reference.md)
