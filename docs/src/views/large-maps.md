# Working with very large maps

dirmap is designed for drives with millions of files. A scan of 3.5 million files draws and zooms
as smoothly as a scan of a thousand, because:

- Only the tiles you can see are worked out, down to about a pixel, and only when the view has
  moved far enough to need new detail. Panning and small zooms just move what's already drawn.
- That work happens on background threads, so the window never stops responding. While new
  detail is on its way, the map keeps showing what it has at lower detail rather than going
  blank.
- The folder list, the File types list and the other lists only draw the rows on screen.

## Tips

- **Start from the folder list.** It's sorted largest first, so opening the top folder at each
  level leads straight to where the space is. Then use **Zoom to selection** to see it on the map.
- **Use breadcrumbs and Back.** Double-click into a folder to work inside it; the breadcrumbs
  show the way back, and **Alt+←** retraces your steps.
- **Filter instead of searching by eye.** **Show:** categories, size limits and the Custom
  filter make the files you care about stand out. **Show matches only** removes everything else
  from the map, which is much easier to read when the matches are a small fraction of the scan.
- **Turn off file names** on tiles if the map feels busy (**File names on tiles** in the filter bar).
- **Use a flat look** (Settings > Treemap > Presets > Flat) for a crisper view of very small tiles.

## If zooming feels slow

See [Zooming or rendering is slow](../troubleshooting/index.md#zooming-or-rendering-is-slow).

## Related topics

- [Navigate a dense treemap](../tutorials/dense-treemap.md)
- [Scanning millions of files](../advanced/millions-of-files.md)
- [How dirmap works](../advanced/architecture.md)
