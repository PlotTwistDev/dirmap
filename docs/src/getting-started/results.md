# Interpreting scan results

When a scan finishes, the status bar sums it up, for example:

`D:\: 812.4 GB in 3,512,804 files, scanned in 94.2s. 12 items couldn't be read.`

That is the total size of every file found, how many files there are, how long the scan took
and, if any, how many folders or files couldn't be read (and so are missing from the totals).

## Reading the folder list

| Column | Meaning |
|---|---|
| **Name** | The file or folder. Folders have an arrow to open them. |
| **Size** | The total size of the file, or of everything inside the folder at any depth. |
| **% of parent** | This item's size as a share of the folder that contains it, with a bar. The top item is always 100%. |
| **Files** | For folders: how many files are inside, at any depth. |
| **Last change** | For files: when the file was last modified. For folders: the newest modification of anything inside. |

Everything is sorted largest first, so the first few rows of each folder are where the space is.

## Reading the treemap

Each rectangle is one file. Its **area** is proportional to its size: a rectangle twice as big is
a file twice as big. Files in the same folder are grouped together, and each folder's group is
shaded like a cushion so you can see where one folder ends and the next begins. Colours show the
file type (or age, or change), and the **File types** list says which colour is which.

A large single block of colour is one big file. A finely speckled area is a folder with many
small files. See [The treemap](../views/treemap.md).

## Reading the File types list

Each row is one extension (`.mp4`, `.dll`, or **(none)** for files without one) with its colour,
total size, share of the scan (**%**) and number of files. It's sorted by size, so the top rows are
the kinds of files using the most space. Click a row to light up just those files on the map.

## Why the total may differ from Windows

dirmap adds up the sizes of files it can read. Windows' "used space" also includes the
Recycle Bin, restore points, system files dirmap can't read, file-system overhead, and the
difference between a file's size and the space it occupies on disk. See
[Why is free space lower than expected?](../tutorials/missing-space.md) and
[How sizes are calculated](../disk-usage/sizes.md).

## Related topics

- [How sizes are calculated](../disk-usage/sizes.md)
- [The directory tree](../views/directory-tree.md)
- [The treemap](../views/treemap.md)
