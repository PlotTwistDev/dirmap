# Frequently asked questions

## Does dirmap change or delete anything by itself?

No. Scanning only reads folder listings. Files are changed only when you choose a right-click
command such as **Delete**, **Empty Folder**, **Remove Empty Folders** or **Move to…**, and the
deleting ones ask first (except Remove Empty Folders, which only removes empty folders).

## Does dirmap send anything over the internet?

No. dirmap has no online features, and the help is built in.

## Why doesn't dirmap's total match Windows?

dirmap adds up file sizes; Windows reports space used on the disk, including things programs
can't see. See [Why is free space lower than expected?](../tutorials/missing-space.md).

## Can I see the size on disk instead of the file size?

Not yet. See [Feature status](../about/feature-status.md) and [How sizes are calculated](../disk-usage/sizes.md).

## Is it safe to delete what dirmap shows?

dirmap tells you what uses space, not what's safe to remove. Don't delete files in system or
program folders (`C:\Windows`, `Program Files`, `/usr`, `/System`, `/Library`) unless you know
what they are. Prefer **Delete (to Recycle Bin)** so you can undo.

## Why is a second scan so much faster?

The operating system caches folder listings, so a second scan of the same drive soon after is
usually much faster. Network drives and drives with antivirus scanning on access are slower.

## How many files can dirmap handle?

Millions. It's tested with synthetic scans of 3.5 million files. Memory is the limit: see
[Scanning millions of files](../advanced/millions-of-files.md).

## Does dirmap follow shortcuts, links or junctions?

No, never; nothing is counted twice through a link. Hard links are the exception, because they
can't be told apart from ordinary files. See [Links, junctions and mounted drives](../disk-usage/links-and-mounts.md).

## Can dirmap watch a drive and update automatically?

No; a scan is a picture of one moment. Refresh a folder with **F5**, or compare scans with
[snapshots](../analysis/snapshots.md).

## Why are some rectangles grey?

Grey tiles are files whose extension has no colour assigned (rare) or, with **Color by: Change**,
files that haven't changed. Files hidden by a filter leave empty dark background instead.

## Where are my settings stored, and how do I reset them?

See [Where settings are stored](../settings/reference.md#where-settings-are-stored). **Reset all** in
the Settings window restores the defaults.

## Can I use dirmap without a mouse?

Partly. File commands, the Custom filter and the treemap's zoom, Home and Back/Forward have
keyboard shortcuts, but moving through the folder list with the arrow keys isn't supported yet.
See [Keyboard shortcuts](../reference/keyboard.md).

## Is there a portable version?

dirmap is always portable: it's a single file. Its settings are kept in your user profile (see
above), not next to the program.

## Related topics

- [Troubleshooting](index.md)
- [Feature status](../about/feature-status.md)
