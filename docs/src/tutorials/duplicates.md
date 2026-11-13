# Find duplicate files

**Goal:** find files stored more than once and free the space used by extra copies, safely.

## Steps

1. Scan the drive or folder you want to check, for example your **Documents**, **Pictures** or a
   downloads folder. Scanning a whole system drive works too, but see the warnings below.
2. Open the **Duplicates** tab.
3. Set **Files of at least** to the smallest size worth bothering with. **1 MB** (the default) is a
   good start; try `100 MB` first on a large drive to get a quick result.
4. Press **🔍 Find duplicates** and wait for the progress bar. Only files that share their size
   with another file are read.
5. The heading shows how many sets were found and how much space removing the extra copies would
   free. Sets are sorted by space wasted.
6. Click a path to select that copy in the file manager and on the map. Tick **Show only duplicates
   on the map** to see where the copies are.
7. To remove a copy, select it and press **Delete** (it asks first, and goes to the Recycle Bin
   or Trash), or right-click it in the file manager.

## Before you delete

- Keep the copy in the place you'd look for it; delete the ones in backup, download or "old"
  folders.
- **Don't delete duplicates inside Windows, Program Files, game folders, `node_modules` or other
  program folders.** Programs often ship identical files that each need to be in place.
- Two paths can be the same file on disk (a hard link); deleting one frees nothing.
- If you need certainty that two files are identical, compare their SHA-256 hashes: select each and
  press **Ctrl+H**.

## Related topics

- [Duplicate files](../analysis/duplicates.md)
- [Right-click menus and file operations](../analysis/file-operations.md)
