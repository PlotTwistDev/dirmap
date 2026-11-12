# Filter examples

Ready-made patterns for the **🔍 Custom** filter (**Ctrl+F**). Copy one with the **Copy** button,
or press **Use in Custom filter** to apply it straight away. See [Custom filter syntax](syntax.md)
for how they work.

## Everyday clean-up

Large media files:

```filter
*.mp4, *.mkv, *.mov, *.avi
```

Installers and downloads you may no longer need:

```filter
*.iso, *.msi, *.dmg, *.exe, *.zip
```

Temporary and backup files:

```filter
*.tmp, *.temp, *.bak, *.old, ~$*
```

Log and crash dump files:

```filter
*.log, *.dmp, *.mdmp
```

Virtual machine and WSL disks (see [Virtual disk files](../disk-usage/virtual-disks.md)):

```filter
*.vhdx, *.vhd, *.vdi, *.vmdk, *.qcow2
```

Copies that file managers make, such as `Report (1).docx` or `photo - Copy.jpg`:

```filter
*(1).*, *(2).*, * - Copy.*, * - Copy (*).*
```

## Developers

Build output and dependency caches by type:

```filter
*.obj, *.pdb, *.o, *.rlib, *.rmeta, *.pch, *.ipch
```

AI model weights:

```filter
*.gguf, *.safetensors, *.ckpt, *.pt, *.pth, *.onnx
```

## Photography

Camera raw files, leaving out sidecar files:

```filter
*.cr2, *.cr3, *.nef, *.arw, *.dng, *.raf, *.orf, *.rw2
```

Exported JPEGs whose names contain `edit`:

```filter
*edit*.jpg
```

## VFX, animation and rendering

All EXR frames:

```filter
*.exr
```

EXR frames, but not previews or proxies:

```filter
*.exr, !*preview*, !*proxy*
```

One shot's beauty pass, every four-digit frame:

```filter
sh010_*beauty*.####.exr
```

Frames 1001 to 1100 of a render, however they're padded:

```filter
render.{frame:1001-1100}.exr
```

The first 24 frames of every sequence that uses four-digit frame numbers:

```filter
*.[1001-1024].exr
```

Simulation caches:

```filter
*.bgeo.sc, *.vdb, *.abc, *.usd*
```

Texture files of every common kind:

```filter
*.tx, *.tex, *.tif, *.tiff, *.exr, *.png
```

Files with a version number, such as `comp_v012.nk`:

```filter
*_v###.*
```

## Combining with other filters

The Custom pattern combines with the **Show:** categories and the size limits. For example:

- **Custom** `*.exr` with **Larger than** `100 MB`: unusually large frames, often a sign of a
  render setting gone wrong.
- **Show: 🎬 Video** with **Custom** `!*proxy*`: every video except proxies.
- **Custom** `*.log` with **Larger than** `1 GB`: runaway log files.

## Related topics

- [Custom filter syntax](syntax.md)
- [Locate an image or render sequence](../tutorials/render-sequence.md)
- [Presets and recent filters](presets.md)
