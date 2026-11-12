# Render farms and VFX projects

Visual effects, animation and games projects produce huge numbers of large, numbered files:
render frames, simulation caches, textures and plates. dirmap has several features aimed at them.

## A typical investigation

1. **Scan the project or render share.** For network storage, raise **Scanner threads** first;
   see [External and network drives](network-drives.md).
2. **Find the heavy shots.** The folder list shows each shot's or sequence's folder total;
   sorting by **Files** finds folders with the most frames.
3. **List sequences.** In the **Sequences** tab press **Find sequences** to see every image
   sequence once, with frame count, range, average frame size and missing frames.
4. **Filter by pass, version or frame range** with the Custom filter, for example every shot's
   beauty pass for frames 1001 to 1100:

   ```filter
   *_beauty.[1001-1100].exr
   ```

5. **Compare over time.** Save a snapshot after each delivery, then compare to see which shots
   grew. With **Color by: Change**, newly written renders show red.

## Useful patterns

All of these are checked against dirmap's matcher. See [Custom filter syntax](../filtering/syntax.md).

| Pattern | Finds |
|---|---|
| `*.exr, !*preview*, !*proxy*` | EXR frames, without previews and proxies |
| `*_beauty.####.exr` | The beauty pass of every shot, four-digit frames |
| `*_v###.*` | Anything with a three-digit version number |
| `*.{frame:1001-1010}.exr` | The first ten frames of every sequence, however padded |
| `*.bgeo.sc, *.vdb, *.abc` | Simulation and geometry caches |
| `*.tx, *.tex` | Pre-converted render textures |

## Checking renders

- **Missing frames:** the Sequences tab lists gaps for each sequence. Frames rendered on twos are
  recognised as such and not reported as missing.
- **Oversized frames:** filter to a sequence and add **Larger than** to find frames much bigger
  than the rest, often a sign of a bad render setting (wrong compression, extra AOVs).
- **Empty or tiny frames:** **Smaller than** `1 KB` with `*.exr` finds frames that failed to write.
- **Stale versions:** **Color by: Age** shows old versions untouched for months.

## Duplicates on project storage

Projects often copy plates and textures between shots. The **Duplicates** tab finds identical
files by contents. With millions of frames, start with a high minimum size, such as 50 MB, so the
search reads less. See [Duplicate files](../analysis/duplicates.md).

## Limitations

- Sequences are recognised only when the frame number is the last number before the extension
  (`name.1001.exr`, `name_1001.exr`). The Custom filter can match other layouts.
- Frame numbers can have up to 18 digits.
- dirmap shows file sizes as the storage reports them; compression or deduplication on the server
  isn't reflected.

## Related topics

- [Numbered sequences](../filtering/sequences.md)
- [Locate an image or render sequence](../tutorials/render-sequence.md)
- [Filter examples](../filtering/examples.md)
