//! A model's vocabulary: the tokens it reads and writes, read from a GGUF file's metadata or a
//! publisher's `tokenizer.json`, without running the model. Used by the Tokens tab of the model
//! page. Nothing here turns text into tokens: that needs the model's own tokenizer (Ask AI).

use std::path::Path;

use crate::models::{Format, Model};

/// What a token is for, as GGUF numbers them (`tokenizer.ggml.token_type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Normal,
    Unknown,
    /// A special token the model uses to mark turns, ends and so on (`<|im_end|>`).
    Control,
    /// Added by the publisher, read as one piece.
    Added,
    Unused,
    /// One raw byte (`<0x0A>`), for text no other token covers.
    Byte,
}

impl Kind {
    fn from_gguf(n: u64) -> Kind {
        match n {
            2 => Kind::Unknown,
            3 => Kind::Control,
            4 => Kind::Added,
            5 => Kind::Unused,
            6 => Kind::Byte,
            _ => Kind::Normal,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Normal => "normal",
            Kind::Unknown => "unknown",
            Kind::Control => "special",
            Kind::Added => "added",
            Kind::Unused => "unused",
            Kind::Byte => "byte",
        }
    }
}

/// The writing system a token belongs to, judged by its first letter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Script {
    Latin,
    Cyrillic,
    Greek,
    Arabic,
    Hebrew,
    Indic,
    Thai,
    /// Chinese characters and Japanese kana.
    Cjk,
    Korean,
    OtherLetters,
    Numbers,
    /// Punctuation, symbols and emoji: much of the code a model reads.
    Symbols,
    /// Only spaces, tabs or newlines (indentation runs in code models).
    Spaces,
    /// Part of a character: a byte-level token that only makes sense joined to others.
    Partial,
    Special,
}

impl Script {
    pub const ALL: [Script; 15] = [
        Script::Latin,
        Script::Cjk,
        Script::Korean,
        Script::Cyrillic,
        Script::Arabic,
        Script::Indic,
        Script::Greek,
        Script::Hebrew,
        Script::Thai,
        Script::OtherLetters,
        Script::Numbers,
        Script::Symbols,
        Script::Spaces,
        Script::Partial,
        Script::Special,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Script::Latin => "Latin (English, European)",
            Script::Cyrillic => "Cyrillic",
            Script::Greek => "Greek",
            Script::Arabic => "Arabic",
            Script::Hebrew => "Hebrew",
            Script::Indic => "Indic (Hindi, Bengali, Tamil…)",
            Script::Thai => "Thai",
            Script::Cjk => "Chinese and Japanese",
            Script::Korean => "Korean",
            Script::OtherLetters => "Other scripts",
            Script::Numbers => "Numbers",
            Script::Symbols => "Punctuation, symbols, emoji",
            Script::Spaces => "Spaces and indentation",
            Script::Partial => "Parts of characters",
            Script::Special => "Special and unused",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Script::Latin => "Latin",
            Script::Cyrillic => "Cyrillic",
            Script::Greek => "Greek",
            Script::Arabic => "Arabic",
            Script::Hebrew => "Hebrew",
            Script::Indic => "Indic",
            Script::Thai => "Thai",
            Script::Cjk => "CJK",
            Script::Korean => "Korean",
            Script::OtherLetters => "other",
            Script::Numbers => "number",
            Script::Symbols => "symbol",
            Script::Spaces => "space",
            Script::Partial => "partial",
            Script::Special => "special",
        }
    }

    /// Whether the app's built-in fonts have this script's letters (they have Latin, Greek and
    /// Cyrillic, plus symbols and emoji); others are shown by their code points.
    pub fn drawable(self) -> bool {
        !matches!(self, Script::Cjk | Script::Korean | Script::Arabic | Script::Hebrew | Script::Indic | Script::Thai | Script::OtherLetters)
    }

    fn of_char(c: char) -> Option<Script> {
        let u = c as u32;
        Some(match u {
            0x30..=0x39 => Script::Numbers,
            _ if c.is_ascii_alphabetic() => Script::Latin,
            0xC0..=0x24F | 0x1E00..=0x1EFF => Script::Latin,
            0x370..=0x3FF | 0x1F00..=0x1FFF => Script::Greek,
            0x400..=0x52F => Script::Cyrillic,
            0x590..=0x5FF => Script::Hebrew,
            0x600..=0x6FF | 0x750..=0x77F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Script::Arabic,
            0x900..=0xDFF => Script::Indic,
            0xE00..=0xE7F => Script::Thai,
            0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => Script::Korean,
            0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FFFF => Script::Cjk,
            _ if c.is_numeric() => Script::Numbers,
            _ if c.is_alphabetic() => Script::OtherLetters,
            _ => return None,
        })
    }
}

