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

When the treemap is showing (not the sunburst) and the cursor isn't in a text box.

| Windows and Linux | macOS | Action |
|---|---|---|
| **+ or =** | **+ or =** | Zoom in, around the pointer (or the middle of the map). |
| **−** | **−** | Zoom out, around the pointer (or the middle of the map). |
| **Home** | **Home** | Whole map: back to the complete scan. |
| **Alt+←** | **⌥←** | Back to the previous place. |
| **Alt+→** | **⌥→** | Forward again. |

### Selected file or folder

Act on the selected item, wherever it was selected (folder list, map or a list), unless the cursor is in a text box. The same commands are in the folder list's right-click menu.

| Windows and Linux | macOS | Action |
|---|---|---|
| **Enter** | **Enter** | Open it with its usual app (a folder opens in the file manager). |
| **Ctrl+C or Ctrl+Insert** | **⌘C** | Copy its full path. |
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
