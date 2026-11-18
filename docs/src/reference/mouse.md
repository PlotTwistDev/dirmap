# Mouse controls

What each mouse action does in each part of the window. Like the [keyboard shortcuts](keyboard.md),
this list is generated from the program and checked against the code by the tests.

A trackpad works too: two-finger scrolling acts as the wheel, and pinching zooms the treemap.

<!-- BEGIN GENERATED: do not edit by hand; see src/help/reference.rs -->

### Files workspace

| Input | Action |
|---|---|
| **Click** | Select the item. |
| **Ctrl+click (⌘-click on a Mac)** | Add the item to the selection, or take it out. |
| **Shift+click** | Select everything from the last item clicked to this one. |
| **Double-click** | Open a folder, or a file with its usual app. |
| **Middle-click a folder, place or tab** | Open the folder or place in a new tab; close the tab. |
| **Right-click** | The item's menu: Open, Open With, Copy, Cut, Paste, Rename (several items: Batch Rename), Move to Trash, Colour, Tags and Note, Copy Path, Add to Favourites, Show in Explorer or Finder, Properties, Analyse This Folder and, with two panes, Copy or Move to the other pane. On empty space: New, Paste, Undo, Redo, Refresh and more. |
| **Drag onto a folder, a tab, a folder in the path, the other pane or a sidebar place** | Move the items there when it's on the same drive, copy them when it's another drive. Hold Ctrl or Alt to copy (⌥ on a Mac), Shift to move (⌘ on a Mac). The label by the pointer says which. Hold over a tab to switch to it. |
| **Ctrl+wheel over the icons (⌘+scroll on a Mac), or pinch** | Icons view: make the icons bigger or smaller. |
| **Click the view buttons at the bottom right of the list** | Show the folder as Details (rows), Icons, Columns or Gallery. Each folder remembers its own view, sort and grouping (View > Remember Each Folder's View). |
| **Click an item in a column at the left (Columns view)** | Go back to that folder with the item selected; double-click to open it. |
| **Drag a folder onto Favourites** | Bookmark it there; nothing is moved. A line shows where it will go. Drop onto the middle of a favourite to move or copy items into that folder instead. |
| **Drag a favourite** | Reorder it, or move it to another group. Right-click a favourite to rename or remove it; right-click a group's title for New Group, Rename and Move. |
| **Drag the line between two panes** | Share the room between them. Double-click to make them even. |
| **Wheel, drag or double-click on a preview** | Zoom, pan, and switch between fitting the image and showing it 1:1. |
| **Click a column heading** | Sort by that column; click again to reverse. Folders always come first. |
| **Click a folder in the path** | Go there. Click the empty part of the path to type one. Click the arrow after a folder for the folders inside it. |
| **Click a place or drive in the sidebar** | Go there. Right-click for Open in New Tab, Open in Other Pane, Add to Favourites, Copy Path and Analyse. |
| **Back / forward mouse buttons** | Back and Forward. |
| **Drag the sidebar's right edge** | Make the sidebar wider or narrower. |
| **Drag a tab** | Move it along the tab strip. Right-click a tab for New Tab, Close Tab and Close Other Tabs. |
| **Drag a divider between column headings** | Resize the column to its right. Double-click the divider to reset it. |

### Sequence player

| Input | Action |
|---|---|
| **Double-click a sequence** | Play it. Select two sequences and press Enter to compare them (A/B). |
| **Click or drag in the timeline** | Go to that frame. |
| **Mouse wheel** | Zoom in or out around the pointer. Double-click to fit. |
| **Drag the picture** | Move it around when zoomed in. |
| **Drag the wipe line** | A/B: move the line between the two sequences. |

### Analysis dock

The treemap or sunburst under the list in the Files workspace. Wheel, drag and the + − Home keys work as in Storage Analysis.

| Input | Action |
|---|---|
| **Click a tile** | Select it. The item in the list that holds it is selected too. |
| **Double-click a tile** | Open its folder in the list; the map follows. |
| **Right-click a tile** | The same menu as in the list, plus Open in List or Show in List. |
| **Click a file type under the map** | Highlight those files on the map; click again to stop. |
| **Drag the line above the dock** | Give the list or the map more room. |

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

### File manager

| Input | Action |
|---|---|
| **Hover** | Show the item's path, size and date in the status bar. |
| **Click** | Select the item (also on the maps). |
| **Ctrl+click (⌘-click on a Mac)** | Add the item to the selection, or take it out. |
| **Shift+click** | Select everything from the last item clicked to this one. |
| **Double-click** | Open a folder (the map follows), or a file with its usual app. |
| **Middle-click a folder** | Open it in a new tab. |
| **Right-click** | Select the item and open the right-click menu. On empty space: New Folder, Paste, Open in New Tab. |
| **Drag onto a folder, a folder in the path, a tab or empty space** | Move the selected items there when it's on the same drive, copy them when it's another drive. Hold Ctrl or Alt to copy (⌥ on a Mac), Shift to move (⌘ on a Mac). |
| **Click a column heading (List)** | Sort by that column; click again to reverse. |
| **Click a folder in the path** | Go to that folder. |
| **Click a tab, or middle-click it** | Switch to that tab, or close it. |
| **Back / forward mouse buttons** | Back and Forward through folders you've opened. |
| **Drag the border below the file manager** | Give the file manager or the map more room. |

### Tree layout

The file manager's Tree layout.

| Input | Action |
|---|---|
| **Click** | Select the item. |
| **Click the arrow, or double-click a folder** | Open or close the folder. |
| **Right-click** | Select the item and open the right-click menu. |
| **Click a column heading** | Sort by that column; click again to reverse. |

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
- [The file manager](../views/directory-tree.md)
