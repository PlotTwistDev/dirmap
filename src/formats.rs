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
    /// A picture and playback from the system's decoders.
    Video,
    /// Playback and a waveform from the system's decoders.
    Audio,
    /// A picture of the first page from the system.
    Pdf,
    /// A word-processing document whose words are read here.
    Document,
}

/// One format.
#[derive(Debug)]
pub struct Format {
    /// Short name, used in code to single out a format ("docx", "svg").
    pub id: &'static str,
    /// What people call it. For the Supported Formats list (phase 1 of the roadmap).
    #[allow(dead_code)]
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

/// Files with no extension that are text by name.
const TEXT_NAMES: &[&str] = &["readme", "license", "makefile", "dockerfile", "changelog", "authors"];

pub static FORMATS: &[Format] = &[
    Format { id: "png", name: "PNG image", category: Category::Picture, exts: &["png"] },
    Format { id: "jpeg", name: "JPEG image", category: Category::Picture, exts: &["jpg", "jpeg", "jpe"] },
    Format { id: "gif", name: "GIF image", category: Category::Picture, exts: &["gif"] },
    Format { id: "webp", name: "WebP image", category: Category::Picture, exts: &["webp"] },
    Format { id: "tiff", name: "TIFF image", category: Category::Picture, exts: &["tif", "tiff"] },
    Format { id: "bmp", name: "BMP image", category: Category::Picture, exts: &["bmp"] },
    Format { id: "tga", name: "Targa image", category: Category::Picture, exts: &["tga"] },
    Format { id: "ico", name: "Windows icon", category: Category::Picture, exts: &["ico"] },
    Format { id: "qoi", name: "QOI image", category: Category::Picture, exts: &["qoi"] },
    Format { id: "pnm", name: "Netpbm image", category: Category::Picture, exts: &["pbm", "pgm", "ppm", "pnm"] },
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
            "txt", "rst", "log", "csv", "tsv", "json", "jsonl", "xml", "yaml", "yml", "toml", "ini", "cfg", "conf", "rs", "py", "js", "mjs", "ts",
            "tsx", "jsx", "c", "h", "cpp", "hpp", "cc", "cs", "java", "kt", "go", "rb", "php", "swift", "m", "mm", "sh", "bash", "zsh", "fish", "ps1",
            "psm1", "bat", "cmd", "lua", "sql", "html", "htm", "css", "scss", "less", "vue", "svelte", "glsl", "hlsl", "wgsl", "osl", "vex", "mel",
            "nk", "usda", "obj", "mtl", "gitignore", "dockerfile", "makefile", "cmake", "gradle", "properties", "env", "srt", "vtt", "tex", "bib",
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
    Format { id: "pdf", name: "PDF document", category: Category::Pdf, exts: &["pdf"] },
    Format { id: "docx", name: "Word document", category: Category::Document, exts: &["docx", "docm", "dotx"] },
    Format { id: "odt", name: "OpenDocument text", category: Category::Document, exts: &["odt", "ott"] },
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
    } else if at(0, b"%PDF-") {
        "pdf"
    } else if at(4, b"ftyp") {
        // ISO media: video, unless the brand says it's a picture (HEIC, AVIF) or sound.
        match head.get(8..12) {
            Some(b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"mif1" | b"msf1" | b"avif" | b"avis") => return None,
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
        assert!(sniff(b"\0\0\0\x18ftypheic\0\0\0\0").is_none(), "HEIC isn't decoded yet");
        assert!(sniff(b"hello, world").is_none());
        assert_eq!(sniff(b"OTTO\0\x0b\0\x80").map(|f| f.id), Some("font"));
        assert_eq!(by_name("Inter.TTF").map(|f| f.category), Some(Category::Font));
    }
}
