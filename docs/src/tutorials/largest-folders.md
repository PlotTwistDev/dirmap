# Find the folders using the most space

**Goal:** find which folders hold most of the data, so you know where to look.

## Steps

1. Scan the drive or folder.
2. In the folder list, the top item is everything you scanned. Its contents are listed **largest
   first**, with **% of parent** showing each folder's share.
3. Open the first (largest) folder by clicking its arrow or double-clicking it. Again, the largest
   folder inside is first.
4. Keep opening the largest folder until the sizes split up evenly or you reach files. That's
   where the space is.
5. Click a folder, then press **Zoom to selection** to see it on the treemap. On the map, each
   folder is a region outlined by its cushion shading; hover over it to see the folder outlined.

## Tips

- Sort by **Files** (click the heading) to find folders with huge numbers of small files instead,
  such as caches and `node_modules` folders.
- Sort by **Last change** to see which folders have changed recently.
- The **Sunburst** view (**View: Sunburst**) shows how space splits between folders at several
  levels at once: big segments in the inner rings are big folders.
- **Color by: Age** and look for large blue areas: big folders nobody has changed in years.

## Related topics

- [The directory tree](../views/directory-tree.md)
- [The sunburst](../views/sunburst.md)
- [How sizes are calculated](../disk-usage/sizes.md)
