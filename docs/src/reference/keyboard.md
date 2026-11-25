# Keyboard shortcuts

Every keyboard shortcut Filescape has. This list is generated from the program itself and checked
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
| **Ctrl+1** | **⌘1** | Switch to the Files workspace. |
| **Ctrl+2** | **⌘2** | Switch to the Remote workspace (this computer beside a server). |
| **Ctrl+3** | **⌘3** | Switch to the Storage Analysis workspace. |
| **Ctrl+4** | **⌘4** | Switch to the AI Models workspace. |
| **Ctrl+F** | **⌘F** | Files: put the cursor in the Filter box. Storage Analysis: open the Custom filter. |

### Files workspace

After you've clicked in the list or the sidebar, unless the cursor is in a text box.

| Windows and Linux | macOS | Action |
|---|---|---|
| **↑ / ↓** | **↑ / ↓** | Select the item above or below. Hold Shift to extend the selection. |
| **← / →, ↑ / ↓ (Icons view)** | **← / →, ↑ / ↓ (Icons view)** | Move along a row of icons, or to the row above or below. |
| **← / → (Columns view)** | **← / → (Columns view)** | Up to the folder at the left, selecting the one you were in; into the selected folder. |
| **← / → (Gallery view)** | **← / → (Gallery view)** | The previous or next item in the strip. |
| **Home / End** | **Home / End** | Select the first or last item. |
| **Page Up / Page Down** | **Page Up / Page Down** | Move a screenful. |
| **Enter or Ctrl+↓** | **Enter or ⌘↓** | Open the selected folder, or the file with its usual app. |
| **Backspace, Alt+↑ or Ctrl+↑** | **Backspace, ⌥↑ or ⌘↑** | Up to the enclosing folder, selecting the one you were in. Above a drive: the Computer view. |
| **Alt+← / Alt+→** | **⌥← / ⌥→** | Back and Forward. |
| **Ctrl+A** | **⌘A** | Select everything shown. |
| **Escape** | **Escape** | Select nothing. |
| **Type a name** | **Type a name** | Jump to the first item whose name starts with what you type. |
| **Ctrl+L, Alt+D or F4** | **⌘L** | Type a path (works while typing elsewhere too). Start with > for commands or @ for favourites. |
| **Ctrl+T** | **⌘T** | New tab, showing the same folder. |
| **Ctrl+W** | **⌘W** | Close the tab. |
| **Ctrl+Shift+T** | **⌘⇧T** | Reopen the tab you closed last. |
| **Ctrl+Tab / Ctrl+Shift+Tab** | **⌃Tab / ⌃⇧Tab** | Next or previous tab. |
| **F5 or Ctrl+R** | **F5 or ⌘R** | Read the folder again. |
| **Ctrl+Shift+.** | **⌘⇧.** | Show or hide hidden files and folders. |
| **Ctrl+C / Ctrl+X / Ctrl+V** | **⌘C / ⌘X / ⌘V** | Copy or cut the selected items, then paste them into the folder you're in. |
| **Ctrl+Shift+C** | **⌘⇧C** | Copy the full paths of the selected items as text. |
| **F2** | **F2** | Rename the selected item. |
| **Delete** | **Delete or ⌘Backspace** | Move the selected items to the Recycle Bin or Trash (it asks first). |
| **Shift+Delete** | **⇧Delete** | Delete the selected items for good (it asks first). |
| **Ctrl+Shift+N** | **⌘⇧N** | New folder here, with its name ready to type. |
| **Ctrl+Z** | **⌘Z** | Undo the last copy, move, rename, new item or Move to Trash. Press again to undo the one before. |
| **Ctrl+Y or Ctrl+Shift+Z** | **⌘Y or ⇧⌘Z** | Redo what was just undone. |
| **F6** | **F6** | Move the keyboard to the sidebar, and back to the list. In the sidebar ↑ and ↓ move, Enter opens and Escape goes back. |
| **Alt+Enter** | **⌥Enter** | Properties: size, contents and dates. |
| **Shift+F10** | **Shift+F10** | Windows: Windows' own menu for the selected items, with what other apps add to it. |
| **Space** | **Space** | Quick Look: a large preview of the selected file. ↑ / ↓ move to the next file while it's open; Space or Escape closes it. |
| **Alt+P** | **⌥P** | Show or hide the preview pane (works while typing elsewhere too). In Storage Analysis, the preview pane beside the map. |
| **Ctrl+\** | **⌘\** | One pane or two side by side (works while typing elsewhere too). |
| **Tab** | **Tab** | Two panes: work in the other pane. |
| **Shift+F5 / Shift+F6** | **⇧F5 / ⇧F6** | Two panes: copy or move the selected items to the other pane's folder. |
| **Ctrl+U** | **⌘U** | Two panes: swap them. |
| **Ctrl+J** | **⌘J** | Show or hide the analysis dock (works while typing elsewhere too). |

