//! Finding AI models in a scan: language and image models saved by Ollama, LM Studio, Hugging
//! Face or by hand. Files that make up one model (shards, Ollama layers) are grouped together,
//! GGUF and safetensors headers are read for the model's family, size and precision, and
//! byte-identical copies are found with [`crate::dupes`].
//!
//! Only headers are read (a few KB to a few MB per model), never whole files, except to confirm
//! copies, and then only for files whose size matches another model file. Files that are only in
//! the cloud are never read: that would download them.

use crate::scan::{self, Node};
use std::collections::HashMap;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Smaller files are configs, tokenizers and the like, not weights.
pub const MIN_SIZE: u64 = 16 << 20;

/// Which app a model belongs to, from where it is saved.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Source {
    Ollama,
    LmStudio,
    HuggingFace,
    /// Saved anywhere else: downloads, ComfyUI or Stable Diffusion folders, projects.
    Folder,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Ollama => "Ollama",
            Source::LmStudio => "LM Studio",
            Source::HuggingFace => "Hugging Face",
            Source::Folder => "Folder",
        }
    }
}

/// How the weights are stored.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Format {
    /// llama.cpp's format, used by Ollama and LM Studio. The only one Filescape can run.
    Gguf,
    Safetensors,
    /// PyTorch pickles: .pt, .pth, .ckpt and pytorch_model*.bin.
    PyTorch,
    Onnx,
    CoreMl,
    Other,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Gguf => "GGUF",
            Format::Safetensors => "safetensors",
            Format::PyTorch => "PyTorch",
            Format::Onnx => "ONNX",
            Format::CoreMl => "Core ML",
            Format::Other => "other",
        }
    }

    fn of_ext(ext: &str) -> Option<Format> {
        Some(match ext {
            "gguf" | "llamafile" => Format::Gguf,
            "safetensors" => Format::Safetensors,
            "pt" | "pth" | "ckpt" | "bin" => Format::PyTorch,
            "onnx" => Format::Onnx,
            "mlmodel" => Format::CoreMl,
            "ggml" | "h5" | "keras" | "tflite" | "npz" | "npy" | "pb" | "engine" | "mar" | "pkl" | "joblib" | "lora"
            | "nemo" => Format::Other,
            _ => return None,
        })
    }

    /// From the first bytes of a file without a useful extension (Ollama and Hugging Face blobs).
    fn sniff(head: &[u8]) -> Option<Format> {
        if head.starts_with(b"GGUF") {
            return Some(Format::Gguf);
        }
        // safetensors: an 8-byte header length, then the JSON header.
        if head.len() >= 9 && head[8] == b'{' && u64::from_le_bytes(head[..8].try_into().unwrap()) < 1 << 30 {
            return Some(Format::Safetensors);
        }
        // PyTorch saves a zip ("PK") or, older, a pickle (0x80 then the protocol).
        if head.starts_with(b"PK\x03\x04") || (head.len() > 1 && head[0] == 0x80 && head[1] <= 5) {
            return Some(Format::PyTorch);
        }
        None
    }
}

/// What a GGUF or safetensors header says about the model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    /// The name the model gives itself (GGUF `general.name`).
    pub name: Option<String>,
    /// Model family: llama, qwen3, gemma3, ...
    pub arch: Option<String>,
    /// "8B", "1.7B", ... from GGUF `general.size_label` or counted from safetensors shapes.
    pub params: Option<String>,
    /// Quantization or precision: Q4_K_M, F16, BF16, ...
    pub quant: Option<String>,
    /// The longest prompt the model was trained for, in tokens.
    pub context: Option<u64>,
    /// GGUF vision projectors (mmproj) and LoRA adapters go with a model; they don't run alone.
    pub companion: bool,
}

/// One file of a model.
#[derive(Clone, Debug)]
pub struct File {
    pub path: PathBuf,
    pub size: u64,
    /// Only in the cloud (or partly): not read, not compared.
    pub not_local: bool,
}

/// One model: the files that make it up, and what is known about it.
#[derive(Clone, Debug)]
pub struct Model {
    pub name: String,
    pub source: Source,
    pub format: Format,
    pub info: Info,
    pub files: Vec<File>,
    /// Total size of the files (logical bytes).
    pub size: u64,
    /// Other models (indexes into the same list) whose files are byte-identical to some of this
    /// model's files.
    pub copies: Vec<usize>,
    /// Bytes of this model that are identical to a file of a model listed before it: what
    /// deleting this copy would free.
    pub extra: u64,
    /// For Ollama: blobs no model uses any more (left behind by `ollama rm` or failed pulls).
    pub orphan: bool,
}

impl Model {
    /// Filescape can load and run this model itself (phase 2): a whole GGUF model on this computer.
    pub fn runnable(&self) -> bool {
        self.format == Format::Gguf && !self.info.companion && !self.orphan && self.files.iter().all(|f| !f.not_local)
    }

