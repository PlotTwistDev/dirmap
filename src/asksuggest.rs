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
/// `vision`: the loaded model can look at it (a picture).
fn for_sort(sort: Sort, readable: bool, vision: bool) -> Vec<Suggestion> {
    use Mark as M;
    if sort == Sort::Image && vision {
        return vec![
            s("Describe this picture", "Describe this picture: what it shows, and anything that stands out.", true, M::Explain),
            s("What text is in it?", "Write out any text you can see in this picture. Say if some of it is hard to read.", true, M::Search),
            s("What could this be?", "What is this picture likely to be, judging by what it shows, its name and its details?", true, M::Chat),
        ];
    }
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
            s("What is this model?", "What is this AI model, and what is it for? Use what Filescape read from the file.", false, M::Explain),
            s("Could my computer run it?", "Could this computer run this model, and how much memory would it need? Use what Filescape read from the file.", false, M::Check),
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
    let seeing = sort == Sort::Image && vision() && crate::askabout::can_show(path);
    let mut v = for_sort(sort, readable, seeing);
    if v.is_empty() {
        // Nothing Filescape can read in it: what it is, from its name and details.
        v.push(s("What is this file?", "What is this file likely to be, and what opens it?", false, Mark::Explain));
    }
    v
}

/// Ask AI is shown in the Files workspace (Settings › Files › Ask AI). Set from the settings,
/// read by the right-click menus, which don't see the settings themselves.
static SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_shown(on: bool) {
    SHOWN.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn shown() -> bool {
    SHOWN.load(std::sync::atomic::Ordering::Relaxed)
}

/// The loaded model can look at pictures (it has a vision projector). Set when a model loads.
static VISION: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_vision(on: bool) {
    VISION.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn vision() -> bool {
    VISION.load(std::sync::atomic::Ordering::Relaxed)
}

/// What the right-click menu's Ask AI submenu asks. `Ask` opens the conversation with nothing
/// asked yet; the others ask straight away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Ask,
    Summarise,
    Explain,
    FolderContents,
    FolderDocuments,
    FolderStructure,
    FolderUnneeded,
    FolderPurpose,
    FolderStorage,
    FolderRelated,
    ManySummarise,
    ManyCompare,
    ManyCommon,
}

/// What to give the model for a preset, beyond names, sizes and dates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Needs {
    /// Names, sizes and dates only.
    Facts,
    /// The text of the chosen files.
    Contents,
    /// The text of the documents directly inside the folder.
    FolderDocuments,
    /// The folder, and what sits beside it in its parent folder.
    Siblings,
}

