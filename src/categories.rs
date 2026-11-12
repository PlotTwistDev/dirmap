//! Groups file extensions into kinds people filter by: audio, video, images, AI models, ...

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Category {
    Audio,
    Video,
    Images,
    Models,
    Archives,
    Documents,
    Code,
    Programs,
    DiskImages,
    Other,
}

impl Category {
    pub const ALL: [Category; 10] = [
        Category::Audio,
        Category::Video,
        Category::Images,
        Category::Models,
        Category::Archives,
        Category::Documents,
        Category::Code,
        Category::Programs,
        Category::DiskImages,
        Category::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Audio => "🎵 Audio",
            Category::Video => "🎬 Video",
            Category::Images => "🖼 Images",
            Category::Models => "🔮 AI models",
            Category::Archives => "📦 Archives",
            Category::Documents => "📄 Documents",
            Category::Code => "💻 Code",
            Category::Programs => "⚙ Programs",
            Category::DiskImages => "💿 Disk images",
            Category::Other => "Other",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Category::Audio => "mp3, wav, flac, aac, ogg, m4a, opus, …",
            Category::Video => "mp4, mkv, mov, avi, webm, wmv, …",
            Category::Images => "jpg, png, gif, webp, heic, raw camera files, psd, …",
            Category::Models => {
                "LLM and ML weights: gguf, ggml, safetensors, pt/pth, ckpt, onnx, h5, tflite, mlmodel, npz, \
                 and pytorch_model*.bin / model*.bin"
            }
            Category::Archives => "zip, 7z, rar, tar, gz, xz, zst, cab, …",
            Category::Documents => "pdf, Office files, txt, md, epub, …",
            Category::Code => "source code and project files",
            Category::Programs => "exe, dll, msi, so, dylib, …",
            Category::DiskImages => "iso, vhd/vhdx, vmdk, qcow2, vdi, dmg, img",
            Category::Other => "anything not in the groups above",
        }
    }

    pub fn bit(self) -> u16 {
        1 << (self as u16)
    }
}

/// Which kind of file this is, from its name.
pub fn of(name: &str) -> Category {
    let lower = name.to_ascii_lowercase();
    let ext = crate::scan::ext_of(&lower);
    // ".bin" is far too generic on its own; count it as weights only with a model-ish name.
    if ext == "bin" && (lower.starts_with("pytorch_model") || lower.starts_with("model") || lower.contains("weights")) {
        return Category::Models;
    }
    match ext {
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "oga" | "m4a" | "wma" | "opus" | "aif" | "aiff" | "alac" | "ape"
        | "mid" | "midi" | "amr" | "dsf" => Category::Audio,
        "mp4" | "mkv" | "mov" | "avi" | "wmv" | "webm" | "m4v" | "mpg" | "mpeg" | "flv" | "ts" | "m2ts" | "mts"
        | "3gp" | "vob" | "ogv" | "rmvb" | "braw" | "r3d" => Category::Video,
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tif" | "tiff" | "webp" | "heic" | "heif" | "avif" | "svg" | "ico"
        | "psd" | "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "rw2" | "raf" | "exr" | "hdr" | "jxl" => {
            Category::Images
        }
        "gguf" | "ggml" | "safetensors" | "pt" | "pth" | "ckpt" | "onnx" | "h5" | "keras" | "tflite" | "mlmodel"
        | "npz" | "npy" | "pb" | "engine" | "mar" | "pkl" | "joblib" | "lora" | "nemo" | "llamafile" => Category::Models,
        "zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "lz4" | "cab" | "lzma" | "z" | "arj" => {
            Category::Archives
        }
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp" | "rtf" | "txt" | "md"
        | "epub" | "mobi" | "csv" | "pages" | "numbers" | "key" | "one" | "pst" | "ost" | "msg" | "eml" => {
            Category::Documents
        }
        "rs" | "c" | "h" | "cpp" | "hpp" | "cc" | "cs" | "java" | "kt" | "py" | "pyc" | "js" | "mjs" | "tsx"
        | "jsx" | "go" | "rb" | "php" | "swift" | "m" | "sh" | "ps1" | "bat" | "lua" | "pl" | "scala" | "sql"
        | "json" | "yaml" | "yml" | "toml" | "xml" | "html" | "css" | "scss" | "ipynb" | "rlib" | "rmeta" | "o"
        | "obj" | "pdb" | "class" | "jar" | "whl" => Category::Code,
        "exe" | "dll" | "msi" | "sys" | "so" | "dylib" | "app" | "apk" | "appx" | "msix" | "deb" | "rpm" | "a"
        | "lib" | "com" | "scr" => Category::Programs,
        "iso" | "vhd" | "vhdx" | "vmdk" | "qcow2" | "vdi" | "dmg" | "img" | "wim" | "esd" | "avhdx" => {
            Category::DiskImages
        }
        _ => Category::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(of("song.MP3"), Category::Audio);
        assert_eq!(of("llama-3-8b.Q4_K_M.gguf"), Category::Models);
        assert_eq!(of("pytorch_model-00001-of-00002.bin"), Category::Models);
        assert_eq!(of("firmware.bin"), Category::Other);
        assert_eq!(of("Win11.iso"), Category::DiskImages);
        assert_eq!(of("README"), Category::Other);
    }
}