    /// "llama · 8B · Q4_K_M · 8K context".
    pub fn details(&self) -> String {
        let mut parts = vec![self.format.label().to_string()];
        let i = &self.info;
        parts.extend(i.arch.clone());
        parts.extend(i.params.clone());
        parts.extend(i.quant.clone());
        if let Some(c) = i.context {
            parts.push(format!("{} context", tokens(c)));
        }
        if i.companion {
            parts.push("add-on".into());
        }
        if self.files.len() > 1 {
            parts.push(format!("{} files", self.files.len()));
        }
        parts.join(" · ")
    }
}

/// 131072 → "128K".
fn tokens(n: u64) -> String {
    if n >= 1024 && n % 1024 == 0 { format!("{}K", n / 1024) } else if n >= 1000 { format!("{}K", n / 1000) } else { n.to_string() }
}

/// Progress of [`find`]: copies are checked by reading files, which can take a while.
pub use crate::dupes::Progress;

/// Every model in a scanned tree, largest first, with copies marked. `root` is the scanned
/// folder on disk. Returns `None` when cancelled.
pub fn find(tree: &Node, root: &Path, progress: &Progress) -> Option<Vec<Model>> {
    let mut models = find_in(tree, root, &progress.cancel)?;
    finish(&mut models, progress)?;
    Some(models)
}

/// The models in one scanned tree, before copies are looked for. Lists from several trees
/// are joined, then passed to [`finish`].
pub fn find_in(tree: &Node, root: &Path, cancel: &AtomicBool) -> Option<Vec<Model>> {
    let mut found = Found::default();
    // The scanned folder can itself be an Ollama, LM Studio or Hugging Face folder.
    let place = if is_ollama_store(tree) {
        found.ollama.push((root.to_path_buf(), Vec::new()));
        Place::Ollama
    } else if let Some(repo) = root.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix("models--")) {
        found.hf.push((repo.replacen("--", "/", 1), Vec::new()));
        Place::HuggingFace(0)
    } else if is_lm_studio(root) {
        Place::LmStudio
    } else {
        Place::Plain
    };
    walk(tree, &mut root.to_path_buf(), place, &mut found, cancel)?;
    found.finish(cancel)
}

/// Drops models listed twice (from overlapping folders), sorts by size and marks copies.
pub fn finish(models: &mut Vec<Model>, progress: &Progress) -> Option<()> {
    let mut seen = std::collections::HashSet::new();
    models.retain(|m| m.files.first().is_none_or(|f| seen.insert(f.path.clone())));
    // Among copies of the same size, the one an app manages comes first, so the loose copy (in
    // Downloads, say) is the one marked "copy".
    models.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| (a.source == Source::Folder).cmp(&(b.source == Source::Folder))).then_with(|| a.name.cmp(&b.name)));
    for m in models.iter_mut() {
        m.copies.clear();
        m.extra = 0;
    }
    mark_copies(models, progress)
}

/// Where Ollama, LM Studio and Hugging Face keep models on this computer, and Downloads (where
/// model files often land), as (label, folder). Only folders that exist.
pub fn known_places() -> Vec<(&'static str, PathBuf)> {
    let mut out: Vec<(&'static str, PathBuf)> = Vec::new();
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let home = crate::fsys::places::home();
    if let Some(p) = env("OLLAMA_MODELS") {
        out.push(("Ollama", p));
    }
    if let Some(h) = &home {
        out.push(("Ollama", h.join(".ollama").join("models")));
    }
    // The Linux service keeps them under its own user.
    #[cfg(target_os = "linux")]
    out.push(("Ollama", PathBuf::from("/usr/share/ollama/.ollama/models")));
    if let Some(h) = &home {
        out.push(("LM Studio", h.join(".lmstudio").join("models")));
        out.push(("LM Studio", h.join(".cache").join("lm-studio").join("models")));
    }
    if let Some(p) = env("HF_HUB_CACHE") {
        out.push(("Hugging Face", p));
    } else if let Some(p) = env("HF_HOME") {
        out.push(("Hugging Face", p.join("hub")));
    } else if let Some(h) = &home {
        out.push(("Hugging Face", h.join(".cache").join("huggingface").join("hub")));
    }
    if let Some(h) = &home {
        out.push(("Downloads", h.join("Downloads")));
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|(_, p)| p.is_dir() && seen.insert(p.clone()));
    out
}

/// What kind of folder the walk is in.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    Plain,
    LmStudio,
    /// Inside an Ollama models folder (the one holding `blobs` and `manifests`).
    Ollama,
    /// Inside `models--org--name` of a Hugging Face cache; the index is into `Found::hf`.
    HuggingFace(usize),
}

#[derive(Default)]
struct Found {
    /// Ollama model folders (`.../models`) and the blob files found in them.
    ollama: Vec<(PathBuf, Vec<File>)>,
    /// Hugging Face repos: ("org/name", files).
    hf: Vec<(String, Vec<File>)>,
    /// Everything else, by folder: (source, folder, files).
    loose: Vec<(Source, PathBuf, File)>,
}

