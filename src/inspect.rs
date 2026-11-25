//! Looking inside a model: what its own files say about it, what can be worked out from its tensor
//! directory, and what its publisher's notes claim. Only headers and small text files are read,
//! never the weights, and never a pickle-based checkpoint (.ckpt, .pt, .pth, .bin), since opening
//! one can run code.

use crate::gguf;
use crate::models::{Format, Model};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Where a fact comes from, shown beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// Read from the model file itself.
    Verified,
    /// Worked out from the model file, such as adding up its tensors.
    Calculated,
    /// Stated in the publisher's files beside the model (config.json, README), not checked.
    Documented,
}

impl How {
    pub fn label(self) -> &'static str {
        match self {
            How::Verified => "file",
            How::Calculated => "calculated",
            How::Documented => "publisher",
        }
    }

    pub fn explain(self) -> &'static str {
        match self {
            How::Verified => "Read from the model file itself.",
            How::Calculated => "Worked out by Filescape from the model file, for example by adding up its tensors.",
            How::Documented => "Stated in the publisher's files beside the model (config.json or README). Filescape hasn't checked it against the weights.",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fact {
    pub label: &'static str,
    pub value: String,
    pub how: How,
}

/// One tensor, in any format.
#[derive(Clone, Debug, PartialEq)]
pub struct Tensor {
    pub name: String,
    /// Number type, such as Q4_K, F16 or BF16.
    pub kind: String,
    /// Shape, outermost dimension first (as PyTorch and safetensors write it; GGUF stores it the
    /// other way round).
    pub shape: Vec<u64>,
    pub elements: u64,
    pub bytes: u64,
    /// Which of the model's files it is in.
    pub file: usize,
}

impl Tensor {
    pub fn shape_text(&self) -> String {
        if self.shape.is_empty() {
            return "scalar".into();
        }
        self.shape.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(" × ")
    }
}

/// How much of the model is stored in one number type.
#[derive(Clone, Debug, PartialEq)]
pub struct KindShare {
    pub kind: String,
    pub tensors: usize,
    pub elements: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Inspection {
    pub facts: Vec<Fact>,
    pub tensors: Vec<Tensor>,
    /// Largest share first.
    pub kinds: Vec<KindShare>,
    /// Every metadata entry, as (key, value text), in file order.
    pub meta: Vec<(String, String)>,
    /// Problems met while reading, for the user.
    pub problems: Vec<String>,
    /// Why the model's insides couldn't be read at all, if they couldn't.
    pub unread: Option<String>,
}

/// Reads everything that can be read safely about a model. Slow only for very large tensor
/// directories; run it off the UI thread.
pub fn inspect(m: &Model) -> Inspection {
    let mut out = Inspection::default();
    if m.files.iter().all(|f| f.not_local) {
        out.unread = Some("The model's files are only in the cloud. Reading them would download them, so Filescape hasn't.".into());
        return out;
    }
    match m.format {
        Format::Gguf => gguf_model(m, &mut out),
        Format::Safetensors => safetensors_model(m, &mut out),
        Format::PyTorch => {
            out.unread = Some(
                "PyTorch checkpoints (.ckpt, .pt, .pth, .bin) are saved with Python's pickle, which can run code when the file \
                 is opened. Filescape never opens them, so only their names and sizes are shown. The same model in \
                 safetensors or GGUF format can be inspected."
                    .into(),
            );
        }
        Format::Onnx | Format::CoreMl | Format::Other => {
            out.unread = Some(format!("Filescape doesn't look inside {} models yet. Their names and sizes are shown.", m.format.label()));
        }
    }
    if m.files.iter().any(|f| f.not_local) && out.unread.is_none() {
        out.problems.push("Some of the model's files are only in the cloud and weren't read, so totals leave them out.".into());
    }
    // What the publisher says, beside the model.
    if let Some(dir) = publisher_dir(m) {
        documented(&dir, &mut out);
    }
    summarise(&mut out);
    out
}

// ---------- GGUF ----------

fn gguf_model(m: &Model, out: &mut Inspection) {
    let mut first: Option<gguf::Header> = None;
    for (i, f) in m.files.iter().enumerate() {
        if f.not_local {
            continue;
        }
        match gguf::read(&f.path, true) {
            Ok(h) => {
                for t in &h.tensors {
                    out.tensors.push(Tensor {
                        name: t.name.clone(),
                        kind: gguf::type_name(t.kind).to_string(),
                        shape: t.dims.iter().rev().copied().collect(),
                        elements: t.elements(),
                        bytes: t.bytes,
                        file: i,
                    });
                }
                if first.is_none() {
                    first = Some(h);
                }
            }
            Err(e) => out.problems.push(format!("{}: {e}", file_name(&f.path))),
        }
    }
    let Some(h) = first else {
        if out.problems.is_empty() {
            out.unread = Some("None of the model's files could be read.".into());
        }
        return;
    };
    out.meta = h.meta.iter().map(|(k, v)| (k.clone(), v.show())).collect();
    let v = How::Verified;
    let mut fact = |label, value: Option<String>| {
        if let Some(value) = value.filter(|s| !s.trim().is_empty()) {
            out.facts.push(Fact { label, value, how: v });
        }
    };
    let arch = h.str("general.architecture").map(str::to_string);
    let num = |key: &str| h.arch_u64(key).map(group);
    // A per-layer array (some models vary heads by layer) shows as its range.
    let heads = |key: &str| -> Option<String> {
        let a = arch.as_deref()?;
        match h.get(&format!("{a}.{key}"))? {
            gguf::Value::Array(xs) => {
                let ns: Vec<u64> = xs.iter().filter_map(gguf::Value::as_u64).collect();
                let (lo, hi) = (ns.iter().min()?, ns.iter().max()?);
                Some(if lo == hi { lo.to_string() } else { format!("{lo} to {hi}, by layer") })
            }
            x => x.as_u64().map(group),
        }
    };
    fact("Name", h.str("general.name").map(str::to_string));
    fact("Made by", h.str("general.organization").map(str::to_string));
    fact("Architecture", arch.clone());
    fact("Size label", h.str("general.size_label").map(str::to_string));
    fact("Fine-tuned as", h.str("general.finetune").map(str::to_string));
    fact("Precision", h.u64("general.file_type").and_then(crate::models::ftype_name).map(str::to_string));
    fact("Layers", num("block_count"));
    fact("Hidden size", num("embedding_length"));
    fact("Feed-forward size", num("feed_forward_length"));
    fact("Attention heads", heads("attention.head_count"));
    fact("Key/value heads", heads("attention.head_count_kv"));
    fact("Experts", num("expert_count").map(|n| match h.arch_u64("expert_used_count") {
        Some(u) => format!("{n}, {u} used per token"),
        None => n,
    }));
    fact("Context", num("context_length").map(|n| format!("{n} tokens")));
    let vocab = h.get("tokenizer.ggml.tokens").and_then(gguf::Value::len).or_else(|| h.arch_u64("vocab_size"));
    fact("Vocabulary", vocab.map(|n| format!("{} tokens", group(n))));
    fact("Tokenizer", h.str("tokenizer.ggml.model").map(str::to_string));
    fact("Chat template", h.get("tokenizer.chat_template").map(|_| "Included".to_string()));
    fact("Licence", licence_of(&h));
    fact("GGUF version", Some(h.version.to_string()));
    if m.files.len() > 1 {
        fact("Parts", Some(format!("{} files", m.files.len())));
    }
}

fn licence_of(h: &gguf::Header) -> Option<String> {
    let id = h.str("general.license").map(str::to_string);
    let name = h.str("general.license.name").map(str::to_string);
    match (id, name) {
        (Some(i), Some(n)) if i != n => Some(format!("{n} ({i})")),
        (i, n) => i.or(n),
    }
}

// ---------- safetensors ----------

fn safetensors_model(m: &Model, out: &mut Inspection) {
    let mut read_any = false;
    for (i, f) in m.files.iter().enumerate() {
        if f.not_local || !(file_name(&f.path).to_ascii_lowercase().ends_with(".safetensors") || is_blob(&f.path)) {
            continue;
        }
        match safetensors_header(&f.path) {
            Ok((tensors, meta)) => {
                read_any = true;
                for (name, kind, shape, bytes) in tensors {
                    let elements = shape.iter().product();
                    out.tensors.push(Tensor { name, kind, shape, elements, bytes, file: i });
                }
                for (k, v) in meta {
                    if !out.meta.iter().any(|(k2, _)| *k2 == k) {
                        out.meta.push((k, v));
                    }
                }
            }
            // A Hugging Face blob may be another kind of file (a .bin); that's not a problem.
            Err(_) if is_blob(&f.path) => {}
            Err(e) => out.problems.push(format!("{}: {e}", file_name(&f.path))),
        }
    }
    if !read_any && out.problems.is_empty() {
        out.unread = Some("None of the model's safetensors files could be read.".into());
    }
    if m.files.len() > 1 {
        out.facts.push(Fact { label: "Parts", value: format!("{} files", m.files.len()), how: How::Verified });
    }
}

fn is_blob(p: &Path) -> bool {
    p.parent().and_then(|d| d.file_name()).is_some_and(|n| n == "blobs")
}

type StEntry = (String, String, Vec<u64>, u64);

/// A safetensors header: tensors as (name, dtype, shape, bytes) and the `__metadata__` strings.
fn safetensors_header(path: &Path) -> Result<(Vec<StEntry>, Vec<(String, String)>), String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut len = [0u8; 8];
    f.read_exact(&mut len).map_err(|_| "too short to be a safetensors file".to_string())?;
    let len = u64::from_le_bytes(len);
    if len < 2 || len > 100 << 20 {
        return Err("not a safetensors file, or a damaged one".into());
    }
    let mut b = vec![0u8; len as usize];
    f.read_exact(&mut b).map_err(|_| "the header is cut short".to_string())?;
    let json: serde_json::Value = serde_json::from_slice(&b).map_err(|_| "the header isn't valid".to_string())?;
    let obj = json.as_object().ok_or("the header isn't valid")?;
    let mut tensors = Vec::new();
    let mut meta = Vec::new();
    for (k, t) in obj {
        if k == "__metadata__" {
            if let Some(m) = t.as_object() {
                for (mk, mv) in m {
                    meta.push((mk.clone(), mv.as_str().map(str::to_string).unwrap_or_else(|| mv.to_string())));
                }
            }
            continue;
        }
        let kind = t["dtype"].as_str().unwrap_or("?").to_string();
        let shape: Vec<u64> = t["shape"].as_array().map(|a| a.iter().filter_map(|d| d.as_u64()).collect()).unwrap_or_default();
        let bytes = match t["data_offsets"].as_array().map(|a| a.iter().filter_map(|x| x.as_u64()).collect::<Vec<_>>()) {
            Some(o) if o.len() == 2 && o[1] >= o[0] => o[1] - o[0],
            _ => shape.iter().product::<u64>() * dtype_bytes(&kind).unwrap_or(0),
        };
        tensors.push((k.clone(), kind, shape, bytes));
    }
    Ok((tensors, meta))
}

fn dtype_bytes(d: &str) -> Option<u64> {
    Some(match d {
        "F64" | "I64" | "U64" => 8,
        "F32" | "I32" | "U32" => 4,
        "F16" | "BF16" | "I16" | "U16" => 2,
        "I8" | "U8" | "BOOL" | "F8_E4M3" | "F8_E5M2" => 1,
        _ => return None,
    })
}

// ---------- the publisher's files ----------

/// The folder holding config.json and README.md for this model: a Hugging Face repository's
/// newest snapshot, or the model file's own folder.
fn publisher_dir(m: &Model) -> Option<PathBuf> {
    let first = &m.files.first()?.path;
    let dir = first.parent()?;
    if is_blob(first) {
        let snaps = dir.parent()?.join("snapshots");
        let newest = std::fs::read_dir(&snaps)
            .ok()?
            .flatten()
            .filter(|e| e.path().is_dir())
            .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())?;
        return Some(newest.path());
    }
    // Ollama keeps its own files, not a publisher's.
    (m.source != crate::models::Source::Ollama).then(|| dir.to_path_buf())
}

