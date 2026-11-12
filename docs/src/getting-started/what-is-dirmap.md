# What is dirmap?

dirmap is a disk usage analyser for Windows, Linux and macOS, in the style of WinDirStat. It
answers one question quickly: **where has my space gone?**

It scans a drive or folder, adds up the size of every file and folder, and shows the result in
three linked views:

- A **directory tree**, like Explorer's folder list, sorted largest first, with each item's size,
  its share of the folder above it, how many files it holds and when it last changed.
- A **treemap**: every file is a rectangle whose area is proportional to its size, grouped by
  folder. Big files are big rectangles, so you can see them at once even among millions of
  small ones.
- A **sunburst**: the same data as rings, the selected folder in the middle and each ring one
  folder level deeper.

Clicking in one view selects the same item in the others.

## What you can do with it

- See which folders and files use the most space, on a whole drive or any folder.
- See which kinds of files (videos, images, AI models, archives, disk images…) use the most
  space, in the **File types** list.
- Filter the map by file type, size, or name pattern, including wildcards and numbered
  frame sequences such as `render.####.exr`.
- Find numbered image or render sequences and their missing frames.
- Find duplicate files (identical contents) and how much space the extra copies use.
- Save a snapshot of a scan and later see what is new, deleted, grew or shrank.
- Open, reveal, hash, move or delete files straight from the folder list.

## What dirmap doesn't do

dirmap reads your drives; it never changes anything unless you use a right-click command such
as **Delete** or **Move to**, and those always say exactly what they will do. It doesn't run in
the background or watch your drives for changes: a scan is a picture of the moment it was taken.
See [Feature status](../about/feature-status.md) for features that are planned but not yet
available.

## Related topics

- [Quick Start](quick-start.md)
- [Understanding the interface](interface.md)
- [How sizes are calculated](../disk-usage/sizes.md)