fn walk(n: &Node, disk: &mut PathBuf, place: Place, out: &mut Found, cancel: &AtomicBool) -> Option<()> {
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    for c in &n.children {
        // A folder this small can't hold a model. (Children are sorted by the size shown, which
        // can be the size on disk, so this can't stop at the first small one.)
        if c.logical < MIN_SIZE {
            continue;
        }
        disk.push(&*c.name);
        if c.is_dir {
            let inner = match place {
                Place::Plain | Place::LmStudio if is_ollama_store(c) => {
                    out.ollama.push((disk.clone(), Vec::new()));
                    Place::Ollama
                }
                Place::Plain | Place::LmStudio if c.name.starts_with("models--") => {
                    let repo = c.name["models--".len()..].replacen("--", "/", 1);
                    out.hf.push((repo, Vec::new()));
                    Place::HuggingFace(out.hf.len() - 1)
                }
                Place::Plain if is_lm_studio(disk) => Place::LmStudio,
                p => p,
            };
            walk(c, disk, inner, out, cancel)?;
        } else {
            let file = File { path: disk.clone(), size: c.logical, not_local: c.flags & scan::store::NOT_LOCAL != 0 };
            match place {
                Place::Ollama => {
                    if c.name.starts_with("sha256") {
                        out.ollama.last_mut().unwrap().1.push(file);
                    }
                }
                Place::HuggingFace(k) => {
                    // Blobs have no extension; snapshot files (Windows without symlinks) have one.
                    let ext = scan::ext_of(&c.name).to_ascii_lowercase();
                    if ext.is_empty() || Format::of_ext(&ext).is_some() {
                        out.hf[k].1.push(file);
                    }
                }
                Place::LmStudio | Place::Plain => {
                    if crate::categories::of(&c.name) == crate::categories::Category::Models {
                        let src = if place == Place::LmStudio { Source::LmStudio } else { Source::Folder };
                        out.loose.push((src, disk.parent().unwrap_or(disk).to_path_buf(), file));
                    }
                }
            }
        }
        disk.pop();
    }
    Some(())
}

/// Ollama keeps models in a folder (`~/.ollama/models`, or wherever OLLAMA_MODELS points) with
/// `blobs` and `manifests` inside.
fn is_ollama_store(n: &Node) -> bool {
    n.children.iter().any(|c| c.is_dir && &*c.name == "blobs") && n.name.as_ref() != "blobs" && {
        // `manifests` is tiny, so it may be anywhere in the size order.
        n.children.iter().any(|c| c.is_dir && &*c.name == "manifests")
    }
}

/// `~/.lmstudio/models` or the older `~/.cache/lm-studio/models`.
fn is_lm_studio(dir: &Path) -> bool {
    let mut it = dir.components().rev().map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase());
    let (Some(last), Some(parent)) = (it.next(), it.next()) else { return false };
    last == "models" && (parent == ".lmstudio" || parent == "lm-studio")
}

impl Found {
    fn finish(self, cancel: &AtomicBool) -> Option<Vec<Model>> {
        let mut models = Vec::new();
        for (store, blobs) in self.ollama {
            models.extend(ollama_models(&store, blobs, cancel)?);
        }
        for (repo, files) in self.hf {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            models.extend(hf_models(&repo, files));
        }
        let mut by_key: HashMap<(Source, PathBuf, String), Vec<File>> = HashMap::new();
        for (src, dir, f) in self.loose {
            let key = loose_key(&f.path);
            by_key.entry((src, dir, key)).or_default().push(f);
        }
        for ((src, dir, key), mut files) in by_key {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            files.sort_by(|a, b| a.path.cmp(&b.path));
            let format = format_of(&files[0]);
            let info = read_info(format, &files);
            let name = match &info.name {
                Some(n) if format == Format::Gguf && !n.is_empty() => n.clone(),
                _ => display_name(&dir, &key),
            };
            let size = files.iter().map(|f| f.size).sum();
            models.push(Model { name, source: src, format, info, files, size, copies: Vec::new(), extra: 0, orphan: false });
        }
        models.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
        Some(models)
    }
}

/// Files that are one model share a key: shards (`x-00001-of-00004.gguf`) and the generic names
/// of a Hugging Face style folder (`model.safetensors`, `pytorch_model-00001-of-00002.bin`) are
/// grouped per folder; other files are models of their own.
fn loose_key(file: &Path) -> String {
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let stem = shard_stem(&name);
    if is_generic_stem(stem) { String::new() } else { stem.to_string() }
}

/// "model-00001-of-00004.safetensors" → "model"; "x.Q4_K_M.gguf" → "x.Q4_K_M".
fn shard_stem(name: &str) -> &str {
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    // "-00001-of-00004"
    if let Some((before, after)) = stem.rsplit_once("-of-")
        && after.len() >= 3
        && after.bytes().all(|b| b.is_ascii_digit())
        && let Some((base, num)) = before.rsplit_once('-')
        && num.len() >= 3
        && num.bytes().all(|b| b.is_ascii_digit())
    {
        return base;
    }
    stem
}

fn is_generic_stem(stem: &str) -> bool {
    matches!(
        stem.to_ascii_lowercase().as_str(),
        "model" | "pytorch_model" | "diffusion_pytorch_model" | "consolidated" | "adapter_model" | "tf_model" | "flax_model"
            | "weights" | "model_weights" | "openvino_model"
    ) || stem.starts_with("consolidated.")
}

