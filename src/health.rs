//! Health checks for a model: is every part there, is any file cut short or damaged, is it in a
//! format that's safe to open, and does it match what it says it is. They use only what
//! `inspect` already read, so they're instant; checking a file's checksum reads it all and is
//! started separately (see [`checksum_of`]).

use crate::inspect::Inspection;
use crate::models::{Format, Model};
use crate::scan::human;
use std::collections::HashSet;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Something is wrong: the model is unlikely to load or work.
    Problem,
    /// Worth knowing about, but the model probably works.
    Warn,
    /// A good sign.
    Good,
    /// Neither good nor bad.
    Info,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub level: Level,
    pub title: String,
    pub detail: String,
}

fn check(level: Level, title: impl Into<String>, detail: impl Into<String>) -> Check {
    Check { level, title: title.into(), detail: detail.into() }
}

/// Every check for a model, problems first.
pub fn checks(m: &Model, d: &Inspection) -> Vec<Check> {
    let mut out = Vec::new();
    safety(m, &mut out);
    if m.files.iter().any(|f| f.not_local) {
        out.push(check(Level::Info, "Some files are only in the cloud", "They weren't read, so they weren't checked. Download them to check them."));
    }
    parts(m, d, &mut out);
    if d.unread.is_none() && !d.layouts.is_empty() {
        files(m, d, &mut out);
        tensors(d, &mut out);
        claims(m, d, &mut out);
    }
    for p in &d.problems {
        out.push(check(Level::Problem, "A file couldn't be read", p.clone()));
    }
    out.sort_by_key(|c| c.level);
    out
}

fn safety(m: &Model, out: &mut Vec<Check>) {
    match m.format {
        Format::Gguf | Format::Safetensors => out.push(check(
            Level::Good,
            "Safe format",
            format!("{} files hold only numbers and descriptions, with no code that runs when they're opened.", m.format.label()),
        )),
        Format::PyTorch => out.push(check(
            Level::Warn,
            "Can contain code",
            "PyTorch checkpoints are saved with Python's pickle, which can run code when the file is opened. \
             Only open them in apps you trust, from sources you trust; prefer a safetensors or GGUF version of the model.",
        )),
        _ => {}
    }
}

/// Numbered parts that are missing (`-00002-of-00004`, or a GGUF's own `split.count`).
fn parts(m: &Model, d: &Inspection, out: &mut Vec<Check>) {
    let mut want: Option<u64> = None;
    let mut have: HashSet<u64> = HashSet::new();
    for f in &m.files {
        let name = f.path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if let Some((k, n)) = part_numbers(&name) {
            have.insert(k);
            want = Some(want.map_or(n, |w| w.max(n)));
        }
    }
    if let Some(n) = d.layouts.iter().filter_map(|l| l.split_count).max() {
        want = Some(want.map_or(n, |w| w.max(n)));
        if have.is_empty() {
            have.extend(1..=m.files.len() as u64);
        }
    }
    let Some(n) = want else { return };
    let missing: Vec<u64> = (1..=n).filter(|k| !have.contains(k)).collect();
    if missing.is_empty() {
        out.push(check(Level::Good, format!("All {n} parts are here"), "The model is split into several files, and none is missing."));
    } else {
        let list = missing.iter().map(|k| k.to_string()).collect::<Vec<_>>().join(", ");
        out.push(check(
            Level::Problem,
            format!("{} of {n} parts missing", missing.len()),
            format!("Part {list} isn't here, so the model can't be loaded. Download the missing parts again, into the same folder."),
        ));
    }
}

/// `model-00002-of-00004.gguf` → (2, 4).
fn part_numbers(name: &str) -> Option<(u64, u64)> {
    let i = name.find("-of-")?;
    let before = &name[..i];
    let k = before.rsplit('-').next()?.parse().ok()?;
    let n = name[i + 4..].split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    (k >= 1 && k <= n).then_some((k, n))
}