fn documented(dir: &Path, out: &mut Inspection) {
    let d = How::Documented;
    let has = |out: &Inspection, label: &str| out.facts.iter().any(|f| f.label == label);
    if let Some(c) = small_text(&dir.join("config.json")).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
        // Multimodal configs keep the language model's numbers under text_config.
        let text = c.get("text_config").filter(|t| t.is_object()).unwrap_or(&c);
        let num = |k: &str| text.get(k).or_else(|| c.get(k)).and_then(|v| v.as_u64()).map(group);
        let s = |k: &str| c.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let arch = s("model_type").or_else(|| c["architectures"].get(0).and_then(|v| v.as_str()).map(str::to_string));
        let rows: [(&'static str, Option<String>); 8] = [
            ("Architecture", arch),
            ("Layers", num("num_hidden_layers")),
            ("Hidden size", num("hidden_size")),
            ("Feed-forward size", num("intermediate_size")),
            ("Attention heads", num("num_attention_heads")),
            ("Key/value heads", num("num_key_value_heads")),
            ("Context", num("max_position_embeddings").map(|n| format!("{n} tokens"))),
            ("Vocabulary", num("vocab_size").map(|n| format!("{n} tokens"))),
        ];
        for (label, value) in rows {
            if let Some(value) = value
                && !has(out, label)
            {
                out.facts.push(Fact { label, value, how: d });
            }
        }
        if let Some(t) = s("torch_dtype").or_else(|| s("dtype"))
            && !has(out, "Precision")
        {
            out.facts.push(Fact { label: "Precision", value: t, how: d });
        }
    }
    if !has(out, "Licence")
        && let Some(l) = small_text(&dir.join("README.md")).and_then(|t| front_matter(&t, "license"))
    {
        out.facts.push(Fact { label: "Licence", value: l, how: d });
    }
}

