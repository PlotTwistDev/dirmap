# Scanning millions of files

dirmap is built for drives and storage systems with millions of files. This page explains what
affects speed and memory, and how to get the best from very large scans.

## How fast is scanning?

The scan reads folder listings only, never file contents, using many threads at once. On a local
SSD, expect hundreds of thousands of files per second once the system has the folder listings
cached; the first scan after a restart is slower because they have to come from the disk. Network
storage is usually limited by the network's round-trip time rather than by dirmap; see
[External and network drives](network-drives.md).

The scanning screen shows files per second, so you can compare settings.

## Threads

**⚙ Settings > Advanced > Scanner threads** sets how many folders are read at once. **0** (the
default) uses one per processor core, which is right for local drives. For network drives, where
each request spends most of its time waiting, more threads (16, 32 or 64) often help a lot.

## Memory

The whole scan is kept in memory, so everything after the scan is instant. Each file and folder
takes roughly 100–150 bytes, plus its name. In practice:

| Files scanned | Approximate memory |
|---|---|
| 100,000 | 20–40 MB |
| 1 million | 150–250 MB |
| 3.5 million | 0.5–0.9 GB |
| 10 million | 1.5–2.5 GB |

Some features hold extra data while they're in use:

- **Show matches only** builds a filtered copy of the part of the tree that matches.
- **Compare with snapshot** loads the snapshot's whole tree as well.
- **Find duplicates** keeps a list of candidate files while it runs.
- While a scan is finishing, the live tree and the final tree briefly exist side by side.

When you scan again, the previous scan is freed in the background.

## Tips for very large scans

- **Exclude what you don't need** under **⚙ Settings > Filtering**, such as backup folders or
  build caches with millions of tiny files. They're then not even read.
- **Turn off the live tree** (**⚙ Settings > General > Show folders and files as they are found**)
  for a slightly faster scan with less redrawing.
- **Filter rather than scroll.** Filtering 3.5 million files takes a fraction of a second because
  it works on memory, in the background.
- **Duplicates:** start with a high minimum size (such as 100 MB), then lower it.
- **Save a snapshot** of a big scan if you want to compare later; it's much smaller on disk than
  in memory.

## Command line

`dirmap --cli PATH` scans without opening a window and prints the largest entries and extensions,
which is handy over SSH or in scheduled jobs. See [Command line](../reference/command-line.md).

## Related topics

- [Working with very large maps](../views/large-maps.md)
- [How dirmap works](architecture.md)
- [System requirements](../getting-started/system-requirements.md)
