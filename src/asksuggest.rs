//! Ask AI's suggested questions for a file or folder, by what it is: a licence gets "Can I use
//! this commercially?", a log "What errors are in it?", a folder "What is this project for?".
//! Only questions Filescape can give the model what it needs for are offered: one that needs the
//! file's text only where Filescape can read it ([`crate::askabout::can_read`]), and none about
//! what's in a picture until a model that can see is supported.

use std::path::Path;

/// What sort of thing it is, as far as Ask AI's suggestions go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Licence,
    Readme,
    Log,
    Data,
    Config,
    Code,
    Document,
    Image,
    AiModel,
    Folder,
    Other,
}

/// The picture each suggestion gets in the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Summary,
    Explain,
    Search,
    Warning,
    Check,
    Folder,
    Storage,
    Chat,
}

/// One suggested question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// What the button says.
    pub label: &'static str,
    /// What is asked.
    pub question: &'static str,
    /// It needs the file's contents, not only its name, size and dates.
    pub read: bool,
    pub mark: Mark,
}

const fn s(label: &'static str, question: &'static str, read: bool, mark: Mark) -> Suggestion {
    Suggestion { label, question, read, mark }
}

/// What `name` is. `is_dir` for folders (an app bundle or other package counts as a file).
pub fn sort_of(name: &str, is_dir: bool) -> Sort {
    if is_dir {
        return Sort::Folder;
    }
    let lower = name.to_ascii_lowercase();
    let stem = lower.split('.').next().unwrap_or("");
    let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
    if matches!(stem, "license" | "licence" | "copying" | "unlicense" | "notice" | "copyright" | "eula" | "patents") || ext == "license" {
        return Sort::Licence;
    }
    if stem == "readme" || stem == "read_me" {
        return Sort::Readme;
    }
    match ext {
        "log" | "out" | "err" | "crash" | "ips" | "dmp" => Sort::Log,
        "csv" | "tsv" | "xlsx" | "xlsm" | "xls" | "xlsb" | "ods" | "numbers" | "parquet" => Sort::Data,
        "json" | "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "config" | "plist" | "env" | "properties" | "xml" | "reg" | "service" | "desktop" => Sort::Config,
        "rs" | "py" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "c" | "h" | "cc" | "cpp" | "hpp" | "cs" | "java" | "kt" | "swift" | "m" | "mm" | "go" | "rb" | "php" | "pl"
        | "lua" | "r" | "scala" | "sh" | "bash" | "zsh" | "fish" | "ps1" | "psm1" | "bat" | "cmd" | "sql" | "vbs" | "applescript" | "scpt" | "glsl" | "hlsl" | "wgsl"
        | "cmake" | "gradle" | "mk" | "vex" | "nk" | "hip" | "mel" | "ma" => Sort::Code,
        "pdf" | "doc" | "docx" | "docm" | "odt" | "rtf" | "txt" | "md" | "markdown" | "rst" | "tex" | "pages" | "epub" | "ppt" | "pptx" | "odp" | "key" | "html" | "htm"
        | "eml" | "msg" => Sort::Document,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "heif" | "avif" | "tif" | "tiff" | "bmp" | "svg" | "psd" | "exr" | "dng" | "cr2" | "cr3" | "nef" | "arw" | "raf"
        | "orf" | "rw2" => Sort::Image,
        "gguf" | "safetensors" | "ckpt" | "onnx" | "pt" | "pth" | "mlmodel" | "tflite" | "ggml" => Sort::AiModel,
        _ if matches!(stem, "makefile" | "dockerfile" | "justfile" | "rakefile" | "gemfile" | "vagrantfile") => Sort::Code,
        _ if lower.starts_with('.') && !lower[1..].contains('.') => Sort::Config,
        _ => Sort::Other,
    }
}

