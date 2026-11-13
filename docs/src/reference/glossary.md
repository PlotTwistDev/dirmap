# Glossary

**Breadcrumbs**: The row of folder names above the map (🏠 *scan* › *folder* › …) showing which folder the map is
showing. Click one to go there.

**Category**: A group of extensions such as Audio or Disk images, used by the **Show:** buttons. See
[File categories](../filtering/categories.md).

**Cushion shading**: The shading on the treemap that makes each folder's area look like a cushion, so the borders
between folders show without drawing boxes.

**Custom filter**: A file-name pattern, such as `*.exr` or `render.####.exr`, that limits the map to matching files.
See [Custom filter syntax](../filtering/syntax.md).

**Duplicate set**: Two or more files with identical contents. See [Duplicate files](../analysis/duplicates.md).

**Extension**: The part of a file name after the last dot, such as `mp4`, used to tell what kind of file it is.

**Frame number**: The number in the name of each file of a numbered sequence, such as `1001` in
`shot010.1001.exr`. Matched with `#`, `{frame}` or `[1001-1100]`.

**Hard link**: A second name for the same file. dirmap counts the file under each name.

**Highlight mode**: Shows filter matches in colour and hides everything else, keeping the map's layout.

**Junction / symbolic link**: A signpost to another folder or file. dirmap never follows them.

**Padding**: Leading zeros that give frame numbers a fixed width: `0042` is padded to four digits.

**Scan**: Reading a drive or folder to find every file and its size. The result stays in memory until you
scan again or close dirmap.

**Scan exclusions**: Settings that leave folders or files out of a scan entirely. See
[Scan exclusions](../filtering/scan-filters.md).

**Selection**: The one file or folder last clicked, shown highlighted in the folder list and outlined on the
map. Keyboard shortcuts act on it.

**Sequence**: A set of numbered files in one folder, such as rendered frames, that differ only in their frame
number. See [Numbered sequences](../filtering/sequences.md).

**Show matches only**: Rebuilds the map and folder list from the files that match the filter.

**Snapshot**: A saved copy of a scan's results (`.dmsnap` file) for comparing later. See
[Comparing with a snapshot](../analysis/snapshots.md).

**Sunburst**: A ring chart of the scan, each ring one folder level deeper.

**Files view**: A Finder- or Explorer-style browser of the scan (List, Icons or Columns), chosen
with **View: Files**.

**Tile**: One rectangle on the treemap, standing for one file (or, when too small to draw, a group of
very small files).

**Treemap**: The main map, where every file is a rectangle with an area proportional to its size.

**Unreadable**: A folder or file dirmap wasn't allowed to read, so isn't included in the totals.

## Related topics

- [Understanding the interface](../getting-started/interface.md)
- [Help Centre](../index.md)
