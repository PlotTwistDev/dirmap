# Find every file with one extension

**Goal:** find all files of one type, such as every `.iso` or every `.psd`, and see where they
are.

## Method 1: from the File types list

1. Scan the drive or folder.
2. In the **File types** tab, find the extension. It's sorted by total size; the **Size**, **%**
   and **Files** columns tell you how much space that type uses.
3. **Click** it: those files stay bright on the map and everything else goes dark.
4. **Right-click** it and choose **🔍 Find all .ext files** for a list of every file of that type,
   largest first. Click one to select it in the file manager and on the map.

## Method 2: with the Custom filter

1. Press **Ctrl+F** (or click **🔍 Custom**).
2. Type the extension, for example `iso` or `*.iso`. As you type, dirmap suggests extensions found
   in the scan; press **Tab** to complete one.
3. The count next to **Highlight** / **Show matches only** shows how many files match and how
   much space they use.
4. Choose **Show matches only** to see just those files on the map and in the file manager, with
   the folders that hold them.

Several types at once: `*.iso, *.img, *.dmg`.

## Related topics

- [File types and extensions](../disk-usage/file-types.md)
- [Custom filter syntax](../filtering/syntax.md)
- [Compare file types across folders](compare-types.md)