### Remote workspace

Everything in the Files workspace works here too. These are different or extra.

| Input | Action |
|---|---|
| **F5 / F6** | Two panes: copy or move the selected items to the other pane's folder, as in WinSCP. Ctrl+R reads the folder again. |
| **Ctrl+Alt+T** | Open an SSH terminal in the folder (SFTP servers). |

### Sequence player

While an image sequence is playing (Enter or double-click on a sequence in Files).

| Input | Action |
|---|---|
| **Space** | Play or pause. |
| **L / J / K** | Play forwards, play backwards, pause. |
| **← / →** | Back or on one frame (with Shift, ten). |
| **Home / End** | First or last frame. |
| **F** | Fit the frame to the window. |
| **Escape** | Close the player. |

### Treemap

In Storage Analysis, when the treemap is showing (not the sunburst), the cursor isn't in a text box, and you last clicked the map (or the file manager is in its Tree layout).

| Windows and Linux | macOS | Action |
|---|---|---|
| **+ or =** | **+ or =** | Zoom in, around the pointer (or the middle of the map). |
| **−** | **−** | Zoom out, around the pointer (or the middle of the map). |
| **Home** | **Home** | Whole map: back to the complete scan. |
| **Alt+←** | **⌥←** | Back to the previous place. |
| **Alt+→** | **⌥→** | Forward again. |

### File manager

In Storage Analysis, after you've clicked in the file manager (not the map), in the List, Icons and Columns layouts, unless the cursor is in a text box.

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
| **Ctrl+Z** | **⌘Z** | Undo the last file change, as in the Files workspace. |
| **Ctrl+V** | **⌘V** | Paste: copy or move what you copied or cut into the current folder. Paths copied as text in another app are copied in too. |

### Selected file or folder

In Storage Analysis. Act on the selected item, wherever it was selected (file manager, map or a list), unless the cursor is in a text box. In the file manager they act on every selected item. The same commands are in the right-click menu.

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

### Address bar

While typing a path in the Files workspace.

| Input | Action |
|---|---|
| **↑ / ↓** | Move through the suggestions. |
| **Tab** | Complete the name: the chosen suggestion, or as much as the matching folders share. |
| **Enter** | Go there, or run the chosen command. |
| **Escape** | Stop typing and show the path again. |

### Help window

While the pointer is over the help window.

| Windows and Linux | macOS | Action |
|---|---|---|
| **Ctrl+F** | **⌘F** | Search the help. |
| **Alt+←** | **⌥←** | Back to the previous help page. |
| **Alt+→** | **⌥→** | Forward again. |

<!-- END GENERATED -->

## Not available yet

Some shortcuts you might expect from other disk-usage tools aren't in Filescape yet: arrow keys to
move through the folder list, **Backspace** to go up a level, **Escape** to close the help window,
and Back and Forward in the sunburst. See [Feature status](../about/feature-status.md).

## Related topics

- [Mouse controls](mouse.md)
- [Moving around the treemap](../views/treemap-navigation.md)
- [Working with files and folders](../analysis/file-operations.md)
