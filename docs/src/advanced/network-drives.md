# External and network drives

## External drives

USB and Thunderbolt drives scan like internal ones, at the speed of the drive. They appear in the
drive list once connected; press **🖴 Drives…** to refresh it. On Linux and macOS they're
usually mounted under `/media`, `/run/media` or `/Volumes` and are left out when scanning `/` (see
[Links, junctions and mounted drives](../disk-usage/links-and-mounts.md)); scan them directly.

## Network drives and shares

Mapped network drives are listed with `[network]` in the drive list. You can also type a network
path: `\\server\share\folder` on Windows, or a mounted path such as `/mnt/nas` on Linux and macOS.

Scanning over a network works, but every folder listing is a request to the server, so speed
depends mostly on the network's delay:

- **Use more scanner threads.** **⚙ Settings > Advanced > Scanner threads** at 32 (or more) lets
  many requests run at once and often makes network scans several times faster.
- **Scan only what you need**, such as one project folder rather than the whole share.
- **Exclude snapshot folders** such as `.snapshot` or `~snapshot` on NAS devices, which can
  multiply the apparent size. Add them under **Folders to skip**.
- **Scan on the server** if you can: running dirmap (or `dirmap --cli`) on the storage server
  itself is far faster than over the network.
- A share that disconnects mid-scan shows its remaining folders as unreadable. Scan again.

dirmap looks up the drive list in the background, so a slow or disconnected share doesn't freeze
the window.

## Sizes on network storage

Network storage often compresses, deduplicates or snapshots data. dirmap shows file sizes as the
share reports them, which can be very different from the space used on the server's disks. Use
the storage system's own tools for its physical usage.

## Related topics

- [Scanning millions of files](millions-of-files.md)
- [Settings reference](../settings/reference.md)
- [Render farms and VFX projects](vfx.md)
