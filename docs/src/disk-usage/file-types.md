# File types and extensions

## Extensions

A file's type is taken from its **extension**: the part of the name after the last dot. `movie.mp4`
is `.mp4`; `archive.tar.gz` is `.gz`. A name that starts with a dot and has no other dot (such as
`.bashrc`) or has no dot at all (such as `README`) has **no extension**, listed as **(none)**.
Extensions are compared without regard to case, so `.JPG` and `.jpg` are counted together.

## The File types list

The **File types** tab on the right lists every extension found, largest total first:

| Column | Meaning |
|---|---|
| Colour swatch | The colour of these files on the treemap (in **Color by: Type**). |
| **Type** | The extension, or **(none)**. |
| **Size** | Total size of all files with this extension. |
| **%** | Share of the whole scan. |
| **Files** | How many there are. |

- **Click** a row to highlight those files on the map: everything else is darkened. Click it
  again (or **Clear highlight** above the list) to stop.
- **Right-click** a row for:
  - **🔍 Find all .ext files**: a window listing every file of that type, largest first. Click one
    to select it in the file manager and on the map.
  - **Highlight in Map** / **Clear Highlight**.
  - **Copy Extension**: copies `.ext` to the clipboard.
  - **Add to Exclusion Filter**: adds `*.ext` to the files skipped by future scans. See
    [Scan exclusions](../filtering/scan-filters.md).

In **Show matches only** mode the list covers only the matching files and says so.

## Colours

The 16 extensions using the most space get bright, clearly different colours. Every other
extension gets a softer colour worked out from its name, so it is the same in every scan.
Colours are assigned when a scan finishes, so the same extension can have a different bright
colour in a different scan.

## Categories

The **Show:** buttons group extensions into kinds such as Audio, Video, Images, AI models and
Disk images. See [File categories](../filtering/categories.md) for exactly which extensions each
one includes.

## Related topics

- [File categories](../filtering/categories.md)
- [Colour modes](../views/color-modes.md)
- [Find every file with one extension](../tutorials/files-by-extension.md)
