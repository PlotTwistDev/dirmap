# The Files view

Choose **View: Files** in the filter bar to browse the scan like Finder or Windows Explorer:
folders you open by double-clicking, a path you can click, Back and Forward, and the usual list,
icon and column layouts. Unlike a file manager, every item shows how much space it uses and its
share of the folder, and you can sort by size. The Files view takes the place of the map; the
folder list, filters and side panel stay where they are.

![The Files view as a list](../images/file-browser.png)

## Layouts

Pick a layout with the buttons at the top right of the view.

| Layout | What it shows |
|---|---|
| **☰ List** | One line per item, with **Size**, **% of folder** (with a bar), **Kind**, **Items** (files inside a folder, at any depth) and **Last change**. Click a column heading to sort by it. In a narrow window the less important columns are left out. |
| **⊞ Icons** | A grid of large icons with each item's name and size, and a thin bar under the icon showing its share of the folder. |
| **◫ Columns** | One column per folder, from the top of the scan down to the current folder. Selecting a folder shows what's inside it in the next column; selecting a file shows its details there, with **Open** and **Show in Finder** (or Explorer) buttons. |

![The Icons layout](../images/file-browser-icons.png)

![The Columns layout, with a file's details](../images/file-browser-columns.png)

## Moving around

| Action | What it does |
|---|---|
| Double-click a folder | Opens it in the Files view. |
| Double-click a file | Opens it with its usual app. |
| Click a folder in the path | Goes to that folder. |
| **◀** and **▶** | Back and Forward through the folders you've opened. The mouse's side buttons and **Alt+←** / **Alt+→** (**⌥←** / **⌥→** on a Mac) do the same. The ◀ and ▶ buttons on the toolbar work here too. |
| **⏶**, or **Backspace** | Up to the enclosing folder. The folder you were in stays selected. |

The keyboard works as it does in a file manager: **↑** and **↓** move the selection, **Enter**
opens the selected folder, and typing the start of a name jumps to it. In **Icons** the **←**
and **→** keys move along a row; in **Columns** they go up a level and into the selected folder.
The full list is in [Keyboard shortcuts](../reference/keyboard.md#files-view).

## Sorting

The **Sort** menu, or a click on a List column heading, chooses the order. Pick the same one
again to reverse it.

- **Size**, **Items** and **Last change** start with the largest, fullest or newest.
- **Name** and **Kind** sort A to Z with folders first, like Explorer. Numbers in names are
  sorted by value, so `shot2` comes before `shot10`.

The order applies to every layout and every folder you open.

## Selection is shared

The Files view, the folder list and the maps all have the same selection and the same current
folder:

- Selecting something in the Files view selects it in the folder list, which opens to show it.
- Selecting something in the folder list, the Duplicates tab or anywhere else shows it in the
  Files view, opening the folder that contains it if needed.
- Switch to **Treemap** or **Sunburst** and it shows the folder you were browsing, with the
  selection outlined. Switch back and you're where you left off.

The bottom of the view shows how many items the folder holds, their total size, and the
selected item's size and share of the folder.

## Filters

The **Show:** categories, **Larger than** / **Smaller than** and the **🔍 Custom** filter work
here as they do on the maps. See [Filtering overview](../filtering/overview.md).

- With **Highlight**, files that don't match and folders with no matches inside are greyed out.
- With **Show matches only**, they're left out altogether, and folder sizes count only the
  matching files.

The **Color by** setting colours the size bars (and, on macOS and Linux, the file icons) the same
way it colours the map.

## Right-click menu

Right-click any item for the same commands as the folder list: Open, Copy Path, Show in Finder
or Explorer, open a terminal there, Refresh, Properties, Compute Hash, Add to Exclusion Filter,
Delete, Move to and so on. The first item on a folder is **Open Folder**, which opens it in the
Files view. See [Right-click menus and file operations](../analysis/file-operations.md).

The keyboard shortcuts for those commands work on the item selected here, except that **Enter**
on a folder opens it in the Files view rather than in Finder or Explorer.

## Icons

On Windows the Files view uses Explorer's own icons. On macOS and Linux it draws a folder or a
page in the file type's colour, with the extension written on the larger icons.

## Not yet available

The Files view looks at the scan, so it shows what was there when you scanned (or last
refreshed an item). It can't yet select several items at once, rename, create folders or drag
and drop. Use **Show in Finder** or **Show in Explorer** for those.

## Related topics

- [The directory tree](directory-tree.md)
- [The treemap](treemap.md)
- [The sunburst](sunburst.md)
- [Mouse controls](../reference/mouse.md#files-view)
