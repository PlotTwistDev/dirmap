# File categories

The **Show:** buttons in the filter bar limit the map to kinds of files. Click one to turn it on;
click more to add other kinds (a file of **any** chosen kind is shown); click **All** to turn them
all off. Hover over a button to see a summary of what it covers.

A file's category comes from its extension, ignoring case. The complete lists are:

| Category | Extensions |
|---|---|
| 🎵 **Audio** | `mp3` `wav` `flac` `aac` `ogg` `oga` `m4a` `wma` `opus` `aif` `aiff` `alac` `ape` `mid` `midi` `amr` `dsf` |
| 🎬 **Video** | `mp4` `mkv` `mov` `avi` `wmv` `webm` `m4v` `mpg` `mpeg` `flv` `ts` `m2ts` `mts` `3gp` `vob` `ogv` `rmvb` `braw` `r3d` |
| 🖼 **Images** | `jpg` `jpeg` `png` `gif` `bmp` `tif` `tiff` `webp` `heic` `heif` `avif` `svg` `ico` `psd` `raw` `cr2` `cr3` `nef` `arw` `dng` `orf` `rw2` `raf` `exr` `hdr` `jxl` |
| 🔮 **AI models** | `gguf` `ggml` `safetensors` `pt` `pth` `ckpt` `onnx` `h5` `keras` `tflite` `mlmodel` `npz` `npy` `pb` `engine` `mar` `pkl` `joblib` `lora` `nemo` `llamafile`, and `.bin` files whose names start with `pytorch_model` or `model`, or contain `weights` |
| 📦 **Archives** | `zip` `7z` `rar` `tar` `gz` `tgz` `bz2` `xz` `zst` `lz4` `cab` `lzma` `z` `arj` |
| 📄 **Documents** | `pdf` `doc` `docx` `xls` `xlsx` `ppt` `pptx` `odt` `ods` `odp` `rtf` `txt` `md` `epub` `mobi` `csv` `pages` `numbers` `key` `one` `pst` `ost` `msg` `eml` |
| 💻 **Code** | `rs` `c` `h` `cpp` `hpp` `cc` `cs` `java` `kt` `py` `pyc` `js` `mjs` `tsx` `jsx` `go` `rb` `php` `swift` `m` `sh` `ps1` `bat` `lua` `pl` `scala` `sql` `json` `yaml` `yml` `toml` `xml` `html` `css` `scss` `ipynb` `rlib` `rmeta` `o` `obj` `pdb` `class` `jar` `whl` |
| ⚙ **Programs** | `exe` `dll` `msi` `sys` `so` `dylib` `app` `apk` `appx` `msix` `deb` `rpm` `a` `lib` `com` `scr` |
| 💿 **Disk images** | `iso` `vhd` `vhdx` `vmdk` `qcow2` `vdi` `dmg` `img` `wim` `esd` `avhdx` |
| **Other** | Everything else, including files with no extension. |

Some extensions are ambiguous. `.ts` is counted as video (MPEG transport stream), not TypeScript;
`.key` as a Keynote document; `.obj` as a compiled object file rather than a 3D model; and a
plain `.bin` is **Other** unless its name looks like model weights, because so many unrelated
files end in `.bin`.

To filter by extensions that aren't grouped the way you want, use the
[Custom filter](syntax.md), for example `*.ts, *.tsx` or `*.obj, *.fbx, *.abc`.

## Related topics

- [Filtering overview](overview.md)
- [File types and extensions](../disk-usage/file-types.md)
- [Custom filter syntax](syntax.md)