fn files(m: &Model, d: &Inspection, out: &mut Vec<Check>) {
    let mut whole = true;
    for l in &d.layouts {
        let name = m.files.get(l.file).map(|f| f.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).unwrap_or_default();
        let ts: Vec<_> = d.tensors.iter().filter(|t| t.file == l.file).collect();
        let end = ts.iter().map(|t| t.offset + t.bytes).max().unwrap_or(l.data_start);
        if l.size < end {
            whole = false;
            out.push(check(
                Level::Problem,
                "A file is cut short",
                format!(
                    "{name} is {} but its tensors need {}: {} is missing from the end. The download probably stopped early; download it again.",
                    human(l.size),
                    human(end),
                    human(end - l.size)
                ),
            ));
        } else if l.size > end + l.alignment.unwrap_or(0) + 64 {
            out.push(check(
                Level::Info,
                "Extra data at the end of a file",
                format!("{name} has {} after its last tensor. It's usually harmless, but it isn't part of the model.", human(l.size - end)),
            ));
        }
        // Tensors must not overlap, and GGUF data must be aligned.
        let mut spans: Vec<(u64, u64, &str)> = ts.iter().filter(|t| t.bytes > 0).map(|t| (t.offset, t.offset + t.bytes, t.name.as_str())).collect();
        spans.sort();
        if let Some(w) = spans.windows(2).find(|w| w[1].0 < w[0].1) {
            whole = false;
            out.push(check(Level::Problem, "Tensors overlap", format!("In {name}, {} and {} share the same bytes, so the file is damaged.", w[0].2, w[1].2)));
        }
        if let Some(a) = l.alignment
            && let Some(t) = ts.iter().find(|t| (t.offset - l.data_start.min(t.offset)) % a != 0)
        {
            whole = false;
            out.push(check(Level::Problem, "A tensor is out of place", format!("In {name}, {} doesn't start on a {a}-byte boundary, as GGUF requires.", t.name)));
        }
        if ts.iter().any(|t| t.offset < l.data_start) {
            whole = false;
            out.push(check(Level::Problem, "A tensor points inside the header", format!("{name} is damaged.")));
        }
    }
    if whole {
        let n = d.layouts.len();
        out.push(check(
            Level::Good,
            if n == 1 { "The file is complete".to_string() } else { format!("All {n} files are complete") },
            "Every tensor fits inside its file, none overlap, and nothing is missing from the end. This checks how the file is laid out, not every number in it.",
        ));
    }
}

fn tensors(d: &Inspection, out: &mut Vec<Check>) {
    let mut seen = HashSet::new();
    if let Some(t) = d.tensors.iter().find(|t| !seen.insert(t.name.as_str())) {
        out.push(check(Level::Problem, "Two tensors have the same name", format!("{} appears twice, so apps can't tell which to use.", t.name)));
    }
    let unknown: Vec<&str> = d.tensors.iter().filter(|t| t.kind == "unknown" || t.kind == "?").map(|t| t.name.as_str()).collect();
    if !unknown.is_empty() {
        out.push(check(
            Level::Warn,
            format!("{} tensor{} of a type Filescape doesn't know", unknown.len(), if unknown.len() == 1 { "" } else { "s" }),
            format!("Such as {}. It may be a newer format: the app that runs it may need updating.", unknown[0]),
        ));
    }
    if let Some(t) = d.tensors.iter().find(|t| t.shape.contains(&0)) {
        out.push(check(Level::Warn, "An empty tensor", format!("{} has a dimension of 0.", t.name)));
    }
}

/// Does it match what it says it is?
fn claims(m: &Model, d: &Inspection, out: &mut Vec<Check>) {
    let counted: u64 = d.tensors.iter().map(|t| t.elements).sum();
    let label = d.facts.iter().find(|f| f.label == "Size label").map(|f| f.value.clone()).or_else(|| m.info.params.clone());
    if let Some(claim) = label.as_deref().and_then(parse_size_label)
        && counted > 0
        && !m.info.companion
    {
        let ratio = counted as f64 / claim;
        if !(0.7..=1.45).contains(&ratio) && !label.as_deref().is_some_and(|l| l.contains('x')) {
            out.push(check(
                Level::Warn,
                "Size doesn't match its label",
                format!(
                    "It says {} but has {} parameters. The label may be wrong, or parts of the model may be missing or added.",
                    label.unwrap_or_default(),
                    crate::models::params_label(counted)
                ),
            ));
        }
    }
    if m.format == Format::Gguf && !m.info.companion {
        let has = |label: &str| d.facts.iter().any(|f| f.label == label);
        if !has("Vocabulary") {
            out.push(check(Level::Warn, "No vocabulary", "The file has no tokenizer, so apps can't turn text into tokens for it."));
        }
        if !has("Chat template") {
            out.push(check(
                Level::Info,
                "No chat template",
                "Chat apps won't know how this model expects a conversation to be laid out, and may need it set by hand. Base (non-chat) models often have none.",
            ));
        }
    }
}

/// "8B" → 8e9, "1.7B" → 1.7e9, "135M" → 1.35e8. Labels like "8x7B" aren't simple counts.
fn parse_size_label(s: &str) -> Option<f64> {
    let s = s.trim();
    let (num, mult) = match s.chars().last()?.to_ascii_uppercase() {
        'B' => (&s[..s.len() - 1], 1e9),
        'M' => (&s[..s.len() - 1], 1e6),
        'T' => (&s[..s.len() - 1], 1e12),
        'K' => (&s[..s.len() - 1], 1e3),
        _ => return None,
    };
    num.parse::<f64>().ok().filter(|n| *n > 0.0).map(|n| n * mult)
}