/// A name for a model from its folder and file stem.
fn display_name(dir: &Path, key: &str) -> String {
    if !key.is_empty() {
        return key.to_string();
    }
    let mut parts = dir.iter().rev().map(|c| c.to_string_lossy().to_string());
    let last = parts.next().unwrap_or_default();
    // Diffusers keep each part of a model in its own folder: name it "model/unet".
    if matches!(last.as_str(), "unet" | "vae" | "text_encoder" | "text_encoder_2" | "text_encoder_3" | "transformer" | "vae_decoder" | "vae_encoder")
        && let Some(parent) = parts.next()
    {
        return format!("{parent}/{last}");
    }
    last
}

fn format_of(f: &File) -> Format {
    let name = f.path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match Format::of_ext(scan::ext_of(&name)) {
        Some(fmt) => fmt,
        None if f.not_local => Format::Other,
        None => read_head(&f.path).and_then(|h| Format::sniff(&h)).unwrap_or(Format::Other),
    }
}

fn read_head(p: &Path) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; 16];
    let n = std::fs::File::open(p).ok()?.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}

/// Reads the header of the model's first file (that's where GGUF splits keep the metadata).
fn read_info(format: Format, files: &[File]) -> Info {
    let Some(first) = files.iter().find(|f| !f.not_local) else { return Info::default() };
    match format {
        Format::Gguf => gguf_info(&first.path).unwrap_or_default(),
        Format::Safetensors => {
            // Parameters are counted over all shards.
            let mut info = Info::default();
            let mut total = 0u64;
            let mut dtypes: HashMap<String, u64> = HashMap::new();
            for f in files.iter().filter(|f| !f.not_local && format_of(f) == Format::Safetensors) {
                if let Some((n, d)) = safetensors_params(&f.path) {
                    total += n;
                    for (k, v) in d {
                        *dtypes.entry(k).or_default() += v;
                    }
                }
            }
            if total > 0 {
                info.params = Some(params_label(total));
                info.quant = dtypes.into_iter().max_by_key(|(_, n)| *n).map(|(d, _)| d);
            }
            info
        }
        _ => Info::default(),
    }
}

/// 8_030_261_248 → "8B"; 1_720_000_000 → "1.7B"; 335_000_000 → "335M".
pub fn params_label(n: u64) -> String {
    let f = n as f64;
    if f >= 1e9 {
        let b = f / 1e9;
        if b >= 10.0 { format!("{:.0}B", b) } else { format!("{}B", trim(b)) }
    } else if f >= 1e6 {
        format!("{:.0}M", f / 1e6)
    } else {
        format!("{:.0}K", f / 1e3)
    }
}

