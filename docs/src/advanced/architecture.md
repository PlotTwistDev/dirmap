# How dirmap works

This page is for the curious and for anyone judging whether dirmap fits a large or unusual storage
system. You don't need it to use dirmap.

## Scanning

- The scanner walks the folder tree with a pool of threads (one per processor core by default),
  reading each folder's listing once. Sizes and dates come from the listing itself; file contents
  are never read during a scan.
- Links and junctions are never followed. On Linux and macOS each folder's device is checked so
  the scan stays on one file system unless told otherwise.
- As folders are read, their totals are added to every folder above them, which is what lets the
  live tree show correct totals while the scan is still running.
- When the walk finishes, the result is turned into a compact tree sorted by size, and the
  per-extension totals are worked out, on a background thread.

## The data in memory

The finished scan is one tree of files and folders. Each entry holds its name, size, file count,
newest modification time and children. Every view, filter and search works from this tree, never
from the disk, which is why filtering and switching views are immediate even with millions of
files. The cost is memory: see [Scanning millions of files](millions-of-files.md).

## The treemap

- The layout is the **squarified treemap** algorithm (Bruls, Huizing and van Wijk), which keeps
  rectangles close to square. Shading uses **cushion treemaps** (van Wijk and van de Wetering), as
  in WinDirStat.
- The map is laid out once per folder; zooming and panning move a camera over that layout rather
  than laying it out again. Only the part in view is turned into tiles, down to about one pixel,
  on background threads; between rebuilds the graphics card simply redraws the existing tiles
  through a new transform.
- Tiles are drawn on the GPU (through wgpu, or OpenGL as a fallback), with shading, the pointer
  light and the bevel worked out per pixel. If neither is available, a slower CPU renderer draws
  images instead.

## Filters, duplicates and sequences

- Filters are compiled once and run against the in-memory tree on a background thread. A newer
  filter cancels the work for an older one. **Show matches only** builds a reduced tree with a map
  back to the original, so the selection and position carry over.
- The duplicate finder groups files by size, then by a hash of the first 64 KB, then by a hash of
  the whole file, reading in parallel. See [Duplicate files](../analysis/duplicates.md).
- Sequence detection groups numbered files by folder and name pattern from the in-memory tree,
  without reading the disk.

## The help system

This manual is a set of Markdown files in `docs/src`, kept in the same repository as the code
and versioned with it. A build script bundles every page and image into the program, so the
Help Centre works offline and always describes the version you're running. The same files build
the online version with mdBook (`mdbook build docs`).

When dirmap changes, the help is updated in the same change:

| To add | Do this |
|---|---|
| A page | Write it under `docs/src` and add it to `SUMMARY.md`, which is the table of contents. |
| A setting | Add a row to the [Settings reference](../settings/reference.md) with a hidden `<!-- setting:field_name -->` marker. |
| A shortcut or mouse action | Add it to `src/help/reference.rs` with the code that implements it, then run `DIRMAP_UPDATE_DOCS=1 cargo test help` to regenerate the [keyboard](../reference/keyboard.md) and [mouse](../reference/mouse.md) tables. |
| A help link or F1 target | Add the page (and `#heading`) to `src/help/topics.rs` and use the constant. |
| A filter example | Add a row to one of the **Pattern, Matches, Doesn't match** tables, or a ` ```filter ` code block. |

`cargo test` (which runs on every push) fails if any of this drifts:

- a page is missing from the table of contents, or listed but missing;
- a link, heading anchor or image doesn't resolve, or an image is unused;
- a setting or Custom filter preference has no documentation, or documentation for one that's gone;
- a shortcut in the reference no longer matches the code, or the generated tables are out of date;
- a filter example doesn't compile, or doesn't match (or not match) the names the docs say;
- the File categories table disagrees with the program;
- a help link used by the app points at a page or heading that doesn't exist.

Screenshots are real captures of the program. To retake one, run dirmap on a sample folder,
capture the window and replace the PNG with the same name; the tests check every image is a
readable PNG that a page uses.

## Technical limitations

- **Logical sizes only.** No allocated size, compression, sparse-file or hard-link awareness yet.
- **No change tracking.** A scan doesn't update itself when files change; refresh with **F5** or
  scan again. Windows' NTFS change journal and MFT reading, which could make rescans much faster,
  aren't used.
- **One scan at a time.** Opening another scan replaces the current one.
- **Memory-resident.** Very large scans need proportionally more memory.
- **File names that aren't valid Unicode** are shown with replacement characters, and file
  operations on them may fail.

## Related topics

- [Scanning millions of files](millions-of-files.md)
- [Feature status](../about/feature-status.md)
- [How sizes are calculated](../disk-usage/sizes.md)
