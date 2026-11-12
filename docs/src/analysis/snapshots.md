# Comparing with a snapshot

A **snapshot** is a saved copy of a scan's results: every folder and file with its size and date
(not the files' contents). Scan again later and compare with it to see what has changed: what is
new, what was deleted, and what grew or shrank.

## Saving a snapshot

1. Scan the drive or folder.
2. Open the **Changes** tab and press **💾 Save snapshot…**.
3. Choose where to save it. The suggested name includes the folder and the date, for example
   `dirmap Projects 2026-11-12 0930.dmsnap`.

Snapshots are compact (roughly 30–50 bytes per file) and can be saved anywhere. From a script or
scheduled task, use the command line:

```sh
dirmap --cli D:\Projects --save-snapshot D:\Snapshots\projects.dmsnap
```

## Comparing

1. Scan the same drive or folder again (or open dirmap with
   `dirmap --compare projects.dmsnap D:\Projects`).
2. In the **Changes** tab press **📂 Compare with snapshot…** and pick the snapshot file.

The tab then shows:

- what was compared, and when the snapshot was taken, with a warning if the snapshot is of a
  different folder;
- **Then** *size* · **now** *size* · the difference;
- counts of **new**, **deleted**, **grew** and **shrank** items;
- a list of every change, **biggest change first**. New items are tagged **new** (red), deleted
  ones **deleted** (blue), and the amount is shown in orange for growth and blue for shrinkage.
  Hover over a line to see the old and new size; click a line to select that item.

Press **Color the map by change** (or **Color by: Change** in the filter bar) to colour the map:
red for new, orange for grown, blue for shrunk, grey for unchanged. Hovering over the map also
shows how each file changed. See [Colour modes](../views/color-modes.md).

**Stop comparing** closes the comparison. Scanning again while a snapshot is loaded compares the
new scan with it automatically.

## How changes are worked out

- Items are matched **by name** within each folder. A file that was renamed or moved shows as one
  deleted item and one new item.
- A **new folder** is listed once, with its total size, rather than every file in it. The same
  goes for a **deleted folder**.
- A file counts as changed only if its **size** changed. A file edited without changing size
  isn't listed.

## Related topics

- [Colour modes](../views/color-modes.md)
- [Command line](../reference/command-line.md)
- [Why is free space lower than expected?](../tutorials/missing-space.md)