/// The questions to offer for `sort`, best first. `readable`: Filescape can read its text.
pub fn for_sort(sort: Sort, readable: bool) -> Vec<Suggestion> {
    use Mark as M;
    let all: &[Suggestion] = match sort {
        Sort::Licence => &[
            s("Summarise this licence", "Summarise this licence in plain English.", true, M::Summary),
            s("Can I use this commercially?", "Can I use the work this licence covers commercially? Quote the parts of the licence that say so.", true, M::Check),
            s("What restrictions apply?", "What restrictions and conditions does this licence set, such as attribution, sharing alike or patents?", true, M::Warning),
            s("Explain in plain English", "Explain this licence in plain English, for someone who isn't a lawyer.", true, M::Explain),
        ],
        Sort::Readme => &[
            s("What is this project?", "What is this project, and what is it for?", true, M::Explain),
            s("How do I install it?", "Summarise the installation and setup instructions, step by step.", true, M::Summary),
            s("How do I use it?", "How is it used? Give the main commands or steps.", true, M::Chat),
        ],
        Sort::Log => &[
            s("Find the errors", "What errors and warnings are in this log? List them with when they happened.", true, M::Warning),
            s("What went wrong?", "What most likely went wrong, judging by this log, and why?", true, M::Search),
            s("Suggest fixes", "What could fix the problems in this log? Say which are guesses.", true, M::Check),
        ],
        Sort::Data => &[
            s("Summarise the data", "Summarise what this data contains.", true, M::Summary),
            s("Explain the columns", "Explain what each column holds.", true, M::Explain),
            s("Spot patterns", "What patterns, outliers or gaps stand out in this data?", true, M::Search),
        ],
        Sort::Config => &[
            s("Explain the settings", "Explain what these settings do, one by one.", true, M::Explain),
            s("Look for problems", "Are there settings here that look wrong, risky or inconsistent?", true, M::Warning),
        ],
        Sort::Code => &[
            s("Explain this code", "Explain what this code does, step by step.", true, M::Explain),
            s("Find possible issues", "Are there bugs, risky operations or security problems in this code?", true, M::Warning),
            s("Suggest improvements", "How could this code be clearer or more robust?", true, M::Check),
        ],
        Sort::Document => &[
            s("Summarise", "Summarise this document.", true, M::Summary),
            s("Key points", "List the key points and any dates, numbers or names that matter.", true, M::Search),
            s("Explain in plain English", "Explain this document in plain English.", true, M::Explain),
        ],
        Sort::Image => &[s("What could this be?", "What is this file likely to be, judging by its name, type, size and dates?", false, M::Chat)],
        Sort::AiModel => &[
            s("What is this model?", "What is this AI model likely to be, and what is it for, judging by its name, format and size?", false, M::Explain),
            s("Could my computer run it?", "Roughly how much memory would this model need to run, judging by its size and format?", false, M::Check),
        ],
        Sort::Folder => &[
            s("What is this folder?", "What is this folder, and what is it for?", false, M::Folder),
            s("Explain what's in it", "Explain what the items in this folder are.", false, M::Explain),
            s("What's using the space?", "What is using the most space in this folder?", false, M::Storage),
            s("What might be unnecessary?", "Which items here might be unnecessary (caches, old builds, downloads, duplicates)? These are only guesses from names and sizes.", false, M::Warning),
        ],
        Sort::Other => &[
            s("What is this file?", "What is this file likely to be, and what opens it?", false, M::Explain),
            s("Summarise", "Summarise this file.", true, M::Summary),
        ],
    };
    all.iter().copied().filter(|x| readable || !x.read).collect()
}

/// The suggestions for the file or folder at `path`.
pub fn for_path(path: &Path, is_dir: bool) -> Vec<Suggestion> {
    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let sort = sort_of(&name, is_dir);
    let readable = !is_dir && crate::askabout::can_read(path);
    let mut v = for_sort(sort, readable);
    if v.is_empty() {
        // Nothing Filescape can read in it: what it is, from its name and details.
        v.push(s("What is this file?", "What is this file likely to be, and what opens it?", false, Mark::Explain));
    }
    v
}

/// The question box's hint for `sort`.
pub fn hint(sort: Sort) -> &'static str {
    match sort {
        Sort::Folder => "Ask anything about this folder…",
        _ => "Ask anything about this file…",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts() {
        assert_eq!(sort_of("LICENSE", false), Sort::Licence);
        assert_eq!(sort_of("COPYING.txt", false), Sort::Licence);
        assert_eq!(sort_of("mit.license", false), Sort::Licence);
        assert_eq!(sort_of("README.md", false), Sort::Readme);
        assert_eq!(sort_of("system.log", false), Sort::Log);
        assert_eq!(sort_of("sales.CSV", false), Sort::Data);
        assert_eq!(sort_of("config.toml", false), Sort::Config);
        assert_eq!(sort_of(".gitignore", false), Sort::Config);
        assert_eq!(sort_of("main.rs", false), Sort::Code);
        assert_eq!(sort_of("Makefile", false), Sort::Code);
        assert_eq!(sort_of("report.pdf", false), Sort::Document);
        assert_eq!(sort_of("IMG_0001.HEIC", false), Sort::Image);
        assert_eq!(sort_of("qwen3-1.7b-q4_k_m.gguf", false), Sort::AiModel);
        assert_eq!(sort_of("src", true), Sort::Folder);
        assert_eq!(sort_of("mystery.xyz", false), Sort::Other);
    }

    #[test]
    fn licence_questions_need_the_text() {
        let v = for_sort(Sort::Licence, true);
        assert_eq!(v[0].label, "Summarise this licence");
        assert!(v.iter().any(|x| x.label == "Can I use this commercially?"));
        assert!(v.iter().all(|x| x.read));
        // Unreadable: nothing that needs the text is offered.
        assert!(for_sort(Sort::Licence, false).is_empty());
    }

    #[test]
    fn folders_never_read_contents() {
        assert!(for_sort(Sort::Folder, false).iter().all(|x| !x.read));
        assert_eq!(for_sort(Sort::Folder, false).len(), 4);
    }

    #[test]
    fn pictures_are_not_described_without_vision() {
        assert!(for_sort(Sort::Image, true).iter().all(|x| !x.label.contains("Describe") && !x.read));
    }

    #[test]
    fn unreadable_files_still_get_a_question() {
        let d = std::env::temp_dir().join(format!("filescape-suggest-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("blob.pdf");
        std::fs::write(&p, [0u8, 1, 2, 3, 0, 9]).unwrap();
        let v = for_path(&p, false);
        assert!(!v.is_empty() && v.iter().all(|x| !x.read), "{v:?}");
        let t = d.join("notes.txt");
        std::fs::write(&t, "hello").unwrap();
        assert!(for_path(&t, false).iter().any(|x| x.read));
        let _ = std::fs::remove_dir_all(&d);
    }
}