fn trim(x: f64) -> String {
    let s = format!("{x:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

// ---------- Ollama ----------

/// Reads Ollama's manifests (`manifests/<host>/<namespace>/<model>/<tag>`, small JSON files) to
/// name the blobs, which are only called by their checksum.
fn ollama_models(store: &Path, blobs: Vec<File>, cancel: &AtomicBool) -> Option<Vec<Model>> {
    let mut by_digest: HashMap<String, File> =
        blobs.into_iter().map(|f| (f.path.file_name().unwrap().to_string_lossy().replace(':', "-"), f)).collect();
    let mut manifests = Vec::new();
    list_files(&store.join("manifests"), 5, &mut manifests);
    manifests.sort();
    let mut out: Vec<Model> = Vec::new();
    let mut used: HashMap<String, usize> = HashMap::new();
    for m in manifests {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let Some(name) = ollama_name(&store.join("manifests"), &m) else { continue };
        let Ok(text) = std::fs::read_to_string(&m) else { continue };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let mut files = Vec::new();
        let mut main = None;
        for layer in json["layers"].as_array().into_iter().flatten() {
            let media = layer["mediaType"].as_str().unwrap_or("");
            let Some(digest) = layer["digest"].as_str() else { continue };
            let key = digest.replace(':', "-");
            if !(media.ends_with(".model") || media.ends_with(".projector") || media.ends_with(".adapter")) {
                continue;
            }
            // The same blob can be used by several tags (llama3.2 and llama3.2:latest).
            if let Some(&k) = used.get(&key) {
                if let Some(other) = out.get_mut(k)
                    && !other.name.split(", ").any(|n| n == name)
                {
                    other.name = format!("{}, {name}", other.name);
                }
                continue;
            }
            if let Some(f) = by_digest.remove(&key) {
                if media.ends_with(".model") {
                    main = Some(files.len());
                }
                used.insert(key, out.len());
                files.push(f);
            }
        }
        if files.is_empty() {
            continue;
        }
        let first = main.unwrap_or(0);
        files.swap(0, first);
        let mut info = if files[0].not_local { Info::default() } else { gguf_info(&files[0].path).unwrap_or_default() };
        if main.is_none() {
            info.companion = true;
        }
        let size = files.iter().map(|f| f.size).sum();
        out.push(Model { name, source: Source::Ollama, format: Format::Gguf, info, files, size, copies: Vec::new(), extra: 0, orphan: false });
    }
    // Large blobs no manifest mentions.
    let mut left: Vec<File> = by_digest.into_values().collect();
    left.sort_by(|a, b| b.size.cmp(&a.size));
    for f in left {
        let format = format_of(&f);
        let info = if format == Format::Gguf && !f.not_local { gguf_info(&f.path).unwrap_or_default() } else { Info::default() };
        let name = info.name.clone().unwrap_or_else(|| "Unused Ollama download".into());
        let size = f.size;
        out.push(Model { name, source: Source::Ollama, format, info, files: vec![f], size, copies: Vec::new(), extra: 0, orphan: true });
    }
    Some(out)
}

/// `manifests/registry.ollama.ai/library/llama3.2/3b` → "llama3.2:3b";
/// `manifests/hf.co/bartowski/Qwen3-8B-GGUF/Q4_K_M` → "hf.co/bartowski/Qwen3-8B-GGUF:Q4_K_M".
fn ollama_name(manifests: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(manifests).ok()?;
    let parts: Vec<String> = rel.iter().map(|c| c.to_string_lossy().to_string()).collect();
    let (tag, rest) = parts.split_last()?;
    let model = match rest {
        [host, ns, model] if host == "registry.ollama.ai" && ns == "library" => model.clone(),
        [host, ns, model] if host == "registry.ollama.ai" => format!("{ns}/{model}"),
        _ => rest.join("/"),
    };
    Some(if tag == "latest" { model } else { format!("{model}:{tag}") })
}

fn list_files(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() && depth > 0 {
            list_files(&e.path(), depth - 1, out);
        } else if t.is_file() {
            out.push(e.path());
        }
    }
}

// ---------- Hugging Face ----------

/// A Hugging Face cache repo can hold several models: each GGUF file is its own (one per
/// quantization), and the rest (safetensors shards, .bin) is one model.
fn hf_models(repo: &str, files: Vec<File>) -> Vec<Model> {
    let mut gguf = Vec::new();
    let mut rest = Vec::new();
    for f in files {
        match format_of(&f) {
            Format::Gguf => gguf.push(f),
            Format::Other => {}
            fmt => rest.push((fmt, f)),
        }
    }
    let mut out = Vec::new();
    // GGUF shards of one quantization go together.
    let mut by_stem: HashMap<String, Vec<File>> = HashMap::new();
    for f in gguf {
        let name = f.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        // Blobs have no name: key by their own checksum name, so they stay apart.
        by_stem.entry(shard_stem(&name).to_string()).or_default().push(f);
    }
    for (_, mut files) in by_stem {
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let info = read_info(Format::Gguf, &files);
        let size = files.iter().map(|f| f.size).sum();
        let name = match &info.quant {
            Some(q) => format!("{repo} ({q})"),
            None => repo.to_string(),
        };
        out.push(Model { name, source: Source::HuggingFace, format: Format::Gguf, info, files, size, copies: Vec::new(), extra: 0, orphan: false });
    }
    if !rest.is_empty() {
        // The format most of the bytes are in.
        let mut by_fmt: HashMap<Format, u64> = HashMap::new();
        for (fmt, f) in &rest {
            *by_fmt.entry(*fmt).or_default() += f.size;
        }
        let format = by_fmt.into_iter().max_by_key(|(_, n)| *n).map(|(f, _)| f).unwrap_or(Format::Other);
        let mut files: Vec<File> = rest.into_iter().map(|(_, f)| f).collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let info = read_info(format, &files);
        let size = files.iter().map(|f| f.size).sum();
        out.push(Model { name: repo.to_string(), source: Source::HuggingFace, format, info, files, size, copies: Vec::new(), extra: 0, orphan: false });
    }
    out
}

// ---------- copies ----------

/// Finds byte-identical files across models and marks the models that share them.
fn mark_copies(models: &mut [Model], progress: &Progress) -> Option<()> {
    let mut cands = Vec::new();
    for (mi, m) in models.iter().enumerate() {
        for (fi, f) in m.files.iter().enumerate() {
            if !f.not_local {
                // `tree_path` carries (model, file) here; the real path is in `path`.
                cands.push(crate::dupes::Candidate { tree_path: vec![mi as u32, fi as u32], path: f.path.clone(), size: f.size });
            }
        }
    }
    let groups = crate::dupes::find(cands, progress);
    if progress.cancel.load(Ordering::Relaxed) {
        return None;
    }
    for g in groups {
        let mut owners: Vec<usize> = g.files.iter().map(|c| c.tree_path[0] as usize).collect();
        owners.sort_unstable();
        owners.dedup();
        // The first copy in the list (the largest model, as sorted) is the one kept; the others
        // are extra. Copies inside one model (rare) count as extra too.
        let mut seen = std::collections::HashSet::new();
        let mut kept = false;
        let mut sorted = g.files.clone();
        sorted.sort_by_key(|c| (c.tree_path[0], c.tree_path[1]));
        for c in &sorted {
            let mi = c.tree_path[0] as usize;
            if kept {
                models[mi].extra += g.size;
            }
            kept = true;
            seen.insert(mi);
        }
        for &a in &owners {
            for &b in &owners {
                if a != b && !models[a].copies.contains(&b) {
                    models[a].copies.push(b);
                }
            }
        }
    }
    Some(())
}

// ---------- GGUF ----------

/// Reads the metadata at the start of a GGUF file. Arrays (the tokenizer's vocabulary, often a
/// few MB) are skipped without being kept.
pub fn gguf_info(path: &Path) -> Option<Info> {
    let f = std::fs::File::open(path).ok()?;
    let mut r = BufReader::with_capacity(256 * 1024, f);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).ok()?;
    if &magic != b"GGUF" {
        return None;
    }
    let version = read_u32(&mut r)?;
    if version < 2 {
        return None;
    }
    read_u64(&mut r)?; // tensor count
    let kvs = read_u64(&mut r)?;
    if kvs > 100_000 {
        return None;
    }
    let mut strings: HashMap<String, String> = HashMap::new();
    let mut numbers: HashMap<String, u64> = HashMap::new();
    for _ in 0..kvs {
        let key = read_str(&mut r)?;
        let ty = read_u32(&mut r)?;
        match ty {
            8 => {
                let v = read_str(&mut r)?;
                strings.insert(key, v);
            }
            9 => {
                let item = read_u32(&mut r)?;
                let n = read_u64(&mut r)?;
                skip_array(&mut r, item, n)?;
            }
            _ => {
                if let Some(v) = read_scalar(&mut r, ty)? {
                    numbers.insert(key, v);
                }
            }
        }
    }
    let arch = strings.get("general.architecture").cloned();
    let mut info = Info {
        name: strings.get("general.name").cloned().filter(|s| !s.trim().is_empty()),
        params: strings.get("general.size_label").cloned().filter(|s| !s.is_empty()),
        quant: numbers.get("general.file_type").and_then(|&t| ftype_name(t)).map(str::to_string),
        context: arch.as_ref().and_then(|a| numbers.get(&format!("{a}.context_length")).copied()),
        companion: matches!(arch.as_deref(), Some("clip" | "mllama_vision")) || strings.get("general.type").is_some_and(|t| t == "adapter" || t == "mmproj"),
        arch,
    };
    if info.arch.as_deref() == Some("clip") {
        info.arch = Some("vision projector".into());
    }
    Some(info)
}

