//! The file formats Filescape knows: one table that says what each is and how it's previewed.
//!
//! Everything that asks "what kind of file is this?" for previews and thumbnails comes here,
//! so a format is added in one place. A file is known by its extension first; when the
//! extension is missing or unknown, its first bytes are checked against the signatures of the
//! formats that have one ([`sniff`]).
//!
//! An entry here means Filescape can actually show the format (see `docs/design/
//! filescape-previews.md`): extensions are never listed ahead of the preview that handles them.

use std::collections::HashMap;
use std::sync::OnceLock;

/// How a format is previewed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    /// Decoded here (the `image` crate).
    Picture,
    /// Linear-light pictures, shown through the sRGB curve.
    LinearPicture,
    /// Text, code and markup, read as text.
    Text,
    /// Vector drawings, drawn here (resvg), with their source a click away.
    Drawing,
    /// Fonts: a specimen drawn in the font.
    Font,
    /// Camera RAW: the JPEG preview the camera stores in the file.
    Raw,
    /// Photoshop documents: the flattened picture saved with them.
    Photoshop,
    /// Spreadsheets: their cells' saved values.
    Spreadsheet,
    /// Pictures only the system can decode (HEIC, AVIF: patent-encumbered or C decoders).
    SystemPicture,
    /// A picture and playback from the system's decoders.
    Video,
    /// Playback and a waveform from the system's decoders.
    Audio,
    /// A picture of the first page from the system.
    Pdf,
    /// Word, PowerPoint, OpenDocument and EPUB files, read and formatted here.
    Document,
    /// MIDI music: a piano roll drawn here.
    Midi,
    /// SQLite databases: their tables, read-only.
    Database,
    /// 3D models, drawn here to turn about.
    Model,
    /// Gaussian splat scenes: drawn as points here to turn about, with what's in them.
    Splat,
    /// Maps (GeoJSON, GPX, KML): their shapes drawn here, with no map tiles fetched.
    Map,
    /// CAD: DXF drawn here; DWG, STEP and IGES described from what they say about themselves.
    Cad,
    /// Apps' own formats (iWork, Sketch, Blender…): the picture they save inside, and details.
    Saved,
    /// DICOM and FITS: scientific pictures, decoded here.
    Science,
    /// Disk images: a disc's files listed, a virtual disk's size and partitions (read-only).
    DiskImage,
}

/// One format.
#[derive(Debug)]
pub struct Format {
    /// Short name, used in code to single out a format ("docx", "svg").
    pub id: &'static str,
    /// What people call it, for the Supported formats page.
    pub name: &'static str,
    pub category: Category,
    /// Lower-case extensions, without the dot.
    pub exts: &'static [&'static str],
}

impl Format {
    /// The entry with this id. Panics on a typo, which the tests catch.
    pub fn get(id: &str) -> &'static Format {
        FORMATS.iter().find(|f| f.id == id).unwrap_or_else(|| panic!("no format {id}"))
    }
}

