# Locate an image or render sequence

**Goal:** find a numbered sequence of frames (for example `shot010_comp.1001.exr` to
`shot010_comp.1100.exr`), see how much space it uses, and check for missing frames.

## If you know the name

1. Scan the project or render folder.
2. Press **Ctrl+F** and type the name with `#` for each digit of the frame number:

   ```filter
   shot010_comp.####.exr
   ```

   Use `{frame}` if you don't know how many digits there are, and a range such as
   `{frame:1001-1050}` for part of the sequence.
3. The count shows how many frames match and their total size. Choose **Show matches only** to
   see only those frames, with the folders they're in.

## If you don't know the name

1. Open the **Sequences** tab and press **Find sequences**.
2. Every sequence in the scan is listed once, largest first, with its frame count, frame range,
   average frame size and how many frames are missing.
3. Hover over a sequence to see its folder and exactly which frames are missing.
4. Click it: dirmap fills in the Custom filter for that sequence and selects its folder.

## Checking for missing frames

The Sequences tab reports gaps such as `Missing: 1004, 1010–1012`. Frames rendered on twos
(every other frame) are recognised as a step of 2 and not reported as missing.

To see a range of frames only, for example to check the end of a render:

```filter
shot010_comp.[1090-1100].exr
```

## Finding oversized frames

A frame that is much bigger than its neighbours often means a render problem. With the sequence
filtered, look for larger rectangles on the map, or add **Larger than** (for example `200 MB`).

## Related topics

- [Numbered sequences](../filtering/sequences.md)
- [Custom filter syntax](../filtering/syntax.md#frame-numbers-and-padding)
- [Render farms and VFX projects](../advanced/vfx.md)