fn read_u32(r: &mut impl Read) -> Option<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).ok()?;
    Some(u32::from_le_bytes(b))
}

fn read_u64(r: &mut impl Read) -> Option<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b).ok()?;
    Some(u64::from_le_bytes(b))
}

fn read_str(r: &mut impl Read) -> Option<String> {
    let n = read_u64(r)?;
    if n > 1 << 20 {
        return None;
    }
    let mut b = vec![0u8; n as usize];
    r.read_exact(&mut b).ok()?;
    Some(String::from_utf8_lossy(&b).into_owned())
}

/// Size in bytes of a fixed-size GGUF value type.
fn scalar_size(ty: u32) -> Option<u64> {
    Some(match ty {
        0 | 1 | 7 => 1,
        2 | 3 => 2,
        4..=6 => 4,
        10..=12 => 8,
        _ => return None,
    })
}

/// Integer scalars as u64 (others are read and dropped).
fn read_scalar(r: &mut impl Read, ty: u32) -> Option<Option<u64>> {
    let n = scalar_size(ty)?;
    let mut b = [0u8; 8];
    r.read_exact(&mut b[..n as usize]).ok()?;
    Some(match ty {
        0 | 2 | 4 | 10 => Some(u64::from_le_bytes(b)),
        1 | 3 | 5 | 11 => {
            // Sign-extend, then keep only non-negative values.
            let v = match n {
                1 => b[0] as i8 as i64,
                2 => i16::from_le_bytes([b[0], b[1]]) as i64,
                4 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i64,
                _ => i64::from_le_bytes(b),
            };
            u64::try_from(v).ok()
        }
        _ => None,
    })
}

fn skip_array<R: Read + Seek>(r: &mut BufReader<R>, item: u32, n: u64) -> Option<()> {
    match item {
        8 => {
            for _ in 0..n {
                let len = read_u64(r)?;
                if len > 1 << 20 {
                    return None;
                }
                r.seek_relative(len as i64).ok()?;
            }
        }
        9 => {
            for _ in 0..n {
                let inner = read_u32(r)?;
                let m = read_u64(r)?;
                skip_array(r, inner, m)?;
            }
        }
        t => {
            let bytes = scalar_size(t)?.checked_mul(n)?;
            if bytes > 1 << 30 {
                return None;
            }
            r.seek(SeekFrom::Current(bytes as i64)).ok()?;
        }
    }
    Some(())
}

/// llama.cpp's `llama_ftype`: what most of the weights are stored as.
fn ftype_name(t: u64) -> Option<&'static str> {
    Some(match t {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        8 => "Q5_0",
        9 => "Q5_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        19 => "IQ2_XXS",
        20 => "IQ2_XS",
        21 => "Q2_K_S",
        22 => "IQ3_XS",
        23 => "IQ3_XXS",
        24 => "IQ1_S",
        25 => "IQ4_NL",
        26 => "IQ3_S",
        27 => "IQ3_M",
        28 => "IQ2_S",
        29 => "IQ2_M",
        30 => "IQ4_XS",
        31 => "IQ1_M",
        32 => "BF16",
        36 => "TQ1_0",
        37 => "TQ2_0",
        38 => "MXFP4",
        _ => return None,
    })
}