/// A text file of at most 1 MB.
fn small_text(p: &Path) -> Option<String> {
    let f = std::fs::File::open(p).ok()?;
    if f.metadata().ok()?.len() > 1 << 20 {
        return None;
    }
    let mut s = String::new();
    f.take(1 << 20).read_to_string(&mut s).ok()?;
    Some(s)
}

/// A value from a README's YAML front matter (between `---` lines), such as `license: mit`.
fn front_matter(text: &str, key: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for l in lines {
        if l.trim() == "---" {
            break;
        }
        if let Some(v) = l.strip_prefix(key).and_then(|r| r.strip_prefix(':')) {
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

// ---------- totals ----------

fn summarise(out: &mut Inspection) {
    if out.tensors.is_empty() {
        return;
    }
    let elements: u64 = out.tensors.iter().map(|t| t.elements).sum();
    let bytes: u64 = out.tensors.iter().map(|t| t.bytes).sum();
    let mut kinds: BTreeMap<&str, KindShare> = BTreeMap::new();
    for t in &out.tensors {
        let k = kinds.entry(&t.kind).or_insert_with(|| KindShare { kind: t.kind.clone(), tensors: 0, elements: 0, bytes: 0 });
        k.tensors += 1;
        k.elements += t.elements;
        k.bytes += t.bytes;
    }
    let mut kinds: Vec<KindShare> = kinds.into_values().collect();
    kinds.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.kind.cmp(&b.kind)));
    let c = How::Calculated;
    let at = out.facts.iter().position(|f| f.label == "Precision").map_or(out.facts.len(), |i| i + 1);
    let mut add = vec![
        Fact { label: "Parameters", value: format!("{} ({})", crate::models::params_label(elements), group(elements)), how: c },
        Fact { label: "Tensors", value: group(out.tensors.len() as u64), how: c },
        Fact { label: "Weights size", value: crate::scan::human(bytes), how: c },
    ];
    if elements > 0 {
        add.push(Fact { label: "Bits per weight", value: format!("{:.2} on average", bytes as f64 * 8.0 / elements as f64), how: c });
    }
    if kinds.len() > 1 {
        let main = &kinds[0];
        let pct = main.bytes as f64 * 100.0 / bytes.max(1) as f64;
        add.push(Fact { label: "Number types", value: format!("{} kinds, mostly {} ({pct:.0}% of the bytes)", kinds.len(), main.kind), how: c });
    }
    out.facts.splice(at..at, add);
    out.kinds = kinds;
}

/// 1234567 -> "1,234,567".
pub fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Compares names with their numbers in order: `blk.2` before `blk.10`.
pub fn natural(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    loop {
        match (a.first(), b.first()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let na = a.iter().take_while(|c| c.is_ascii_digit()).count();
                let nb = b.iter().take_while(|c| c.is_ascii_digit()).count();
                let (da, db) = (trim_zeros(&a[..na]), trim_zeros(&b[..nb]));
                let o = da.len().cmp(&db.len()).then_with(|| da.cmp(db));
                if o != Ordering::Equal {
                    return o;
                }
                a = &a[na..];
                b = &b[nb..];
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(y);
                }
                a = &a[1..];
                b = &b[1..];
            }
        }
    }
}

