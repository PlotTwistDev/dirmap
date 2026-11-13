# Keyboard shortcuts

Every keyboard shortcut dirmap has. This list is generated from the program itself and checked
against the code by the tests, so it only lists shortcuts that really work in this version.

On macOS, **⌘** (Command) takes the place of **Ctrl**. Shortcuts that act on the map or on the
selected item don't work while the cursor is in a text box (the folder box, the Custom filter or a
search box), so typing there is never mistaken for a command.

<!-- BEGIN GENERATED: do not edit by hand; see src/help/reference.rs -->

### Anywhere

In the main window, unless the cursor is in a text box.

| Windows and Linux | macOS | Action |
|---|---|---|
| **F1** | **F1** | Open help for the part of the window under the pointer. |
| **Ctrl+F** | **⌘F** | Open the Custom filter and put the cursor in it. |

### Treemap

When the treemap is showing (not the sunburst), the cursor isn't in a text box, and you last clicked the map (or the file manager is in its Tree layout).

| Windows and Linux | macOS | Action |
|---|---|---|
| **+ or =** | **+ or =** | Zoom in, around the pointer (or the middle of the map). |
| **−** | **−** | Zoom out, around the pointer (or the middle of the map). |
| **Home** | **Home** | Whole map: back to the complete scan. |
| **Alt+←** | **⌥←** | Back to the previous place. |
| **Alt+→** | **⌥→** | Forward again. |

### File manager

After you've clicked in the file manager (not the map), in the List, Icons and Columns layouts, unless the cursor is in a text box.

| Windows and Linux | macOS | Action |
|---|---|---|
| **↑ / ↓** | **↑ / ↓** | Select the item above or below (in Icons, the row above or below). |
| **← / →** | **← / →** | Icons: the previous or next item. Columns: up to the enclosing folder, or into the selected folder. |
| **Home / End** | **Home / End** | Select the first or last item. |
| **Page Up / Page Down** | **Page Up / Page Down** | Move 20 items (20 rows of icons). |
| **Enter** | **Enter** | Open the selected folder (the map follows). A selected file opens with its usual app. |
| **Ctrl+↓** | **⌘↓** | Open the selected folder or file. |
| **Backspace or Ctrl+↑** | **Backspace or ⌘↑** | Up to the enclosing folder, selecting the one you were in. |
| **Alt+←** | **⌥←** | Back to the previous folder. |
| **Alt+→** | **⌥→** | Forward again. |
| **Ctrl+A** | **⌘A** | Select everything in the folder. |
| **Type a name** | **Type a name** | Jump to the first item whose name starts with what you type. |
| **Ctrl+T** | **⌘T** | New tab, showing the same folder. |
| **Ctrl+W** | **⌘W** | Close the tab. |
| **Ctrl+Tab / Ctrl+Shift+Tab** | **⌃Tab / ⌃⇧Tab** | Next or previous tab. |
| **Ctrl+Shift+N** | **⌘⇧N** | New folder in the current folder, ready to name. |
| **Ctrl+V** | **⌘V** | Paste: copy or move what you copied or cut into the current folder. Paths copied as text in another app are copied in too. |

### Selected file or folder

Act on the selected item, wherever it was selected (file manager, map or a list), unless the cursor is in a text box. In the file manager they act on every selected item. The same commands are in the right-click menu.

| Windows and Linux | macOS | Action |
|---|---|---|
| **Enter** | **Enter** | Open it with its usual app (a folder opens in the file manager). |
| **Ctrl+C or Ctrl+Insert** | **⌘C** | Copy it, to paste in the file manager. Its path also goes on the clipboard as text. |
| **Ctrl+X** | **⌘X** | Cut it, to move it with Paste in the file manager. |
| **Ctrl+Shift+C** | **⌘⇧C** | Copy its full path. |
| **F2** | **F2** | Rename it. |
| **Ctrl+E** | **⌘E** | Show it in Explorer, Finder or the file manager. |
| **Ctrl+P** | **⌘P** | Open Command Prompt (Windows) or a terminal (macOS, Linux) in its folder. |
| **Ctrl+S** | **Ctrl+S** | Windows: open PowerShell in its folder. |
| **F5** | **F5** | Refresh: read it from disk again (the top item rescans everything). |
| **Alt+Enter** | **⌥Enter** | Properties. |
| **Ctrl+H** | **⌘H** | Compute its SHA-256 hash (files only). |
| **Ctrl+M** | **⌘M** | Move it to another folder. |
| **Delete** | **fn+Delete** | Delete to the Recycle Bin or Trash (asks first). |
| **Shift+Delete** | **⇧fn+Delete** | Delete permanently (asks first). |

### Folder box and Custom filter box

While typing in them.

| Input | Action |
|---|---|
| **Enter (folder box)** | Scan the folder typed. |
| **Enter (Custom box)** | Apply the pattern now and add it to Recent. |
| **↓ / ↑** | Choose a suggested extension or example. |
| **Tab** | Complete the chosen suggestion. |

### Confirmation windows

| Input | Action |
|---|---|
| **Escape** | Cancel a Delete or Empty Folder confirmation. |

### Help window

While the pointer is over the help window.

| Windows and Linux | macOS | Action |
|---|---|---|
| **Ctrl+F** | **⌘F** | Search the help. |
| **Alt+←** | **⌥←** | Back to the previous help page. |
| **Alt+→** | **⌥→** | Forward again. |

<!-- END GENERATED -->

## Not available yet

Some shortcuts you might expect from other disk-usage tools aren't in dirmap yet: arrow keys to
move through the folder list, **Backspace** to go up a level, **Escape** to close the help window,
and Back and Forward in the sunburst. See [Feature status](../about/feature-status.md).

## Related topics

- [Mouse controls](mouse.md)
- [Moving around the treemap](../views/treemap-navigation.md)
- [Working with files and folders](../analysis/file-operations.md)