/// The SHA-256 a file is named by, when its store names it that way: Ollama's `sha256-<hex>`
/// and Hugging Face's large-file blobs (64 hex characters).
pub fn checksum_of(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let hex = name.strip_prefix("sha256-").or_else(|| name.strip_prefix("sha256:")).unwrap_or(name);
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hex.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::{Layout, Tensor};
    use crate::models::{File, Info, Source};
    use std::path::PathBuf;

    fn model(format: Format, names: &[&str]) -> Model {
        let files = names.iter().map(|n| File { path: PathBuf::from(format!("/m/{n}")), size: 0, not_local: false }).collect();
        Model { name: "m".into(), source: Source::Folder, format, info: Info::default(), files, size: 0, copies: vec![], extra: 0, same_as: Vec::new(), orphan: false }
    }

    fn tensor(name: &str, offset: u64, bytes: u64) -> Tensor {
        Tensor { name: name.into(), kind: "F16".into(), shape: vec![bytes / 2], elements: bytes / 2, bytes, file: 0, offset }
    }

    fn titles(c: &[Check], level: Level) -> Vec<String> {
        c.iter().filter(|c| c.level == level).map(|c| c.title.clone()).collect()
    }

    #[test]
    fn missing_parts_and_cut_short_files() {
        let m = model(Format::Safetensors, &["model-00001-of-00003.safetensors", "model-00003-of-00003.safetensors"]);
        let d = Inspection {
            tensors: vec![tensor("a", 100, 64), tensor("b", 164, 64)],
            layouts: vec![Layout { file: 0, size: 200, data_start: 100, alignment: None, split_count: None }],
            ..Default::default()
        };
        let c = checks(&m, &d);
        let problems = titles(&c, Level::Problem);
        assert!(problems.contains(&"1 of 3 parts missing".to_string()), "{c:?}");
        assert!(problems.contains(&"A file is cut short".to_string()), "{c:?}");
        assert!(c.iter().any(|c| c.detail.contains("Part 2 ")));
        // Problems come first.
        assert_eq!(c[0].level, Level::Problem);
    }

    #[test]
    fn a_sound_file_passes() {
        let m = model(Format::Gguf, &["m.gguf"]);
        let d = Inspection {
            tensors: vec![tensor("a", 128, 64), tensor("b", 192, 64)],
            layouts: vec![Layout { file: 0, size: 256, data_start: 128, alignment: Some(32), split_count: None }],
            ..Default::default()
        };
        let c = checks(&m, &d);
        assert!(titles(&c, Level::Problem).is_empty(), "{c:?}");
        assert!(titles(&c, Level::Good).contains(&"The file is complete".to_string()));
        assert!(titles(&c, Level::Good).contains(&"Safe format".to_string()));
    }

    #[test]
    fn overlaps_misalignment_and_labels() {
        let mut m = model(Format::Gguf, &["m.gguf"]);
        m.info.params = Some("8B".into());
        let d = Inspection {
            tensors: vec![tensor("a", 128, 64), tensor("b", 160, 64), tensor("a", 230, 2)],
            layouts: vec![Layout { file: 0, size: 300, data_start: 128, alignment: Some(32), split_count: None }],
            ..Default::default()
        };
        let p = titles(&checks(&m, &d), Level::Problem);
        assert!(p.contains(&"Tensors overlap".to_string()), "{p:?}");
        assert!(p.contains(&"A tensor is out of place".to_string()), "{p:?}");
        assert!(p.contains(&"Two tensors have the same name".to_string()), "{p:?}");
        assert!(titles(&checks(&m, &d), Level::Warn).contains(&"Size doesn't match its label".to_string()));
        assert_eq!(parse_size_label("1.7B"), Some(1.7e9));
        assert_eq!(parse_size_label("8x7B"), None);
    }

    #[test]
    fn checksum_names() {
        let h = "a".repeat(64);
        assert_eq!(checksum_of(Path::new(&format!("/o/blobs/sha256-{h}"))), Some(h.clone()));
        assert_eq!(checksum_of(Path::new(&format!("/hf/blobs/{h}"))), Some(h));
        assert_eq!(checksum_of(Path::new("/x/model.gguf")), None);
        assert_eq!(part_numbers("model-00002-of-00004.safetensors"), Some((2, 4)));
        assert_eq!(part_numbers("qwen-q4_k_m-00001-of-00002.gguf"), Some((1, 2)));
    }

    #[test]
    fn pickles_warn() {
        let m = model(Format::PyTorch, &["x.ckpt"]);
        assert_eq!(titles(&checks(&m, &Inspection::default()), Level::Warn), ["Can contain code"]);
    }
}
