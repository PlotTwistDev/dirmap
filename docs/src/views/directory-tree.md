# The directory tree

The folder list at the top of the window shows what you scanned as a tree, like Explorer's
folder pane, with sizes.

## Columns

| Column | Meaning |
|---|---|
| **Name** | File or folder name, with its icon (Explorer's own icons on Windows). A folder's arrow opens and closes it. |
| **Size** | The file's size, or the total of everything inside the folder. |
| **% of parent** | Share of the folder that contains it, with a bar. Folder bars are blue; file bars use the file's colour. |
| **Files** | For folders: the number of files inside, at any depth. |
| **Last change** | When the file was modified; for a folder, the newest change of anything inside it. |

**% of parent**, **Files** and **Last change** can be hidden under
**⚙ Settings > Folder list**. Name and Size are always shown.

## Sorting

Click a column heading to sort by it; click it again to reverse the order. An arrow (⏷ or ⏶)
shows the current sort. Name sorts A to Z first; Size, Files and Last change sort largest or
newest first. **% of parent** sorts by size. The order applies within each folder.

## Opening folders and selecting

| Action | What it does |
|---|---|
| Click a row | Selects it. The map outlines it. |
| Click the arrow, or double-click a folder | Opens or closes the folder. |
| Right-click a row | Selects it and opens the [right-click menu](../analysis/file-operations.md). |

Selecting something anywhere else (on the map, in the Duplicates or Sequences tab, in a
**Find all** list) opens the tree to show it and scrolls it into view.

Only the rows on screen are drawn, so folders with hundreds of thousands of items open
instantly.

## Highlight mode

When a filter is on in **Highlight** mode, files that don't match, and folders with no matches
anywhere inside, are shown faintly in the tree, as on the map. In **Show matches only** mode the tree lists only the matching files and the
folders that hold them. See [Filtering overview](../filtering/overview.md).

## During a scan

While a scan is running, the tree fills in live (unless turned off in **⚙ Settings > General**).
Click a folder to open it. A spinner marks folders still being read, each open folder shows its
200 largest files so far, and rows stay still while the pointer is over the list so they don't
jump about as sizes change. The tree can't be sorted and the right-click menu isn't available
until the scan has finished.

## Related topics

- [Right-click menus and file operations](../analysis/file-operations.md)
- [Keyboard shortcuts](../reference/keyboard.md)
- [How sizes are calculated](../disk-usage/sizes.md)