// ---------- safetensors ----------

/// Parameter count and bytes per data type, from a safetensors header (8-byte length, then JSON
/// naming every tensor with its type and shape).
fn safetensors_params(path: &Path) -> Option<(u64, HashMap<String, u64>)> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = read_u64(&mut f)?;
    if len > 100 << 20 {
        return None;
    }
    let mut b = vec![0u8; len as usize];
    f.read_exact(&mut b).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&b).ok()?;
    let mut total = 0u64;
    let mut dtypes: HashMap<String, u64> = HashMap::new();
    for (k, t) in json.as_object()? {
        if k == "__metadata__" {
            continue;
        }
        let n: u64 = t["shape"].as_array()?.iter().map(|d| d.as_u64().unwrap_or(0)).product();
        total += n;
        if let Some(d) = t["dtype"].as_str() {
            *dtypes.entry(d.to_string()).or_default() += n;
        }
    }
    Some((total, dtypes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A minimal GGUF header with the given string and u32 values, plus a string array (as a
    /// tokenizer has) that must be skipped.
    pub(crate) fn write_gguf(path: &Path, strings: &[(&str, &str)], numbers: &[(&str, u32)], pad: usize) {
        let mut b = Vec::new();
        b.extend(b"GGUF");
        b.extend(3u32.to_le_bytes());
        b.extend(0u64.to_le_bytes());
        b.extend(((strings.len() + numbers.len() + 1) as u64).to_le_bytes());
        let s = |b: &mut Vec<u8>, x: &str| {
            b.extend((x.len() as u64).to_le_bytes());
            b.extend(x.as_bytes());
        };
        s(&mut b, "tokenizer.ggml.tokens");
        b.extend(9u32.to_le_bytes());
        b.extend(8u32.to_le_bytes());
        b.extend(3u64.to_le_bytes());
        for t in ["<s>", "</s>", "hello"] {
            s(&mut b, t);
        }
        for (k, v) in strings {
            s(&mut b, k);
            b.extend(8u32.to_le_bytes());
            s(&mut b, v);
        }
        for (k, v) in numbers {
            s(&mut b, k);
            b.extend(4u32.to_le_bytes());
            b.extend(v.to_le_bytes());
        }
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(&b).unwrap();
        // Sparse padding, so the file counts as a model without using disk.
        f.set_len((b.len() + pad) as u64).unwrap();
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("filescape-models-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reads_gguf_header() {
        let d = dir("gguf");
        let p = d.join("x.gguf");
        write_gguf(
            &p,
            &[("general.architecture", "qwen3"), ("general.name", "Qwen3 8B"), ("general.size_label", "8B")],
            &[("general.file_type", 15), ("qwen3.context_length", 40960)],
            0,
        );
        let i = gguf_info(&p).unwrap();
        assert_eq!(i.name.as_deref(), Some("Qwen3 8B"));
        assert_eq!(i.arch.as_deref(), Some("qwen3"));
        assert_eq!(i.params.as_deref(), Some("8B"));
        assert_eq!(i.quant.as_deref(), Some("Q4_K_M"));
        assert_eq!(i.context, Some(40960));
        assert!(!i.companion);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn names_and_shards() {
        assert_eq!(shard_stem("model-00001-of-00004.safetensors"), "model");
        assert_eq!(shard_stem("Qwen3-32B-Q4_K_M-00002-of-00003.gguf"), "Qwen3-32B-Q4_K_M");
        assert_eq!(shard_stem("llama-3-8b.Q4_K_M.gguf"), "llama-3-8b.Q4_K_M");
        assert_eq!(params_label(8_030_261_248), "8B");
        assert_eq!(params_label(1_720_000_000), "1.7B");
        assert_eq!(params_label(32_760_000_000), "33B");
        assert_eq!(params_label(335_000_000), "335M");
        let m = Path::new("/h/.ollama/models/manifests");
        assert_eq!(ollama_name(m, &m.join("registry.ollama.ai/library/llama3.2/3b")).as_deref(), Some("llama3.2:3b"));
        assert_eq!(ollama_name(m, &m.join("registry.ollama.ai/library/qwen3/latest")).as_deref(), Some("qwen3"));
        assert_eq!(ollama_name(m, &m.join("hf.co/bartowski/X-GGUF/Q4_K_M")).as_deref(), Some("hf.co/bartowski/X-GGUF:Q4_K_M"));
        assert_eq!(display_name(Path::new("/m/sdxl/unet"), ""), "sdxl/unet");
        assert!(is_lm_studio(Path::new("/Users/me/.lmstudio/models")));
        assert!(!is_lm_studio(Path::new("/Users/me/models")));
    }

    /// A home folder with Ollama, LM Studio, a Hugging Face cache and loose files, including a
    /// copy of the LM Studio model in Downloads.
    #[test]
    fn finds_models_in_a_scan() {
        let home = dir("scan");
        let mb = 1 << 20;
        // Ollama: one model with a projector, one blob no manifest uses.
        let ol = home.join(".ollama/models");
        std::fs::create_dir_all(ol.join("blobs")).unwrap();
        std::fs::create_dir_all(ol.join("manifests/registry.ollama.ai/library/llava")).unwrap();
        write_gguf(&ol.join("blobs/sha256-aaaa"), &[("general.architecture", "llama"), ("general.name", "LLaVA")], &[("general.file_type", 2)], 40 * mb);
        write_gguf(&ol.join("blobs/sha256-bbbb"), &[("general.architecture", "clip")], &[], 20 * mb);
        write_gguf(&ol.join("blobs/sha256-cccc"), &[("general.architecture", "llama")], &[], 30 * mb);
        std::fs::write(ol.join("blobs/sha256-dddd"), "{{ .Prompt }}").unwrap();
        std::fs::write(
            ol.join("manifests/registry.ollama.ai/library/llava/7b"),
            r#"{"layers":[{"mediaType":"application/vnd.ollama.image.model","digest":"sha256:aaaa"},
                          {"mediaType":"application/vnd.ollama.image.projector","digest":"sha256:bbbb"},
                          {"mediaType":"application/vnd.ollama.image.template","digest":"sha256:dddd"}]}"#,
        )
        .unwrap();
        // LM Studio, and the same file copied to Downloads.
        let lm = home.join(".lmstudio/models/lmstudio-community/Qwen3-GGUF");
        std::fs::create_dir_all(&lm).unwrap();
        write_gguf(&lm.join("Qwen3-1.7B-Q8_0.gguf"), &[("general.architecture", "qwen3"), ("general.name", "Qwen3 1.7B")], &[("general.file_type", 7)], 25 * mb);
        std::fs::create_dir_all(home.join("Downloads")).unwrap();
        std::fs::copy(lm.join("Qwen3-1.7B-Q8_0.gguf"), home.join("Downloads/Qwen3-1.7B-Q8_0.gguf")).unwrap();
        // Hugging Face: two safetensors shards as blobs.
        let hf = home.join(".cache/huggingface/hub/models--google--gemma-3-1b-it/blobs");
        std::fs::create_dir_all(&hf).unwrap();
        for (i, name) in ["e1", "e2"].iter().enumerate() {
            let header = format!(r#"{{"w{i}":{{"dtype":"BF16","shape":[1000,1000],"data_offsets":[0,2000000]}}}}"#);
            let mut b = (header.len() as u64).to_le_bytes().to_vec();
            b.extend(header.as_bytes());
            let mut f = std::fs::File::create(hf.join(name)).unwrap();
            f.write_all(&b).unwrap();
            f.set_len(20 * mb as u64 + i as u64).unwrap();
        }
        // A loose checkpoint, and a small file that isn't a model.
        std::fs::create_dir_all(home.join("ComfyUI/models/checkpoints")).unwrap();
        std::fs::File::create(home.join("ComfyUI/models/checkpoints/sd_xl_base_1.0.safetensors")).unwrap().set_len(50 * mb as u64).unwrap();
        std::fs::write(home.join("ComfyUI/tiny.pt"), "x").unwrap();

        let progress = scan::Progress::default();
        let tree = scan::scan(&home, &scan::Options { metric: scan::Metric::Logical, ..Default::default() }, &std::sync::Arc::new(progress), &scan::new_live(&home));
        let models = find(&tree, &home, &Progress::default()).unwrap();
        let names: Vec<&str> = models.iter().map(|m| m.name.as_str()).collect();
        let get = |n: &str| models.iter().find(|m| m.name == n).unwrap_or_else(|| panic!("{n} not in {names:?}"));

        let llava = get("llava:7b");
        assert_eq!(llava.source, Source::Ollama);
        assert_eq!(llava.files.len(), 2);
        assert_eq!(llava.info.quant.as_deref(), Some("Q4_0"));
        assert!(llava.runnable());
        let orphan = models.iter().find(|m| m.orphan).unwrap();
        assert_eq!(orphan.files[0].path, ol.join("blobs/sha256-cccc"));
        assert!(!orphan.runnable());

        let qwen: Vec<&Model> = models.iter().filter(|m| m.name == "Qwen3 1.7B").collect();
        assert_eq!(qwen.len(), 2);
        assert!(qwen.iter().any(|m| m.source == Source::LmStudio));
        assert!(qwen.iter().all(|m| m.copies.len() == 1));
        assert_eq!(qwen.iter().map(|m| m.extra).sum::<u64>(), qwen[0].size);
        assert_eq!(qwen[0].details(), "GGUF · qwen3 · Q8_0");

        let gemma = get("google/gemma-3-1b-it");
        assert_eq!(gemma.format, Format::Safetensors);
        assert_eq!(gemma.files.len(), 2);
        assert_eq!(gemma.info.params.as_deref(), Some("2M"));
        assert_eq!(gemma.info.quant.as_deref(), Some("BF16"));
        assert!(!gemma.runnable());

        let sd = get("sd_xl_base_1.0");
        assert_eq!(sd.source, Source::Folder);
        assert!(!names.contains(&"tiny"));
        assert_eq!(models.len(), 6, "{names:?}");
        std::fs::remove_dir_all(&home).ok();
    }
}
