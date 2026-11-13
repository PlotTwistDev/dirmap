# Navigating the treemap

The treemap works like a map application: zoom in towards what you're pointing at, drag to move
around, and step back when you're done. Every movement is animated so you can see where you've
gone; you can turn that off in **⚙ Settings > Treemap > Smooth zoom**.

## Zooming towards the pointer

Turn the **mouse wheel** forward over the map to zoom in. The spot under the pointer stays
under the pointer, so you can point at something tiny and keep turning the wheel until it fills
the window. Turn the wheel back to zoom out again. You can also:

- **Pinch** on a trackpad, or hold **Ctrl** (**⌘** on a Mac) while scrolling.
- Press **+** (or **=**) and **−** to zoom in and out around the pointer, or around the middle
  of the map if the pointer isn't over it.

Spinning the wheel quickly adds up into one smooth zoom. You can zoom in about ten million
times: far enough for a 1-byte file on a multi-terabyte drive to fill the window. Zooming out stops when the whole folder
fits; to go further up, see [Returning to the parent folder](#returning-to-the-parent-folder).

**⚙ Settings > Treemap > Zoom sensitivity** changes how far each wheel step zooms.

## Panning

When you're zoomed in, **drag** with the left or middle mouse button to move the map. The pointer
becomes a hand. Let go while still moving and the map coasts to a stop (when Smooth zoom is on). Dragging never selects
anything, so you can't select by accident.

## Selecting

**Click** a rectangle to select that file. Its path appears in the status bar and the file
manager shows it, in the folder that holds it. Selecting in the file manager or another tab selects it on the map too.

## Opening a folder

**Double-click** a rectangle to open the folder that holds that file. The folder grows out of its
place in the map until it fills the window, and the breadcrumbs above the map show where you are.
If the file is directly in the folder already on screen, double-clicking zooms in to frame it
instead.

You can also click a folder name in the breadcrumbs (🏠 *scan* › *folder* › …) to jump straight to
it.

## Zooming to the selection

**Zoom to selection** in the toolbar brings the selected file or folder into view, as large as
fits. If it's in a different folder from the one on screen, dirmap moves there first. Use it after
selecting something in the file manager, the Duplicates or Sequences tab, or a **Find all** list.

## Returning to the parent folder

**Right-click** the map, or press **Zoom out** in the toolbar, to go up one level:

- If you're zoomed in, it zooms out to the folder around what you're looking at.
- If the whole folder is already in view, it goes to the folder above, shrinking back into its
  place.

**⟲ Whole map** (or **Home**) returns to the complete scan.

## Back and Forward

dirmap remembers the places you've looked at (the folder and the part of it on screen) once
you've stopped there for a moment, up to 200 of them.

| Action | Mouse | Keyboard | Toolbar |
|---|---|---|---|
| Back | Mouse button 4 (back) | **Alt+←** | **◀** |
| Forward | Mouse button 5 (forward) | **Alt+→** | **▶** |

Going back after zooming, panning and opening folders retraces your steps exactly.

## Clicking during an animation

Clicks are ignored while a folder is opening or closing (a fraction of a second), so a quick
double-click can't land on the wrong tile. Wheel zooming and dragging always respond.

## Related topics

- [Mouse controls](../reference/mouse.md)
- [Keyboard shortcuts](../reference/keyboard.md)
- [Navigate a dense treemap](../tutorials/dense-treemap.md)
- [The sunburst](sunburst.md)
