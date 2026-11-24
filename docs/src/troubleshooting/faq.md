# Frequently asked questions

## Does Filescape change or delete anything by itself?

No. Scanning only reads folder listings. Files are changed only when you choose a right-click
command such as **Delete**, **Empty Folder**, **Remove Empty Folders** or **Move to…**, and the
deleting ones ask first (except Remove Empty Folders, which only removes empty folders).

## Does Filescape send anything over the internet?

Only what you ask for, and the licence check. The Remote workspace connects to the servers you
choose. Activating a licence, and a check once a week while you're online, send Filescape's licence
server the serial (or licence ID) and an anonymous identifier of the computer, made by hashing its
system ID; nothing about your files is sent. The help is built in. See
[Licensing](../settings/licensing.md).

## What stops working when a licence or the evaluation ends?

Only starting Professional features. The file browser, the disk map and basic filtering stay free,
anything already running finishes, and your files, settings and saved data are kept. See
[Licensing](../settings/licensing.md#free-and-professional-features).

## Why doesn't Filescape's total match Windows?

Filescape adds up file sizes; Windows reports space used on the disk, including things programs
can't see. See [Why is free space lower than expected?](../tutorials/missing-space.md).

## Can I see the size on disk instead of the file size?

Not yet. See [Feature status](../about/feature-status.md) and [How sizes are calculated](../disk-usage/sizes.md).

## Is it safe to delete what Filescape shows?

Filescape tells you what uses space, not what's safe to remove. Don't delete files in system or
program folders (`C:\Windows`, `Program Files`, `/usr`, `/System`, `/Library`) unless you know
what they are. Prefer **Delete (to Recycle Bin)** so you can undo.

## Why is a second scan so much faster?

The operating system caches folder listings, so a second scan of the same drive soon after is
usually much faster. Network drives and drives with antivirus scanning on access are slower.

## How many files can Filescape handle?

Millions. It's tested with synthetic scans of 3.5 million files. Memory is the limit: see
[Scanning millions of files](../advanced/millions-of-files.md).

## Does Filescape follow shortcuts, links or junctions?

No, never; nothing is counted twice through a link. Hard links are the exception, because they
can't be told apart from ordinary files. See [Links, junctions and mounted drives](../disk-usage/links-and-mounts.md).

## Can Filescape watch a drive and update automatically?

No; a scan is a picture of one moment. Refresh a folder with **F5**, or compare scans with
[snapshots](../analysis/snapshots.md).

## Why are some rectangles grey?

Grey tiles are files whose extension has no colour assigned (rare) or, with **Color by: Change**,
files that haven't changed. Files hidden by a filter leave empty dark background instead.

## Where are my settings stored, and how do I reset them?

See [Where settings are stored](../settings/reference.md#where-settings-are-stored). **Reset all** in
the Settings window restores the defaults.

## Can I use Filescape without a mouse?

Mostly. The file manager works entirely from the keyboard, as Finder or Explorer do, and file
commands, the Custom filter and the treemap's zoom, Home and Back/Forward have shortcuts too.
There's no keyboard way to move between the file manager and the map, or to use the side panel.
See [Keyboard shortcuts](../reference/keyboard.md).

## Is there a portable version?

Filescape is always portable: it's a single file. Its settings are kept in your user profile (see
above), not next to the program.

## Related topics

- [Troubleshooting](index.md)
- [Feature status](../about/feature-status.md)
