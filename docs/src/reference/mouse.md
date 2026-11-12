# Mouse controls

What each mouse action does in each part of the window. Like the [keyboard shortcuts](keyboard.md),
this list is generated from the program and checked against the code by the tests.

A trackpad works too: two-finger scrolling acts as the wheel, and pinching zooms the treemap.

<!-- BEGIN GENERATED: do not edit by hand; see src/help/reference.rs -->

### Treemap

| Input | Action |
|---|---|
| **Hover** | Show the file's path, size and date in the status bar; a faint outline marks its folder. |
| **Wheel forward / back** | Zoom in / out towards the pointer. The spot under the pointer stays put. |
| **Ctrl+wheel (⌘+scroll on a Mac), or pinch** | Zoom too. |
| **Drag (left or middle button)** | Pan when zoomed in. Let go while moving to coast. |
| **Click** | Select the file (the folder list opens to show it). |
| **Double-click** | Open the folder that holds the file (or frame the file if it's already in the folder on screen). |
| **Right-click** | Zoom out one level: to the folder around the view, then to the parent folder. |
| **Back / forward mouse buttons** | Back and Forward through places you've looked at. |

### Sunburst

| Input | Action |
|---|---|
| **Hover** | Show the item's path, size and date in the status bar. |
| **Click a segment** | Select it. |
| **Double-click a segment** | Make that folder (or the file's folder) the centre. |
| **Wheel forward** | Go into the top-level folder under the pointer. |
| **Wheel back** | Go up one level. |
| **Click the middle, or right-click** | Go up one level. |

### Folder list

| Input | Action |
|---|---|
| **Click** | Select the item. |
| **Click the arrow, or double-click a folder** | Open or close the folder. |
| **Right-click** | Select the item and open the right-click menu. |
| **Click a column heading** | Sort by that column; click again to reverse. |
| **Drag the border below the list** | Give the list or the map more room. |

### Side panel and other lists

| Input | Action |
|---|---|
| **Click a file type** | Highlight that type on the map; click again to stop. |
| **Right-click a file type** | Find all files of that type, highlight, copy or exclude it. |
| **Click a size range (Sizes tab)** | Show only files in that range; click again to stop. |
| **Click a change, duplicate or listed file** | Select it in the folder list and on the map. |
| **Click a sequence** | Filter the map to that sequence and select its folder. |
| **Drag the panel's left edge** | Make the side panel wider or narrower. |

### Size boxes

Larger than, Smaller than, and the Duplicates minimum.

| Input | Action |
|---|---|
| **Drag left or right** | Change the size. |
| **Click, then type** | Enter a size such as 500 MB or 2 GB (a plain number means MB; 0 turns it off). |

<!-- END GENERATED -->

## Related topics

- [Keyboard shortcuts](keyboard.md)
- [Moving around the treemap](../views/treemap-navigation.md)
- [The sunburst](../views/sunburst.md)
- [The folder list](../views/directory-tree.md)