/// How the tokens are spelled in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spelling {
    /// GPT-2 style: every byte is mapped to a printable character (a space is `Ġ`).
    ByteLevel,
    /// SentencePiece: a space is `▁`, and raw bytes are `<0xNN>` tokens.
    SentencePiece,
    /// Written as they are (WordPiece and the like).
    Plain,
}

pub struct Vocab {
    /// Where it was read from, for the summary ("the GGUF file", "tokenizer.json").
    pub from: &'static str,
    /// The tokenizer family, such as "gpt2" (BPE) or "llama" (SentencePiece) for GGUF, or "BPE",
    /// "Unigram" or "WordPiece" for tokenizer.json.
    pub model: Option<String>,
    /// GGUF's pre-tokenizer name (how text is split before it's tokenized), such as "qwen2".
    pub pre: Option<String>,
    pub spelling: Spelling,
    /// The tokens as written in the file.
    pub tokens: Vec<String>,
    pub kinds: Vec<Kind>,
    /// The tokens as text: spaces, newlines and tabs made visible.
    pub shown: Vec<String>,
    pub scripts: Vec<Script>,
    /// Merge rules (BPE).
    pub merges: Option<u64>,
    /// The roles of special tokens: ("Start of text", id).
    pub special: Vec<(&'static str, u32)>,
    pub add_bos: Option<bool>,
    /// Chat templates: (name, template); the default one has an empty name.
    pub templates: Vec<(String, String)>,
}

impl Vocab {
    /// A token's text as the model writes it (spaces as spaces), for copying.
    pub fn text(&self, i: usize) -> String {
        let t = &self.tokens[i];
        match (self.kinds[i], self.spelling) {
            (Kind::Normal, Spelling::ByteLevel) => match decode_byte_level(t, &byte_table()) {
                Ok(s) => s,
                Err(_) => t.clone(),
            },
            (Kind::Normal, Spelling::SentencePiece) => t.replace('▁', " "),
            _ => t.clone(),
        }
    }
}

/// Tokens a model's vocabulary is read from: the GGUF file, else the publisher's tokenizer.json.
pub fn read(m: &Model, publisher: Option<&Path>) -> Result<Vocab, String> {
    if m.format == Format::Gguf
        && let Some(f) = m.files.iter().find(|f| !f.not_local)
    {
        match from_gguf(&f.path) {
            Ok(Some(v)) => return Ok(v),
            Ok(None) => {}
            Err(e) => return Err(e),
        }
    }
    if let Some(dir) = publisher
        && dir.join("tokenizer.json").is_file()
    {
        return from_hf(dir);
    }
    Err("This model doesn't include a vocabulary.".into())
}

/// Whether there is a vocabulary to show, without reading it (for the tab).
pub fn present(meta: &[(String, String)], publisher: Option<&Path>) -> bool {
    meta.iter().any(|(k, _)| k == "tokenizer.ggml.tokens") || publisher.is_some_and(|d| d.join("tokenizer.json").is_file())
}

const SPECIAL_KEYS: [(&str, &str); 11] = [
    ("bos", "Start of text"),
    ("eos", "End of text"),
    ("eot", "End of turn"),
    ("eom", "End of message"),
    ("unknown", "Unknown"),
    ("padding", "Padding"),
    ("separator", "Separator"),
    ("mask", "Mask"),
    ("fim_pre", "Fill in: before"),
    ("fim_suf", "Fill in: after"),
    ("fim_mid", "Fill in: middle"),
];

/// `None` when the file has no vocabulary.
pub fn from_gguf(path: &Path) -> Result<Option<Vocab>, String> {
    let h = crate::gguf::read_full(path, &["tokenizer.ggml.tokens", "tokenizer.ggml.token_type"]).map_err(|e| e.to_string())?;
    let Some(crate::gguf::Value::Array(items)) = h.get("tokenizer.ggml.tokens") else { return Ok(None) };
    let tokens: Vec<String> = items.iter().map(|v| v.as_str().unwrap_or_default().to_string()).collect();
    let kinds: Vec<Kind> = match h.get("tokenizer.ggml.token_type") {
        Some(crate::gguf::Value::Array(t)) if t.len() == tokens.len() => t.iter().map(|v| Kind::from_gguf(v.as_u64().unwrap_or(1))).collect(),
        _ => vec![Kind::Normal; tokens.len()],
    };
    let model = h.str("tokenizer.ggml.model").map(str::to_string);
    let spelling = match model.as_deref() {
        Some("llama" | "t5") => Spelling::SentencePiece,
        Some("bert") => Spelling::Plain,
        Some(_) => Spelling::ByteLevel,
        None => guess_spelling(&tokens),
    };
    let mut special = Vec::new();
    for (key, role) in SPECIAL_KEYS {
        if let Some(id) = h.u64(&format!("tokenizer.ggml.{key}_token_id")).filter(|&i| (i as usize) < tokens.len()) {
            special.push((role, id as u32));
        }
    }
    let mut templates = Vec::new();
    for (k, v) in &h.meta {
        if let Some(rest) = k.strip_prefix("tokenizer.chat_template")
            && let Some(t) = v.as_str()
        {
            templates.push((rest.trim_start_matches('.').to_string(), t.to_string()));
        }
    }
    templates.sort_by_key(|(n, _)| !n.is_empty());
    let add_bos = match h.get("tokenizer.ggml.add_bos_token") {
        Some(crate::gguf::Value::Bool(b)) => Some(*b),
        _ => None,
    };
    Ok(Some(finish(Vocab {
        from: "the GGUF file",
        model,
        pre: h.str("tokenizer.ggml.pre").map(str::to_string),
        spelling,
        tokens,
        kinds,
        shown: Vec::new(),
        scripts: Vec::new(),
        merges: h.get("tokenizer.ggml.merges").and_then(|v| v.len()),
        special,
        add_bos,
        templates,
    })))
}

/// Reads `tokenizer.json` and `tokenizer_config.json` (Hugging Face) in `dir`.
pub fn from_hf(dir: &Path) -> Result<Vocab, String> {
    let p = dir.join("tokenizer.json");
    let len = std::fs::metadata(&p).map_err(|e| e.to_string())?.len();
    if len > 128 << 20 {
        return Err("tokenizer.json is too large to read.".into());
    }
    let text = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
    let j: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("tokenizer.json can't be read: {e}"))?;
    drop(text);
    let model = &j["model"];
    let kind = model["type"].as_str().map(str::to_string);
    let mut pairs: Vec<(u32, String)> = match &model["vocab"] {
        serde_json::Value::Object(o) => o.iter().filter_map(|(t, id)| Some((id.as_u64()? as u32, t.clone()))).collect(),
        // Unigram: [[token, score], …] in id order.
        serde_json::Value::Array(a) => a.iter().enumerate().filter_map(|(i, e)| Some((i as u32, e.get(0)?.as_str()?.to_string()))).collect(),
        _ => Vec::new(),
    };
    let mut added = Vec::new();
    if let Some(a) = j["added_tokens"].as_array() {
        for t in a {
            if let (Some(id), Some(c)) = (t["id"].as_u64(), t["content"].as_str()) {
                added.push((id as u32, c.to_string(), t["special"].as_bool().unwrap_or(false)));
                pairs.push((id as u32, c.to_string()));
            }
        }
    }
    if pairs.is_empty() {
        return Err("tokenizer.json has no vocabulary.".into());
    }
    let n = pairs.iter().map(|(i, _)| *i as usize).max().unwrap_or(0) + 1;
    if n > 4 * pairs.len() + 1024 {
        return Err("tokenizer.json numbers its tokens oddly.".into());
    }
    let mut tokens = vec![String::new(); n];
    let mut kinds = vec![Kind::Unused; n];
    for (i, t) in pairs {
        kinds[i as usize] = if t.len() == 6 && t.starts_with("<0x") && t.ends_with('>') { Kind::Byte } else { Kind::Normal };
        tokens[i as usize] = t;
    }
    for (i, _, special) in &added {
        kinds[*i as usize] = if *special { Kind::Control } else { Kind::Added };
    }
    let byte_level = |v: &serde_json::Value| -> bool {
        v["type"].as_str() == Some("ByteLevel")
            || v["pretokenizers"].as_array().is_some_and(|a| a.iter().any(|p| p["type"].as_str() == Some("ByteLevel")))
            || v["decoders"].as_array().is_some_and(|a| a.iter().any(|p| p["type"].as_str() == Some("ByteLevel")))
    };
    let spelling = if byte_level(&j["pre_tokenizer"]) || byte_level(&j["decoder"]) {
        Spelling::ByteLevel
    } else if kind.as_deref() == Some("WordPiece") {
        Spelling::Plain
    } else {
        guess_spelling(&tokens)
    };
    let id_of = |t: &str| tokens.iter().position(|x| x == t).map(|i| i as u32);
    let mut special = Vec::new();
    let mut templates = Vec::new();
    let mut add_bos = None;
    let config = std::fs::read_to_string(dir.join("tokenizer_config.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    if let Some(c) = &config {
        let name = |k: &str| match &c[k] {
            serde_json::Value::String(s) => Some(s.clone()),
            v => v["content"].as_str().map(str::to_string),
        };
        for (k, role) in [
            ("bos_token", "Start of text"),
            ("eos_token", "End of text"),
            ("unk_token", "Unknown"),
            ("pad_token", "Padding"),
            ("sep_token", "Separator"),
            ("mask_token", "Mask"),
        ] {
            if let Some(id) = name(k).and_then(|t| id_of(&t)) {
                special.push((role, id));
            }
        }
        add_bos = c["add_bos_token"].as_bool();
        match &c["chat_template"] {
            serde_json::Value::String(s) => templates.push((String::new(), s.clone())),
            serde_json::Value::Array(a) => {
                for t in a {
                    if let (Some(n), Some(s)) = (t["name"].as_str(), t["template"].as_str()) {
                        templates.push((if n == "default" { String::new() } else { n.to_string() }, s.to_string()));
                    }
                }
            }
            _ => {}
        }
    }
    // Newer repositories keep the template in its own file.
    if templates.is_empty()
        && let Ok(t) = std::fs::read_to_string(dir.join("chat_template.jinja"))
    {
        templates.push((String::new(), t));
    }
    templates.sort_by_key(|(n, _)| !n.is_empty());
    Ok(finish(Vocab {
        from: "tokenizer.json",
        model: kind,
        pre: None,
        spelling,
        tokens,
        kinds,
        shown: Vec::new(),
        scripts: Vec::new(),
        merges: model["merges"].as_array().map(|a| a.len() as u64),
        special,
        add_bos,
        templates,
    }))
}

fn guess_spelling(tokens: &[String]) -> Spelling {
    let count = |c: char| tokens.iter().take(50_000).filter(|t| t.starts_with(c)).count();
    let (g, s) = (count('Ġ'), count('▁'));
    if g > s && g > 10 {
        Spelling::ByteLevel
    } else if s > 10 {
        Spelling::SentencePiece
    } else {
        Spelling::Plain
    }
}

fn finish(mut v: Vocab) -> Vocab {
    let table = byte_table();
    v.shown = Vec::with_capacity(v.tokens.len());
    v.scripts = Vec::with_capacity(v.tokens.len());
    for (t, &k) in v.tokens.iter().zip(&v.kinds) {
        let (shown, script) = match k {
            Kind::Control | Kind::Added | Kind::Unknown | Kind::Unused => (t.clone(), Script::Special),
            Kind::Byte => (t.clone(), Script::Partial),
            Kind::Normal => match v.spelling {
                Spelling::ByteLevel => match decode_byte_level(t, &table) {
                    Ok(s) => (visible(&s), script_of(&s)),
                    Err(b) => (b.iter().map(|b| format!("\\x{b:02x}")).collect(), Script::Partial),
                },
                Spelling::SentencePiece => {
                    let s = t.replace('▁', " ");
                    (visible(&s), script_of(&s))
                }
                Spelling::Plain => (visible(t), script_of(t.trim_start_matches("##"))),
            },
        };
        v.shown.push(shown);
        v.scripts.push(script);
    }
    v
}

/// GPT-2's map from printable characters back to the bytes they stand for.
fn byte_table() -> std::collections::HashMap<char, u8> {
    let printable: Vec<u32> = (b'!' as u32..=b'~' as u32).chain(0xA1..=0xAC).chain(0xAE..=0xFF).collect();
    let mut map = std::collections::HashMap::new();
    let mut extra = 0;
    for b in 0..=255u32 {
        let c = if printable.contains(&b) {
            b
        } else {
            extra += 1;
            255 + extra
        };
        map.insert(char::from_u32(c).unwrap(), b as u8);
    }
    map
}

/// The text a byte-level token stands for, or its bytes when they aren't whole characters.
fn decode_byte_level(t: &str, table: &std::collections::HashMap<char, u8>) -> Result<String, Vec<u8>> {
    let mut bytes = Vec::with_capacity(t.len());
    for c in t.chars() {
        match table.get(&c) {
            Some(&b) => bytes.push(b),
            // Not byte-level spelling after all: keep the character.
            None => bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
        }
    }
    String::from_utf8(bytes).map_err(|e| e.into_bytes())
}

/// How a space is shown in a token (the "open box" isn't in the app's fonts).
pub const SPACE: char = '·';

/// Spaces, newlines and tabs as visible marks.
fn visible(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            ' ' => out.push(SPACE),
            '\n' => out.push('↵'),
            '\t' => out.push('⇥'),
            c if c.is_control() || c == '\u{FEFF}' || c == '\u{200B}' => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// A search typed by the user, with spaces, tabs and newlines as the marks tokens are shown with.
pub fn visible_query(s: &str) -> String {
    visible(s)
}

fn script_of(s: &str) -> Script {
    if s.is_empty() {
        return Script::Special;
    }
    if s.chars().all(char::is_whitespace) {
        return Script::Spaces;
    }
    s.chars().find_map(Script::of_char).unwrap_or(Script::Symbols)
}

/// What a vocabulary is made of, for the Tokens tab.
pub struct Summary {
    /// Tokens of each script, in `Script::ALL` order.
    pub scripts: Vec<(Script, usize)>,
    /// Tokens by length in characters: 1, 2, … 15, then 16 or more.
    pub lengths: [usize; 16],
    /// Tokens that start a word (begin with a space).
    pub word_starts: usize,
    pub kinds: Vec<(Kind, usize)>,
    pub longest: Vec<u32>,
}

pub fn summarise(v: &Vocab) -> Summary {
    let mut by = std::collections::HashMap::new();
    let mut lengths = [0usize; 16];
    let mut word_starts = 0;
    let mut kinds: Vec<(Kind, usize)> = Vec::new();
    for (i, s) in v.scripts.iter().enumerate() {
        *by.entry(*s).or_insert(0usize) += 1;
        match kinds.iter_mut().find(|(k, _)| *k == v.kinds[i]) {
            Some(e) => e.1 += 1,
            None => kinds.push((v.kinds[i], 1)),
        }
        if v.kinds[i] == Kind::Normal && *s != Script::Partial {
            let n = v.shown[i].chars().count();
            lengths[n.clamp(1, 16) - 1] += 1;
            if v.shown[i].starts_with(SPACE) {
                word_starts += 1;
            }
        }
    }
    let mut longest: Vec<u32> = (0..v.tokens.len() as u32).filter(|&i| v.kinds[i as usize] == Kind::Normal && v.scripts[i as usize] != Script::Spaces).collect();
    longest.sort_by_key(|&i| std::cmp::Reverse(v.shown[i as usize].chars().count()));
    longest.truncate(12);
    Summary {
        scripts: Script::ALL.iter().map(|s| (*s, by.get(s).copied().unwrap_or(0))).filter(|(_, n)| *n > 0).collect(),
        lengths,
        word_starts,
        kinds,
        longest,
    }
}

/// What a chat template says about the model: (the format it uses, what it supports).
pub fn template_traits(t: &str) -> (Option<&'static str>, Vec<&'static str>) {
    let format = if t.contains("<|im_start|>") {
        Some("ChatML (used by Qwen and others)")
    } else if t.contains("<|start_header_id|>") {
        Some("Llama 3")
    } else if t.contains("<start_of_turn>") {
        Some("Gemma")
    } else if t.contains("[INST]") {
        Some("Mistral / Llama 2")
    } else if t.contains("<|user|>") {
        Some("Phi / Zephyr")
    } else if t.contains("<｜User｜>") {
        Some("DeepSeek")
    } else {
        None
    };
    let mut has = Vec::new();
    if t.contains("tools") {
        has.push("tool calls");
    }
    if t.contains("<think>") || t.contains("enable_thinking") || t.contains("reasoning") {
        has.push("a thinking mode");
    }
    if t.contains("image") || t.contains("vision") {
        has.push("images");
    }
    if t.contains("system") {
        has.push("system prompts");
    }
    (format, has)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab(tokens: &[&str], spelling: Spelling) -> Vocab {
        finish(Vocab {
            from: "test",
            model: None,
            pre: None,
            spelling,
            tokens: tokens.iter().map(|s| s.to_string()).collect(),
            kinds: vec![Kind::Normal; tokens.len()],
            shown: Vec::new(),
            scripts: Vec::new(),
            merges: None,
            special: Vec::new(),
            add_bos: None,
            templates: Vec::new(),
        })
    }

    #[test]
    fn byte_level_tokens_are_decoded() {
        // "Ġhello" is " hello"; "Ċ" is a newline; "ä½ł" is 你 in GPT-2 spelling; "ä½" is half of it.
        let v = vocab(&["Ġhello", "Ċ", "ä½ł", "ä½", "ĠĠĠ", "123", "Ġпривет", "!=", "ĠìķĪ"], Spelling::ByteLevel);
        assert_eq!(v.shown[0], "·hello");
        assert_eq!(v.text(0), " hello");
        assert_eq!(v.shown[1].len(), "↵".len());
        assert_eq!(v.scripts[0], Script::Latin);
        assert_eq!(v.shown[1], "↵");
        assert_eq!(v.scripts[1], Script::Spaces);
        assert_eq!(v.shown[2], "你");
        assert_eq!(v.scripts[2], Script::Cjk);
        assert_eq!(v.scripts[3], Script::Partial);
        assert_eq!(v.shown[3], "\\xe4\\xbd");
        assert_eq!(v.scripts[4], Script::Spaces);
        assert_eq!(v.scripts[5], Script::Numbers);
        assert_eq!(v.scripts[6], Script::Cyrillic);
        assert_eq!(v.scripts[7], Script::Symbols);
        assert_eq!(v.scripts[8], Script::Korean);
    }

    #[test]
    fn sentencepiece_spaces() {
        let v = vocab(&["▁the", "ing", "▁▁"], Spelling::SentencePiece);
        assert_eq!(v.shown[0], "·the");
        assert_eq!(v.shown[1], "ing");
        assert_eq!(v.scripts[2], Script::Spaces);
        let s = summarise(&v);
        assert_eq!(s.word_starts, 2);
        assert_eq!((s.lengths[1], s.lengths[2], s.lengths[3]), (1, 1, 1));
    }

    #[test]
    fn reads_a_gguf_vocabulary() {
        let dir = std::env::temp_dir().join(format!("fs-vocab-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("v.gguf");
        // A vocabulary longer than the header reader keeps by default.
        let tokens: Vec<String> = (0..5000).map(|i| format!("Ġt{i}")).chain(["<|im_end|>".to_string()]).collect();
        let mut types = vec![1u32; 5000];
        types.push(3);
        crate::gguf::write_vocab_test(&p, &tokens, &types, 5000, "{% for m in messages %}<|im_start|>{{ m.content }}{% endfor %}");
        let v = from_gguf(&p).unwrap().unwrap();
        assert_eq!(v.tokens.len(), 5001);
        assert_eq!(v.kinds[5000], Kind::Control);
        assert_eq!(v.shown[7], "·t7");
        assert_eq!(v.special, vec![("End of text", 5000)]);
        assert_eq!(v.templates.len(), 1);
        assert_eq!(template_traits(&v.templates[0].1).0, Some("ChatML (used by Qwen and others)"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reads_tokenizer_json() {
        let dir = std::env::temp_dir().join(format!("fs-vocab-hf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("tokenizer.json"),
            r#"{"model":{"type":"BPE","vocab":{"a":0,"Ġb":1,"Ċ":2},"merges":["a b"]},
                "added_tokens":[{"id":3,"content":"</s>","special":true}],
                "pre_tokenizer":{"type":"ByteLevel"}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("tokenizer_config.json"), r#"{"eos_token":"</s>","chat_template":"[INST] {{ m }} [/INST]"}"#).unwrap();
        let v = from_hf(&dir).unwrap();
        assert_eq!(v.tokens.len(), 4);
        assert_eq!(v.shown[1], "·b");
        assert_eq!(v.kinds[3], Kind::Control);
        assert_eq!(v.special, vec![("End of text", 3)]);
        assert_eq!(v.merges, Some(1));
        assert_eq!(template_traits(&v.templates[0].1).0, Some("Mistral / Llama 2"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