fn trim_zeros(d: &[u8]) -> &[u8] {
    let i = d.iter().position(|&c| c != b'0').unwrap_or(d.len());
    &d[i..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{File, Info, Source};

    fn model(format: Format, source: Source, paths: &[PathBuf]) -> Model {
        let files = paths.iter().map(|p| File { path: p.clone(), size: std::fs::metadata(p).map(|m| m.len()).unwrap_or(0), not_local: false }).collect();
        Model { name: "test".into(), source, format, info: Info::default(), files, size: 0, copies: vec![], extra: 0, orphan: false }
    }

    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Tmp {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            let d = std::env::temp_dir().join(format!("filescape-inspect-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&d).unwrap();
            Tmp(d)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fact<'a>(i: &'a Inspection, label: &str) -> Option<&'a Fact> {
        i.facts.iter().find(|f| f.label == label)
    }

    #[test]
    fn gguf_facts_and_tensors() {
        let dir = Tmp::new();
        let p = dir.path().join("m.gguf");
        gguf::write_test(
            &p,
            &[("general.architecture", "llama"), ("general.name", "Tiny"), ("general.license", "apache-2.0")],
            &[("llama.block_count", 2), ("llama.context_length", 4096), ("general.file_type", 15)],
            &[("token_embd.weight", &[64, 100], 1), ("blk.10.ffn_up.weight", &[64, 128], 12), ("blk.2.ffn_up.weight", &[64, 128], 12)],
            0,
        );
        let i = inspect(&model(Format::Gguf, Source::Folder, &[p]));
        assert_eq!(i.unread, None, "{:?}", i.problems);
        assert_eq!(fact(&i, "Layers").unwrap().value, "2");
        assert_eq!(fact(&i, "Layers").unwrap().how, How::Verified);
        assert_eq!(fact(&i, "Licence").unwrap().value, "apache-2.0");
        assert_eq!(fact(&i, "Precision").unwrap().value, "Q4_K_M");
        let params = fact(&i, "Parameters").unwrap();
        assert_eq!(params.how, How::Calculated);
        assert!(params.value.contains("22,784") || params.value.contains("22784"), "{}", params.value);
        assert_eq!(i.tensors.len(), 3);
        // Shown outermost first.
        assert_eq!(i.tensors[0].shape, vec![100, 64]);
        assert_eq!(i.kinds[0].kind, "F16");
        assert!(i.meta.iter().any(|(k, v)| k == "general.name" && v == "Tiny"));
        let mut names: Vec<&str> = i.tensors.iter().map(|t| t.name.as_str()).collect();
        names.sort_by(|a, b| natural(a, b));
        assert_eq!(names, ["blk.2.ffn_up.weight", "blk.10.ffn_up.weight", "token_embd.weight"]);
    }

    #[test]
    fn safetensors_with_publisher_files() {
        let dir = Tmp::new();
        let p = dir.path().join("model.safetensors");
        let header = br#"{"__metadata__":{"format":"pt"},"a.weight":{"dtype":"BF16","shape":[4,8],"data_offsets":[0,64]},"b.bias":{"dtype":"F32","shape":[8],"data_offsets":[64,96]}}"#;
        let mut b = (header.len() as u64).to_le_bytes().to_vec();
        b.extend(header);
        b.extend([0u8; 96]);
        std::fs::write(&p, b).unwrap();
        std::fs::write(dir.path().join("config.json"), r#"{"model_type":"qwen3","num_hidden_layers":28,"vocab_size":151936,"torch_dtype":"bfloat16"}"#).unwrap();
        std::fs::write(dir.path().join("README.md"), "---\nlicense: mit\ntags: [x]\n---\n# Hi\n").unwrap();
        let i = inspect(&model(Format::Safetensors, Source::Folder, &[p]));
        assert_eq!(i.tensors.len(), 2);
        assert_eq!(fact(&i, "Tensors").unwrap().value, "2");
        assert_eq!(fact(&i, "Layers").unwrap().how, How::Documented);
        assert_eq!(fact(&i, "Layers").unwrap().value, "28");
        assert_eq!(fact(&i, "Licence").unwrap().value, "mit");
        assert_eq!(fact(&i, "Weights size").unwrap().value, crate::scan::human(96));
        assert_eq!(i.meta, vec![("format".to_string(), "pt".to_string())]);
    }

    #[test]
    fn pickles_are_never_opened() {
        let dir = Tmp::new();
        let p = dir.path().join("model.ckpt");
        std::fs::write(&p, b"not read").unwrap();
        let i = inspect(&model(Format::PyTorch, Source::Folder, &[p]));
        assert!(i.unread.unwrap().contains("never opens"));
        assert!(i.tensors.is_empty());
    }
}