impl Format {
    /// What its preview shows, and what reads it: for the Supported formats page.
    pub fn about(&self) -> (&'static str, &'static str) {
        match self.id {
            "markdown" => ("Formatted text, with the source a click away", "Filescape"),
            "text" => ("The text, in syntax colours for code (about 200 languages)", "Filescape (syntect, two-face)"),
            "aep" => ("The version that saved it, its compositions and the footage files it uses, marking any missing", "craft-aep"),
            "craft" => ("The picture the app saved inside the file, with its pages, layers, size, length and the version that saved it", "Filescape (craft-doc)"),
            "csv" => ("A table, with column names from the first row", "Filescape (csv)"),
            "sheet" => ("Each sheet's cells, as last saved", "calamine"),
            "docx" | "odt" => ("Formatted text: headings, lists, tables", "Filescape"),
            "pptx" | "odp" => ("Each slide's title and points", "Filescape"),
            "epub" => ("Chapters, formatted; the cover in Icons view", "Filescape"),
            "mail" => ("Sender, recipients, date, the text (HTML as plain text, nothing loaded) and a list of attachments", "Filescape"),
            "mbox" => ("A list of the messages: date, sender and subject", "Filescape"),
            "vcard" => ("A contact card, or a list of contacts", "Filescape"),
            "ical" => ("An event's time, place and details, or a list of events and to-dos", "Filescape"),
            "jxl" => ("The picture; channels one by one", "jxl-oxide"),
            "jp2" => ("The picture; channels one by one", "hayro-jpeg2000"),
            "dxf" => ("The drawing (lines, curves and blocks; text isn't drawn), with its units, layers and size", "Filescape"),
            "daw" => ("The version that saved it, tempo, tracks, plugins, and the sound files it uses (marking missing ones)", "Filescape"),
            "dicom" => ("The scan (uncompressed, RLE, JPEG, JPEG-LS or JPEG 2000) in its window, and what kind of scan it is; patient details are never shown", "Filescape"),
            "fits" => ("The first picture, stretched so faint detail shows, with the object, telescope and exposure", "Filescape"),
            "model3d" => ("A shaded model to turn about; PLY point clouds and Gaussian splat scenes are drawn as points, with their details", "Filescape (gltf for glTF)"),
            "heif" => ("The picture", "The system (macOS; Windows with the HEIF and AV1 extensions; Linux with libheif or ffmpeg)"),
            _ => match self.category {
                Category::Picture => ("The picture; channels one by one", "image"),
                Category::LinearPicture => ("The picture with an exposure slider; every layer of a multilayer EXR", "image, exr"),
                Category::Drawing => ("The drawing, with its source a click away", "resvg"),
                Category::Font => ("A specimen: the alphabet and sample text", "Filescape (ttf-parser, ab_glyph)"),
                Category::Raw => ("The camera's own preview, and the camera, lens and exposure", "Filescape (kamadak-exif)"),
                Category::Photoshop => ("The flattened picture", "Filescape"),
                Category::SystemPicture => ("The picture", "The system"),
                Category::Video => ("A frame, playback and a filmstrip", "The system (AVFoundation, Media Foundation, or ffmpeg on Linux)"),
                Category::Audio => ("Cover, waveform, tags and playback", "The system, with lofty for tags"),
                Category::Pdf => ("Every page", "hayro (the system for protected PDFs)"),
                Category::Midi => ("A piano roll, with tempo and instruments", "midly"),
                Category::Database => ("Each table's columns and first rows, read-only", "SQLite (rusqlite)"),
                Category::Model => ("A shaded model to turn about", "Filescape (gltf for glTF)"),
                Category::Splat => ("The scene drawn as points to turn about, with its splat count, colour detail, size and checks for stray or invisible splats", "Filescape"),
                Category::Cad => ("What the file says: the program that made it, units, author and parts (not drawn)", "Filescape"),
                Category::Saved => ("The picture the app saved inside the file, and the version that saved it", "Filescape"),
                Category::DiskImage => ("A disc's files and folders with sizes (ISO 9660 and UDF); a virtual disk's size, type and partitions; a Windows image's editions", "Filescape"),
                Category::Science => ("The picture, with the scan's or telescope's details", "Filescape"),
                Category::Map => ("The shapes, tracks and places drawn (no map behind them), with distance, climb and times for tracks", "Filescape"),
                Category::Text | Category::Spreadsheet | Category::Document => ("A preview", "Filescape"),
            },
        }
    }
}