impl Preset {
    /// The menu item, for one file, one folder or several items.
    pub fn label(self) -> &'static str {
        match self {
            Preset::Ask => "Ask About It…",
            Preset::Summarise => "Summarise",
            Preset::Explain => "Explain Its Contents",
            Preset::FolderContents => "Explain Its Contents",
            Preset::FolderDocuments => "Summarise Documents",
            Preset::FolderStructure => "Understand the Folder Structure",
            Preset::FolderUnneeded => "Identify Possibly Unnecessary Files",
            Preset::FolderPurpose => "Explain What This Project Is For",
            Preset::FolderStorage => "Analyse Storage Use",
            Preset::FolderRelated => "Find Related Files",
            Preset::ManySummarise => "Summarise These",
            Preset::ManyCompare => "Compare These",
            Preset::ManyCommon => "What Do These Have in Common?",
        }
    }

    pub fn question(self) -> Option<&'static str> {
        Some(match self {
            Preset::Ask => return None,
            Preset::Summarise => "Summarise this file.",
            Preset::Explain => "Explain what this file contains, in plain English.",
            Preset::FolderContents => "Explain what the items in this folder are.",
            Preset::FolderDocuments => "Summarise each of the documents in this folder in a line or two, naming each one.",
            Preset::FolderStructure => "Explain how this folder is organised: what each part is for and how they fit together.",
            Preset::FolderUnneeded => "Which items here might be unnecessary (caches, old builds, downloads, duplicates)? These are only guesses from names and sizes; say how sure you are.",
            Preset::FolderPurpose => "What is this project or folder for, judging by what's in it?",
            Preset::FolderStorage => "What is using the most space in this folder, and what could I look at to free some?",
            Preset::FolderRelated => "Which of the other items beside this folder look related to it, and why?",
            Preset::ManySummarise => "Summarise each of these, naming each one.",
            Preset::ManyCompare => "Compare these: what are the main differences and similarities?",
            Preset::ManyCommon => "What do these have in common, and how are they related?",
        })
    }

    pub fn needs(self) -> Needs {
        match self {
            Preset::Summarise | Preset::Explain | Preset::ManySummarise | Preset::ManyCompare | Preset::ManyCommon => Needs::Contents,
            Preset::FolderDocuments => Needs::FolderDocuments,
            Preset::FolderRelated => Needs::Siblings,
            _ => Needs::Facts,
        }
    }

    /// The submenu for one file.
    pub const FILE: [Preset; 3] = [Preset::Ask, Preset::Summarise, Preset::Explain];
    /// The submenu for one folder.
    pub const FOLDER: [Preset; 8] = [
        Preset::Ask,
        Preset::FolderContents,
        Preset::FolderDocuments,
        Preset::FolderStructure,
        Preset::FolderUnneeded,
        Preset::FolderPurpose,
        Preset::FolderStorage,
        Preset::FolderRelated,
    ];
    /// The submenu for several items.
    pub const MANY: [Preset; 4] = [Preset::Ask, Preset::ManySummarise, Preset::ManyCompare, Preset::ManyCommon];
}

/// The suggestions for several items at once. Reading contents is offered only when they're
/// all files: folders are described by names and sizes.
pub fn for_many(all_files: bool) -> Vec<Suggestion> {
    let mut v = vec![
        s("Summarise each of these", "Summarise each of these, naming each one.", all_files, Mark::Summary),
        s("Compare them", "Compare these: what are the main differences and similarities?", all_files, Mark::Search),
        s("How are they related?", "What do these have in common, and how are they related?", all_files, Mark::Chat),
    ];
    if !all_files {
        v.push(s("What's using the space?", "Which of these use the most space, and what is in them?", false, Mark::Storage));
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
    fn presets_ask_something() {
        for p in Preset::FILE.iter().chain(&Preset::FOLDER).chain(&Preset::MANY) {
            assert_eq!(p.question().is_none(), *p == Preset::Ask, "{p:?}");
            assert!(!p.label().is_empty());
        }
        // Folders are described from names and sizes, except where reading is the point.
        assert_eq!(Preset::FolderUnneeded.needs(), Needs::Facts);
        assert_eq!(Preset::FolderDocuments.needs(), Needs::FolderDocuments);
        assert_eq!(Preset::ManyCompare.needs(), Needs::Contents);
    }

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
        let v = for_sort(Sort::Licence, true, false);
        assert_eq!(v[0].label, "Summarise this licence");
        assert!(v.iter().any(|x| x.label == "Can I use this commercially?"));
        assert!(v.iter().all(|x| x.read));
        // Unreadable: nothing that needs the text is offered.
        assert!(for_sort(Sort::Licence, false, false).is_empty());
    }

    #[test]
    fn folders_never_read_contents() {
        assert!(for_sort(Sort::Folder, false, false).iter().all(|x| !x.read));
        assert_eq!(for_sort(Sort::Folder, false, false).len(), 4);
    }

    #[test]
    fn pictures_are_not_described_without_vision() {
        assert!(for_sort(Sort::Image, true, false).iter().all(|x| !x.label.contains("Describe") && !x.read));
    }

    #[test]
    fn pictures_are_described_with_vision() {
        let v = for_sort(Sort::Image, false, true);
        assert_eq!(v[0].label, "Describe this picture");
        assert!(v.iter().all(|x| x.read));
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
