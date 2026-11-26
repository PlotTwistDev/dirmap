# Supported formats

Every kind of file Filescape previews itself, in the preview pane, Quick Look, Icons view and on
map tiles. Other files still show their details, with **Open**. A file with a missing or wrong
extension that is really a picture, video, sound, PDF, font, database, MIDI file or glTF model
(judged by its first bytes) is previewed as one.

Nothing in a file is ever run: no macros, scripts, formulas or links are followed, and files are
only read, never changed. Everything is read by Filescape itself or by your system's own
decoders; nothing is sent anywhere. The libraries named here are listed with their licences in
**Help › About › Third-party licences**.

See [Quick Look and the preview pane](../views/file-manager.md#quick-look-and-the-preview-pane)
for what each preview can do, and [Previews & thumbnails](../settings/reference.md#previews--thumbnails)
for the settings.

<!-- BEGIN GENERATED: do not edit by hand; see src/help/reference.rs -->

| Format | Extensions | What the preview shows | Read by |
|---|---|---|---|
| PNG image | `.png` | The picture; channels one by one | image |
| JPEG image | `.jpg` `.jpeg` `.jpe` | The picture, including 12-bit and arithmetic-coded JPEGs; channels one by one | image (craft-jpeg for 12-bit and arithmetic-coded) |
| GIF image | `.gif` | The picture; channels one by one | image |
| WebP image | `.webp` | The picture; channels one by one | image |
| JPEG XL image | `.jxl` | The picture; channels one by one | jxl-oxide |
| JPEG 2000 image | `.jp2` `.j2k` `.j2c` `.jpc` `.jpf` `.jpg2` | The picture; channels one by one | hayro-jpeg2000 |
| TIFF image | `.tif` `.tiff` | The picture; channels one by one | image |
| BMP image | `.bmp` | The picture; channels one by one | image |
| Targa image | `.tga` | The picture; channels one by one | image |
| Windows icon | `.ico` | The picture; channels one by one | image |
| QOI image | `.qoi` | The picture; channels one by one | image |
| Netpbm image | `.pbm` `.pgm` `.ppm` `.pnm` | The picture; channels one by one | image |
| Camera RAW photo | `.cr2` `.cr3` `.crw` `.nef` `.nrw` `.arw` `.srf` `.sr2` `.dng` `.raf` `.orf` `.rw2` `.rwl` `.pef` `.srw` `.erf` `.3fr` `.fff` `.iiq` `.mos` `.mef` `.mrw` `.x3f` `.kdc` `.dcr` | The camera's own preview, and the camera, lens and exposure | Filescape (kamadak-exif) |
| Photoshop document | `.psd` `.psb` | The flattened picture, with its layers, groups, text and smart objects listed | Filescape (craft-psd for the layers) |
| HEIC or AVIF picture | `.heic` `.heif` `.hif` `.avif` | The picture | The system (macOS; Windows with the HEIF and AV1 extensions; Linux with libheif or ffmpeg) |
| OpenEXR image | `.exr` | The picture with an exposure slider; every layer of a multilayer EXR | image, exr |
| Radiance HDR image | `.hdr` | The picture with an exposure slider; every layer of a multilayer EXR | image, exr |
| SVG drawing | `.svg` `.svgz` | The drawing, with its source a click away | resvg |
| Font | `.ttf` `.otf` `.ttc` `.otc` | A specimen: the alphabet and sample text | Filescape (ttf-parser, ab_glyph) |
| Markdown | `.md` `.markdown` | Formatted text, with the source a click away | Filescape |
| Text, code and settings | `.txt` `.rst` `.log` `.json` `.jsonl` `.xml` `.yaml` `.yml` `.toml` `.ini` `.cfg` `.conf` `.rs` `.py` `.js` `.mjs` `.ts` `.tsx` `.jsx` `.c` `.h` `.cpp` `.hpp` `.cc` `.cs` `.java` `.kt` `.go` `.rb` `.php` `.swift` `.m` `.mm` `.sh` `.bash` `.zsh` `.fish` `.ps1` `.psm1` `.bat` `.cmd` `.lua` `.sql` `.html` `.htm` `.css` `.scss` `.less` `.vue` `.svelte` `.glsl` `.hlsl` `.wgsl` `.osl` `.vex` `.mel` `.nk` `.usda` `.mtl` `.gitignore` `.dockerfile` `.makefile` `.cmake` `.gradle` `.properties` `.env` `.srt` `.vtt` `.tex` `.bib` `.r` `.jl` `.dart` `.zig` `.nim` `.ex` `.exs` `.erl` `.hs` `.ml` `.clj` `.scala` `.pl` `.pm` `.tcl` `.diff` `.patch` `.lock` | The text, in syntax colours for code (about 200 languages) | Filescape (syntect, two-face) |
| Video | `.mp4` `.m4v` `.mov` `.mkv` `.avi` `.wmv` `.webm` `.mpg` `.mpeg` `.m2ts` `.mts` `.3gp` `.flv` `.mxf` | A frame, playback and a filmstrip | The system (AVFoundation, Media Foundation, or ffmpeg on Linux) |
| Sound | `.mp3` `.wav` `.flac` `.m4a` `.aac` `.ogg` `.oga` `.opus` `.wma` `.aif` `.aiff` `.alac` | Cover, waveform, tags and playback | The system, with lofty for tags |
| CSV or TSV table | `.csv` `.tsv` `.tab` | A table, with column names from the first row | Filescape (csv) |
| SQLite database | `.sqlite` `.sqlite3` `.db` `.db3` `.s3db` `.sl3` | Each table's columns and first rows, read-only | SQLite (rusqlite) |
| 3D model | `.stl` `.obj` `.ply` `.gltf` `.glb` `.3mf` | A shaded model to turn about | Filescape (gltf for glTF) |
| DXF drawing | `.dxf` | The drawing (lines, curves and blocks; text isn't drawn), with its units, layers and size | Filescape |
| CAD model (DWG, STEP, IGES) | `.dwg` `.step` `.stp` `.p21` `.iges` `.igs` | What the file says: the program that made it, units, author and parts (not drawn) | Filescape |
| Pages, Numbers and Keynote | `.pages` `.numbers` `.key` | The picture the app saved inside the file, and the version that saved it | Filescape |
| Design and 3D app files (Sketch, Affinity, CorelDRAW, SketchUp, Blender, Cinema 4D, 3ds Max, Maya, Houdini) | `.sketch` `.afdesign` `.afphoto` `.afpub` `.aftemplate` `.cdr` `.skp` `.blend` `.c4d` `.max` `.ma` `.mb` `.hip` `.hipnc` `.hiplc` | The picture the app saved inside the file, and the version that saved it | Filescape |
| Craft Apps files (PhotoCraft, VectorCraft, EffectCraft, FigCraft, DesignCraft, TextureCraft, DeckCraft, AniCraft, MaterialCraft, FilmCraft, SoundCraft) | `.pcraft` `.vectorcraft` `.drawcraft` `.vctemplate` `.ecproj` `.ectemplate` `.figcraft` `.designcraft` `.tcraft` `.deckcraft` `.anicraft` `.mcraft` `.fcproj` `.scraft` `.scraftz` | The picture the app saved inside the file, with its pages, layers, size, length and the version that saved it | Filescape (craft-doc) |
| After Effects project | `.aep` `.aepx` | The version that saved it, its compositions and the footage files it uses, marking any missing | craft-aep |
| Painting app files (Krita, OpenRaster, Figma export) | `.kra` `.krz` `.ora` `.fig` | The picture the app saved inside the file, with its size, colours and layers | Filescape |
| GIMP and Paint.NET pictures | `.xcf` `.xcf.gz` `.pdn` | The picture, drawn from its layers, with its size, colours and layers | craft-xcf, craft-pdn |
| Music project (Ableton Live, REAPER, FL Studio, Cubase, Nuendo, Logic Pro) | `.als` `.rpp` `.flp` `.cpr` `.npr` `.logicx` | The version that saved it, tempo, tracks, plugins, and the sound files it uses (marking missing ones) | Filescape |
| DICOM medical image | `.dcm` `.dicom` | The scan (uncompressed, RLE, JPEG including 12-bit, JPEG-LS or JPEG 2000) in its window, and what kind of scan it is; patient details are never shown | Filescape (craft-jpeg for 12-bit JPEG) |
| Disk image | `.iso` `.img` `.dmg` `.vhd` `.vhdx` `.avhdx` `.vmdk` `.qcow2` `.qcow` `.vdi` `.wim` `.esd` | A disc's files and folders with sizes (ISO 9660 and UDF); a virtual disk's size, type and partitions; a Windows image's editions | Filescape |
| FITS astronomy image | `.fits` `.fit` `.fts` | The first picture, stretched so faint detail shows, with the object, telescope and exposure | Filescape |
| Map data (GeoJSON, GPX, KML) | `.geojson` `.gpx` `.kml` `.kmz` | The shapes, tracks and places drawn (no map behind them), with distance, climb and times for tracks | Filescape |
| MIDI music | `.mid` `.midi` `.kar` | A piano roll, with tempo and instruments | midly |
| PDF document | `.pdf` | Every page | hayro (the system for protected PDFs) |
| Word document | `.docx` `.docm` `.dotx` | Formatted text: headings, lists, tables | Filescape |
| Spreadsheet | `.xlsx` `.xlsm` `.xlsb` `.xls` `.xltx` `.xltm` `.ods` `.ots` | Each sheet's cells, as last saved | calamine |
| OpenDocument text | `.odt` `.ott` | Formatted text: headings, lists, tables | Filescape |
| PowerPoint presentation | `.pptx` `.pptm` `.ppsx` `.potx` | Each slide's title and points | Filescape |
| OpenDocument presentation | `.odp` `.otp` | Each slide's title and points | Filescape |
| EPUB book | `.epub` | Chapters, formatted; the cover in Icons view | Filescape |
| Email message | `.eml` `.emlx` | Sender, recipients, date, the text (HTML as plain text, nothing loaded) and a list of attachments | Filescape |
| Mailbox | `.mbox` `.mbx` | A list of the messages: date, sender and subject | Filescape |
| Contact cards (vCard) | `.vcf` `.vcard` | A contact card, or a list of contacts | Filescape |
| Calendar (iCalendar) | `.ics` `.ical` `.ifb` | An event's time, place and details, or a list of events and to-dos | Filescape |
| Archives | `.zip` `.7z` `.tar` `.tar.gz` `.tgz` `.tar.xz` `.txz` `.tar.bz2` `.tbz2` | A tree of what's inside, with sizes and how much was saved | Filescape (zip, sevenz-rust2, tar) |

<!-- END GENERATED -->