/// The Supported formats page's table (see `docs/src/reference/formats.md`).
pub fn markdown() -> String {
    let mut out = String::from("| Format | Extensions | What the preview shows | Read by |\n|---|---|---|---|\n");
    let exts = |e: &[&str]| e.iter().map(|x| format!("`.{x}`")).collect::<Vec<_>>().join(" ");
    for f in FORMATS {
        let (see, by) = f.about();
        out.push_str(&format!("| {} | {} | {see} | {by} |\n", f.name, exts(f.exts)));
    }
    out.push_str(&format!(
        "| Archives | {} | A tree of what's inside, with sizes and how much was saved | Filescape (zip, sevenz-rust2, tar) |\n",
        exts(&["zip", "7z", "tar", "tar.gz", "tgz", "tar.xz", "txz", "tar.bz2", "tbz2"])
    ));
    out
}

/// Files with no extension that are text by name.
const TEXT_NAMES: &[&str] = &["readme", "license", "makefile", "dockerfile", "changelog", "authors"];

pub static FORMATS: &[Format] = &[
    Format { id: "png", name: "PNG image", category: Category::Picture, exts: &["png"] },
    Format { id: "jpeg", name: "JPEG image", category: Category::Picture, exts: &["jpg", "jpeg", "jpe"] },
    Format { id: "gif", name: "GIF image", category: Category::Picture, exts: &["gif"] },
    Format { id: "webp", name: "WebP image", category: Category::Picture, exts: &["webp"] },
    Format { id: "jxl", name: "JPEG XL image", category: Category::Picture, exts: &["jxl"] },
    Format { id: "jp2", name: "JPEG 2000 image", category: Category::Picture, exts: &["jp2", "j2k", "j2c", "jpc", "jpf", "jpg2"] },
    Format { id: "tiff", name: "TIFF image", category: Category::Picture, exts: &["tif", "tiff"] },
    Format { id: "bmp", name: "BMP image", category: Category::Picture, exts: &["bmp"] },
    Format { id: "tga", name: "Targa image", category: Category::Picture, exts: &["tga"] },
    Format { id: "ico", name: "Windows icon", category: Category::Picture, exts: &["ico"] },
    Format { id: "qoi", name: "QOI image", category: Category::Picture, exts: &["qoi"] },
    Format { id: "pnm", name: "Netpbm image", category: Category::Picture, exts: &["pbm", "pgm", "ppm", "pnm"] },
    Format {
        id: "raw",
        name: "Camera RAW photo",
        category: Category::Raw,
        exts: &[
            "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "dng", "raf", "orf", "rw2", "rwl", "pef", "srw", "erf", "3fr", "fff", "iiq", "mos", "mef", "mrw",
            "x3f", "kdc", "dcr",
        ],
    },
    Format { id: "psd", name: "Photoshop document", category: Category::Photoshop, exts: &["psd", "psb"] },
    Format { id: "heif", name: "HEIC or AVIF picture", category: Category::SystemPicture, exts: &["heic", "heif", "hif", "avif"] },
    Format { id: "exr", name: "OpenEXR image", category: Category::LinearPicture, exts: &["exr"] },
    Format { id: "hdr", name: "Radiance HDR image", category: Category::LinearPicture, exts: &["hdr"] },
    Format { id: "svg", name: "SVG drawing", category: Category::Drawing, exts: &["svg", "svgz"] },
    Format { id: "font", name: "Font", category: Category::Font, exts: &["ttf", "otf", "ttc", "otc"] },
    Format { id: "markdown", name: "Markdown", category: Category::Text, exts: &["md", "markdown"] },
    Format {
        id: "text",
        name: "Text, code and settings",
        category: Category::Text,
        exts: &[
            "txt", "rst", "log", "json", "jsonl", "xml", "yaml", "yml", "toml", "ini", "cfg", "conf", "rs", "py", "js", "mjs", "ts",
            "tsx", "jsx", "c", "h", "cpp", "hpp", "cc", "cs", "java", "kt", "go", "rb", "php", "swift", "m", "mm", "sh", "bash", "zsh", "fish", "ps1",
            "psm1", "bat", "cmd", "lua", "sql", "html", "htm", "css", "scss", "less", "vue", "svelte", "glsl", "hlsl", "wgsl", "osl", "vex", "mel",
            "nk", "usda", "mtl", "gitignore", "dockerfile", "makefile", "cmake", "gradle", "properties", "env", "srt", "vtt", "tex", "bib",
            "r", "jl", "dart", "zig", "nim", "ex", "exs", "erl", "hs", "ml", "clj", "scala", "pl", "pm", "tcl", "diff", "patch", "lock",
        ],
    },
    Format {
        id: "video",
        name: "Video",
        category: Category::Video,
        exts: &["mp4", "m4v", "mov", "mkv", "avi", "wmv", "webm", "mpg", "mpeg", "m2ts", "mts", "3gp", "flv", "mxf"],
    },
    Format {
        id: "audio",
        name: "Sound",
        category: Category::Audio,
        exts: &["mp3", "wav", "flac", "m4a", "aac", "ogg", "oga", "opus", "wma", "aif", "aiff", "alac"],
    },
    Format { id: "csv", name: "CSV or TSV table", category: Category::Spreadsheet, exts: &["csv", "tsv", "tab"] },
    Format { id: "sqlite", name: "SQLite database", category: Category::Database, exts: &["sqlite", "sqlite3", "db", "db3", "s3db", "sl3"] },
    Format { id: "model3d", name: "3D model", category: Category::Model, exts: &["stl", "obj", "ply", "gltf", "glb", "3mf"] },
    Format { id: "spz", name: "Gaussian splat scene (SPZ)", category: Category::Splat, exts: &["spz"] },
    Format { id: "dxf", name: "DXF drawing", category: Category::Cad, exts: &["dxf"] },
    Format { id: "cad", name: "CAD model (DWG, STEP, IGES)", category: Category::Cad, exts: &["dwg", "step", "stp", "p21", "iges", "igs"] },
    Format { id: "iwork", name: "Pages, Numbers and Keynote", category: Category::Saved, exts: &["pages", "numbers", "key"] },
    Format {
        id: "creative",
        name: "Design and 3D app files (Sketch, Affinity, CorelDRAW, SketchUp, Blender, Cinema 4D, 3ds Max, Maya, Houdini)",
        category: Category::Saved,
        exts: &["sketch", "afdesign", "afphoto", "afpub", "aftemplate", "cdr", "skp", "blend", "c4d", "max", "ma", "mb", "hip", "hipnc", "hiplc"],
    },
    Format {
        id: "craft",
        name: "Craft Apps files (PhotoCraft, VectorCraft, EffectCraft, FigCraft, DesignCraft, TextureCraft, DeckCraft, AniCraft, MaterialCraft, FilmCraft, SoundCraft)",
        category: Category::Saved,
        exts: &["pcraft", "vectorcraft", "drawcraft", "vctemplate", "ecproj", "ectemplate", "figcraft", "designcraft", "tcraft", "deckcraft", "anicraft", "mcraft", "fcproj", "scraft", "scraftz"],
    },
    Format { id: "aep", name: "After Effects project", category: Category::Saved, exts: &["aep", "aepx"] },
    Format { id: "daw", name: "Music project (Ableton Live, REAPER, FL Studio, Cubase, Nuendo, Logic Pro)", category: Category::Document, exts: &["als", "rpp", "flp", "cpr", "npr", "logicx"] },
    Format { id: "dicom", name: "DICOM medical image", category: Category::Science, exts: &["dcm", "dicom"] },
    Format { id: "diskimage", name: "Disk image", category: Category::DiskImage, exts: &["iso", "img", "dmg", "vhd", "vhdx", "avhdx", "vmdk", "qcow2", "qcow", "vdi", "wim", "esd"] },
    Format { id: "fits", name: "FITS astronomy image", category: Category::Science, exts: &["fits", "fit", "fts"] },
    Format { id: "map", name: "Map data (GeoJSON, GPX, KML)", category: Category::Map, exts: &["geojson", "gpx", "kml", "kmz"] },
    Format { id: "midi", name: "MIDI music", category: Category::Midi, exts: &["mid", "midi", "kar"] },
    Format { id: "pdf", name: "PDF document", category: Category::Pdf, exts: &["pdf"] },
    Format { id: "docx", name: "Word document", category: Category::Document, exts: &["docx", "docm", "dotx"] },
    Format { id: "sheet", name: "Spreadsheet", category: Category::Spreadsheet, exts: &["xlsx", "xlsm", "xlsb", "xls", "xltx", "xltm", "ods", "ots"] },
    Format { id: "odt", name: "OpenDocument text", category: Category::Document, exts: &["odt", "ott"] },
    Format { id: "pptx", name: "PowerPoint presentation", category: Category::Document, exts: &["pptx", "pptm", "ppsx", "potx"] },
    Format { id: "odp", name: "OpenDocument presentation", category: Category::Document, exts: &["odp", "otp"] },
    Format { id: "epub", name: "EPUB book", category: Category::Document, exts: &["epub"] },
    Format { id: "mail", name: "Email message", category: Category::Document, exts: &["eml", "emlx"] },
    Format { id: "mbox", name: "Mailbox", category: Category::Document, exts: &["mbox", "mbx"] },
    Format { id: "vcard", name: "Contact cards (vCard)", category: Category::Document, exts: &["vcf", "vcard"] },
    Format { id: "ical", name: "Calendar (iCalendar)", category: Category::Document, exts: &["ics", "ical", "ifb"] },
];

fn by_ext() -> &'static HashMap<&'static str, &'static Format> {
    static MAP: OnceLock<HashMap<&'static str, &'static Format>> = OnceLock::new();
    MAP.get_or_init(|| FORMATS.iter().flat_map(|f| f.exts.iter().map(move |e| (*e, f))).collect())
}

/// The format of a file, from its name alone.
pub fn by_name(name: &str) -> Option<&'static Format> {
    let ext = crate::scan::ext_of(name).to_ascii_lowercase();
    if ext.is_empty() {
        let lower = name.to_ascii_lowercase();
        return TEXT_NAMES.contains(&lower.as_str()).then(|| Format::get("text"));
    }
    by_ext().get(ext.as_str()).copied()
}

/// The format these first bytes of a file belong to, for the formats with a signature. 32
/// bytes are enough.
pub fn sniff(head: &[u8]) -> Option<&'static Format> {
    let at = |i: usize, sig: &[u8]| head.get(i..i + sig.len()) == Some(sig);
    let id = if at(0, b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if at(0, b"\xff\xd8\xff") {
        "jpeg"
    } else if at(0, b"GIF87a") || at(0, b"GIF89a") {
        "gif"
    } else if at(0, b"RIFF") && at(8, b"WEBP") {
        "webp"
    } else if at(0, b"\xff\x0a") || at(0, b"\0\0\0\x0cJXL \r\n\x87\n") {
        "jxl"
    } else if at(0, b"\0\0\0\x0cjP  \r\n\x87\n") || at(0, b"\xff\x4f\xff\x51") {
        "jp2"
    } else if at(0, b"II*\0") || at(0, b"MM\0*") {
        "tiff"
    } else if at(0, b"qoif") {
        "qoi"
    } else if at(0, b"\x76\x2f\x31\x01") {
        "exr"
    } else if at(0, b"#?RADIANCE") || at(0, b"#?RGBE") {
        "hdr"
    } else if at(0, b"\0\x01\0\0") || at(0, b"OTTO") || at(0, b"ttcf") || at(0, b"true") {
        "font"
    } else if at(0, b"8BPS") {
        "psd"
    } else if at(0, crate::tables::SQLITE_MAGIC) {
        "sqlite"
    } else if at(0, b"NGSP") {
        "spz"
    } else if at(0, b"glTF") {
        "model3d"
    } else if at(0, b"MThd") {
        "midi"
    } else if at(0, b"BEGIN:VCARD") {
        "vcard"
    } else if at(0, b"BEGIN:VCALENDAR") {
        "ical"
    } else if at(0, b"Return-Path: ") || at(0, b"Received: ") || at(0, b"MIME-Version: ") {
        "mail"
    } else if at(0, b"FLhd") {
        "daw"
    } else if at(0, b"<REAPER_PROJECT") {
        "daw"
    } else if at(128, b"DICM") {
        "dicom"
    } else if at(0, b"SIMPLE  =") {
        "fits"
    } else if at(0, b"vhdxfile") || at(0, b"QFI\xfb") || at(0, b"KDMV") || at(0, b"MSWIM\0\0\0") || at(0, b"conectix") {
        "diskimage"
    } else if at(0, b"BLENDER") {
        "creative"
    } else if at(0, b"ISO-10303-21;") {
        "cad"
    } else if at(0, b"%PDF-") {
        "pdf"
    } else if at(4, b"ftyp") {
        // ISO media: video, unless the brand says it's a picture (HEIC, AVIF) or sound.
        match head.get(8..12) {
            Some(b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"mif1" | b"msf1" | b"avif" | b"avis") => "heif",
            Some(b"M4A " | b"M4B ") => "audio",
            _ => "video",
        }
    } else if at(0, b"\x1a\x45\xdf\xa3") {
        "video" // Matroska and WebM
    } else if at(0, b"RIFF") && at(8, b"AVI ") {
        "video"
    } else if at(0, b"RIFF") && at(8, b"WAVE") {
        "audio"
    } else if at(0, b"FORM") && (at(8, b"AIFF") || at(8, b"AIFC")) {
        "audio"
    } else if at(0, b"fLaC") || at(0, b"OggS") || at(0, b"ID3") {
        "audio"
    } else {
        return None;
    };
    Some(Format::get(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_extension_belongs_to_one_format() {
        let mut seen = std::collections::HashSet::new();
        for f in FORMATS {
            for e in f.exts {
                assert_eq!(*e, e.to_ascii_lowercase(), "{e} isn't lower case");
                assert!(seen.insert(*e), "{e} is listed twice");
            }
        }
        for id in ["png", "jpeg", "gif", "webp", "tiff", "qoi", "exr", "hdr", "pdf", "video", "audio", "text", "svg", "docx", "odt"] {
            Format::get(id);
        }
    }

    #[test]
    fn names_and_signatures() {
        assert_eq!(by_name("Photo.JPG").map(|f| f.id), Some("jpeg"));
        assert_eq!(by_name("README").map(|f| f.id), Some("text"));
        assert_eq!(by_name("notes.md").map(|f| f.category), Some(Category::Text));
        assert!(by_name("thing.unknownext").is_none());
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR").map(|f| f.id), Some("png"));
        assert_eq!(sniff(b"%PDF-1.7\n").map(|f| f.id), Some("pdf"));
        assert_eq!(sniff(b"\0\0\0\x18ftypqt  \0\0\0\0").map(|f| f.id), Some("video"));
        assert_eq!(sniff(b"\0\0\0\x18ftypM4A \0\0\0\0").map(|f| f.id), Some("audio"));
        assert_eq!(sniff(b"\0\0\0\x18ftypheic\0\0\0\0").map(|f| f.id), Some("heif"));
        assert!(sniff(b"hello, world").is_none());
        assert_eq!(sniff(b"OTTO\0\x0b\0\x80").map(|f| f.id), Some("font"));
        assert_eq!(by_name("Inter.TTF").map(|f| f.category), Some(Category::Font));
        assert_eq!(by_name("tune.MID").map(|f| f.category), Some(Category::Midi));
        assert_eq!(sniff(b"MThd\0\0\0\x06").map(|f| f.id), Some("midi"));
        assert_eq!(sniff(b"BEGIN:VCALENDAR\r\n").map(|f| f.id), Some("ical"));
        assert_eq!(by_name("Inbox.mbox").map(|f| f.id), Some("mbox"));
    }
}
