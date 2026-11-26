//! Music projects: Ableton Live (.als), REAPER (.rpp), FL Studio (.flp), Cubase/Nuendo
//! (.cpr, .npr) and Logic Pro (.logicx, a folder package). Shows the version that saved the project, tempo, tracks, plugins and the sound
//! files it uses, marking any that are missing from this computer.
//!
//! Projects are only read. Plugins aren't loaded, and sound files are only checked for.

use crate::help::doc::{Block, Span};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Projects larger than this aren't read.
const FILE_MAX: u64 = 64 << 20;
/// Ableton projects unpack to at most this much XML.
const XML_MAX: u64 = 256 << 20;
/// Rows listed of tracks, plugins or sound files.
const LIST_MAX: usize = 300;

#[derive(Default)]
pub struct Project {
    app: &'static str,
    version: String,
    tempo: Option<f64>,
    signature: Option<String>,
    /// Name, kind ("Audio", "MIDI", ...).
    tracks: Vec<(String, String)>,
    plugins: Vec<String>,
    samples: Vec<String>,
    clips: usize,
    extra: Vec<(&'static str, String)>,
    title: String,
}

pub fn is_project(name: &str) -> bool {
    matches!(crate::scan::ext_of(name).to_ascii_lowercase().as_str(), "als" | "rpp" | "flp" | "cpr" | "npr" | "logicx")
}

/// A Logic Pro project: a folder that the Finder shows as one file.
pub fn is_logic(name: &str) -> bool {
    crate::scan::ext_of(name).eq_ignore_ascii_case("logicx")
}

/// The alternative (version) of a Logic project to show: 000 when there is one, otherwise the
/// first that has its details saved.
fn logic_alternative(pkg: &Path) -> Option<PathBuf> {
    let alts = pkg.join("Alternatives");
    let first = alts.join("000");
    if first.join("MetaData.plist").is_file() {
        return Some(first);
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&alts).ok()?.flatten().map(|e| e.path()).filter(|p| p.join("MetaData.plist").is_file()).collect();
    dirs.sort();
    dirs.into_iter().next()
}

/// Logic's own picture of the project's window, saved with each alternative.
pub fn logic_window(pkg: &Path) -> Option<image::RgbaImage> {
    let p = logic_alternative(pkg)?.join("WindowImage.jpg");
    if std::fs::metadata(&p).ok()?.len() > FILE_MAX {
        return None;
    }
    image::load_from_memory_with_format(&std::fs::read(p).ok()?, image::ImageFormat::Jpeg).ok().map(|i| i.to_rgba8())
}

fn logic(pkg: &Path) -> Result<Project, String> {
    let alt = logic_alternative(pkg).ok_or("Logic project details not found in the package")?;
    let meta = plist::Value::from_file(alt.join("MetaData.plist")).map_err(|e| format!("Couldn't read the project's details: {e}"))?;
    let d = meta.as_dictionary().ok_or("Couldn't read the project's details")?;
    let num = |k: &str| d.get(k).and_then(|v| v.as_real().or_else(|| v.as_signed_integer().map(|i| i as f64)));
    let text = |k: &str| d.get(k).and_then(|v| v.as_string()).map(str::to_string).filter(|s| !s.is_empty());
    let list = |k: &str| -> Vec<String> { d.get(k).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_string().map(str::to_string)).collect()).unwrap_or_default() };
    let mut p = Project { app: "Logic Pro", tempo: num("BeatsPerMinute"), ..Default::default() };
    if let (Some(n), Some(dn)) = (num("SongSignatureNumerator"), num("SongSignatureDenominator")) {
        p.signature = Some(format!("{n}/{dn}"));
    }
    if let Some(key) = text("SongKey") {
        let mode = text("SongGenderKey").map(|g| format!(" {}", g.to_lowercase())).unwrap_or_default();
        p.extra.push(("Key", format!("{key}{mode}")));
    }
    if let Some(n) = num("NumberOfTracks") {
        p.extra.push(("Tracks", group(n as usize)));
    }
    if let Some(r) = num("SampleRate") {
        p.extra.push(("Sample rate", format!("{} kHz", r / 1000.0)));
    }
    // Sound files are listed relative to the package (or as full paths).
    p.samples = list("AudioFiles");
    for (k, label) in [("SamplerInstrumentsFiles", "Sampler instruments"), ("UltrabeatFiles", "Ultrabeat files"), ("AlchemyFiles", "Alchemy files"), ("ImpulsResponsesFiles", "Impulse responses")] {
        let n = list(k).len();
        if n > 0 {
            p.extra.push((label, group(n)));
        }
    }
    let unused = list("UnusedAudioFiles").len();
    if unused > 0 {
        p.extra.push(("Unused sound files", group(unused)));
    }
    if let Ok(info) = plist::Value::from_file(pkg.join("Resources/ProjectInformation.plist"))
        && let Some(v) = info.as_dictionary().and_then(|d| d.get("LastSavedFrom")).and_then(|v| v.as_string())
    {
        p.version = v.to_string();
    }
    let alts = std::fs::read_dir(pkg.join("Alternatives")).map(|r| r.flatten().filter(|e| e.path().join("MetaData.plist").is_file()).count()).unwrap_or(0);
    if alts > 1 {
        p.extra.push(("Alternatives", format!("{alts} (the first is shown)")));
    }
    Ok(p)
}

fn slurp(path: &Path) -> Result<Vec<u8>, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > FILE_MAX {
        return Err(format!("Too big to read ({})", crate::scan::human(len)));
    }
    let mut b = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(FILE_MAX).read_to_end(&mut b)).map_err(|e| e.to_string())?;
    Ok(b)
}

/// The project's blocks and details for the preview.
pub fn read(path: &Path) -> Result<(Vec<Block>, Vec<(&'static str, String)>), String> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if ext == "logicx" {
        let mut p = logic(path)?;
        p.title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        // Its sound files are inside the package.
        return Ok(p.render(Some(path)));
    }
    let raw = slurp(path)?;
    let mut p = match ext.as_str() {
        "als" => ableton(&raw)?,
        "rpp" => reaper(&String::from_utf8_lossy(&raw))?,
        "flp" => fl(&raw)?,
        _ => cubase(&raw)?,
    };
    if p.title.is_empty() {
        p.title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    }
    Ok(p.render(path.parent()))
}

fn group(n: usize) -> String {
    crate::inspect::group(n as u64)
}

fn dedup(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|x| !x.trim().is_empty() && seen.insert(x.clone()));
}

impl Project {
    fn render(mut self, dir: Option<&Path>) -> (Vec<Block>, Vec<(&'static str, String)>) {
        dedup(&mut self.plugins);
        dedup(&mut self.samples);
        let plain = |t: &str| vec![Span { text: t.to_string(), ..Default::default() }];
        let mut facts: Vec<(&'static str, String)> = vec![("App", self.app.to_string())];
        if !self.version.is_empty() {
            facts.push(("Saved with", self.version.clone()));
        }
        if let Some(t) = self.tempo {
            let t = if (t - t.round()).abs() < 0.005 { format!("{t:.0}") } else { format!("{t:.2}") };
            facts.push(("Tempo", format!("{t} BPM")));
        }
        if let Some(s) = &self.signature {
            facts.push(("Time signature", s.clone()));
        }
        if !self.tracks.is_empty() {
            facts.push(("Tracks", group(self.tracks.len())));
        }
        if self.clips > 0 {
            facts.push(("Clips", group(self.clips)));
        }
        if !self.plugins.is_empty() {
            facts.push(("Plugins", group(self.plugins.len())));
        }
        // Sound files: where they'd be, relative ones beside the project.
        let found = |s: &str| -> bool {
            let p = PathBuf::from(s.replace('\\', "/"));
            if p.is_absolute() || s.contains(":\\") { p.exists() } else { dir.is_some_and(|d| d.join(&p).exists()) }
        };
        let missing: Vec<bool> = self.samples.iter().take(LIST_MAX).map(|s| !found(s)).collect();
        if !self.samples.is_empty() {
            let gone = missing.iter().filter(|m| **m).count();
            let v = if gone > 0 { format!("{} ({} not on this computer)", group(self.samples.len()), group(gone)) } else { group(self.samples.len()) };
            facts.push(("Sound files", v));
        }
        facts.extend(self.extra.iter().cloned());
        let mut blocks = vec![Block::Heading { level: 1, spans: plain(&self.title), id: String::new() }];
        let line: Vec<String> = facts.iter().skip(1).take(4).map(|(k, v)| format!("{k}: {v}")).collect();
        blocks.push(Block::Para(plain(&format!("{} project. {}", self.app, line.join(" · ")))));
        if !self.tracks.is_empty() {
            blocks.push(Block::Heading { level: 2, spans: plain("Tracks"), id: String::new() });
            if self.tracks.iter().all(|t| t.1.is_empty()) {
                let items = self.tracks.iter().take(LIST_MAX).map(|(n, _)| vec![Block::Para(plain(n))]).collect();
                blocks.push(Block::List { start: Some(1), items });
            } else {
                let rows = self.tracks.iter().take(LIST_MAX).map(|(n, k)| vec![plain(n), plain(k)]).collect();
                blocks.push(Block::Table { head: vec![plain("Name"), plain("Kind")], rows });
            }
        }
        if !self.plugins.is_empty() {
            blocks.push(Block::Heading { level: 2, spans: plain("Plugins"), id: String::new() });
            let items = self.plugins.iter().take(LIST_MAX).map(|p| vec![Block::Para(plain(p))]).collect();
            blocks.push(Block::List { start: None, items });
        }
        if !self.samples.is_empty() {
            blocks.push(Block::Heading { level: 2, spans: plain("Sound files"), id: String::new() });
            let items = self
                .samples
                .iter()
                .zip(&missing)
                .map(|(s, gone)| {
                    let mut spans = plain(s);
                    if *gone {
                        spans.push(Span { text: "  (not found)".into(), italic: true, ..Default::default() });
                    }
                    vec![Block::Para(spans)]
                })
                .collect();
            blocks.push(Block::List { start: None, items });
        }
        (blocks, facts)
    }
}

// ---- Ableton Live ----

fn ableton(raw: &[u8]) -> Result<Project, String> {
    let mut xml = String::new();
    if raw.starts_with(&[0x1f, 0x8b]) {
        flate2::read::GzDecoder::new(raw).take(XML_MAX).read_to_string(&mut xml).map_err(|e| format!("Couldn't unpack it: {e}"))?;
    } else {
        xml = String::from_utf8_lossy(raw).into_owned();
    }
    let doc = roxmltree::Document::parse_with_options(&xml, roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: 50_000_000, ..Default::default() }).map_err(|e| format!("Not a Live set: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "Ableton" {
        return Err("Not an Ableton Live set".into());
    }
    let mut p = Project { app: "Ableton Live", ..Default::default() };
    p.version = root.attribute("Creator").unwrap_or("").to_string();
    let value = |n: roxmltree::Node, child: &str| n.children().find(|c| c.tag_name().name() == child).and_then(|c| c.attribute("Value")).map(str::to_string);
    let Some(set) = root.children().find(|c| c.tag_name().name() == "LiveSet") else { return Ok(p) };
    if let Some(tracks) = set.children().find(|c| c.tag_name().name() == "Tracks") {
        for t in tracks.children().filter(|c| c.is_element()) {
            let kind = match t.tag_name().name() {
                "AudioTrack" => "Audio",
                "MidiTrack" => "MIDI",
                "GroupTrack" => "Group",
                "ReturnTrack" => "Return",
                other => other,
            };
            let name = t.children().find(|c| c.tag_name().name() == "Name").and_then(|n| value(n, "EffectiveName").or_else(|| value(n, "UserName"))).unwrap_or_default();
            p.tracks.push((name, kind.to_string()));
        }
    }
    for n in set.descendants().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "Tempo" if p.tempo.is_none() => p.tempo = value(n, "Manual").and_then(|v| v.parse().ok()),
            "AudioClip" | "MidiClip" => p.clips += 1,
            "VstPluginInfo" => p.plugins.extend(value(n, "PlugName")),
            "Vst3PluginInfo" | "AuPluginInfo" => p.plugins.extend(value(n, "Name")),
            "SampleRef" => {
                let file = n.children().find(|c| c.tag_name().name() == "FileRef");
                if let Some(f) = file {
                    // Live 11+: Path; older: a folder list and a name.
                    let path = value(f, "Path").filter(|s| !s.is_empty()).or_else(|| {
                        let rel = value(f, "RelativePath").filter(|s| !s.is_empty())?;
                        Some(rel)
                    });
                    p.samples.extend(path);
                }
            }
            _ => {}
        }
    }
    Ok(p)
}

// ---- REAPER ----

/// The words of an RPP line, with "quoted strings" kept whole.
fn rpp_words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in line.trim().chars() {
        match (quote, c) {
            (None, '"' | '\'' | '`') => quote = Some(c),
            (Some(q), c) if c == q => {
                out.push(std::mem::take(&mut cur));
                quote = None;
            }
            (None, ' ' | '\t') => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn reaper(text: &str) -> Result<Project, String> {
    let first = text.lines().next().unwrap_or("");
    if !first.trim_start().starts_with("<REAPER_PROJECT") {
        return Err("Not a REAPER project".into());
    }
    let mut p = Project { app: "REAPER", ..Default::default() };
    if let Some(v) = rpp_words(first).get(2) {
        p.version = format!("REAPER {}", v.split('/').next().unwrap_or(v));
    }
    // The blocks we're inside, innermost last.
    let mut stack: Vec<String> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix('<') {
            let w = rpp_words(rest);
            let tag = w.first().cloned().unwrap_or_default();
            match tag.as_str() {
                "TRACK" => p.tracks.push((String::new(), String::new())),
                "ITEM" => p.clips += 1,
                "VST" | "AU" | "JS" | "CLAP" | "LV2" | "DX" => {
                    if let Some(name) = w.get(1) {
                        // "VST3: Pro-Q 3 (FabFilter)" → "Pro-Q 3 (FabFilter)"
                        let name = name.split_once(": ").map_or(name.as_str(), |x| x.1);
                        p.plugins.push(name.to_string());
                    }
                }
                _ => {}
            }
            stack.push(tag);
        } else if l == ">" {
            stack.pop();
        } else {
            let w = rpp_words(l);
            match (stack.last().map(String::as_str), w.first().map(String::as_str)) {
                (Some("REAPER_PROJECT"), Some("TEMPO")) => {
                    p.tempo = w.get(1).and_then(|t| t.parse().ok());
                    if let (Some(a), Some(b)) = (w.get(2), w.get(3)) {
                        p.signature = Some(format!("{a}/{b}"));
                    }
                }
                (Some("TRACK"), Some("NAME")) => {
                    if let Some(t) = p.tracks.last_mut() {
                        t.0 = w.get(1).cloned().unwrap_or_default();
                    }
                }
                (Some("SOURCE"), Some("FILE")) => p.samples.extend(w.get(1).cloned()),
                (Some("REAPER_PROJECT"), Some("SAMPLERATE")) => {
                    if let Some(r) = w.get(1) {
                        p.extra.push(("Sample rate", format!("{r} Hz")));
                    }
                }
                _ => {}
            }
        }
    }
    for (i, t) in p.tracks.iter_mut().enumerate() {
        if t.0.is_empty() {
            t.0 = format!("Track {}", i + 1);
        }
    }
    Ok(p)
}

// ---- FL Studio ----

/// FL Studio text: UTF-16 in newer projects, ASCII in older ones, ending in a zero.
fn fl_text(b: &[u8]) -> String {
    let utf16 = b.len() >= 2 && b.len() % 2 == 0 && b.iter().skip(1).step_by(2).filter(|&&c| c == 0).count() * 2 >= b.len() / 2;
    let s = if utf16 { String::from_utf16_lossy(&b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()) } else { String::from_utf8_lossy(b).into_owned() };
    s.trim_end_matches('\0').trim().to_string()
}

fn fl(raw: &[u8]) -> Result<Project, String> {
    if !raw.starts_with(b"FLhd") || raw.len() < 22 {
        return Err("Not an FL Studio project".into());
    }
    let hlen = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as usize;
    let mut i = 8 + hlen;
    if raw.get(i..i + 4) != Some(b"FLdt") {
        return Err("Not an FL Studio project".into());
    }
    i += 8;
    let mut p = Project { app: "FL Studio", ..Default::default() };
    let mut channels = 0;
    let mut patterns = std::collections::HashSet::new();
    let mut old_tempo = None;
    while i < raw.len() {
        let id = raw[i];
        i += 1;
        match id {
            0..=63 => i += 1,
            64..=127 => {
                let v = raw.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
                match (id, v) {
                    (64, _) => channels += 1,
                    (65, Some(v)) => {
                        patterns.insert(v);
                    }
                    (66, Some(v)) => old_tempo = Some(v as f64),
                    _ => {}
                }
                i += 2;
            }
            128..=191 => {
                let v = raw.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                if let (156, Some(v)) = (id, v) {
                    p.tempo = Some(v as f64 / 1000.0);
                }
                i += 4;
            }
            _ => {
                // Text and data: a variable-length size, then the bytes.
                let (mut len, mut shift) = (0usize, 0);
                while let Some(&b) = raw.get(i) {
                    i += 1;
                    len |= ((b & 0x7f) as usize) << shift;
                    shift += 7;
                    if b & 0x80 == 0 || shift > 28 {
                        break;
                    }
                }
                let Some(data) = raw.get(i..i + len) else { break };
                match id {
                    194 => p.title = fl_text(data),
                    199 => p.version = format!("FL Studio {}", fl_text(data)),
                    206 => p.extra.push(("Genre", fl_text(data))),
                    207 => p.extra.push(("Artist", fl_text(data))),
                    196 => p.samples.push(fl_text(data)),
                    201 => {
                        let n = fl_text(data);
                        if !n.is_empty() {
                            p.plugins.push(n);
                        }
                    }
                    _ => {}
                }
                i += len;
            }
        }
    }
    if p.tempo.is_none() {
        p.tempo = old_tempo;
    }
    if channels > 0 {
        p.extra.push(("Channels", group(channels)));
    }
    if !patterns.is_empty() {
        p.extra.push(("Patterns", group(patterns.len())));
    }
    p.extra.retain(|(_, v)| !v.is_empty());
    Ok(p)
}

// ---- Cubase and Nuendo ----

fn cubase(raw: &[u8]) -> Result<Project, String> {
    if !raw.starts_with(b"RIFF") {
        return Err("Not a Cubase or Nuendo project".into());
    }
    let head = &raw[..raw.len().min(8 << 20)];
    // The version is written as text: "Cubase 13.0.30" or "Nuendo 13.0.20".
    let mut p = Project { app: "Cubase", ..Default::default() };
    for app in ["Cubase", "Nuendo"] {
        if let Some(i) = head.windows(app.len() + 1).position(|w| w[..app.len()] == *app.as_bytes() && (w[app.len()] == b' ' || w[app.len()] == 0)) {
            let s: String = head[i..].iter().take(40).take_while(|&&c| c.is_ascii_graphic() || c == b' ').map(|&c| c as char).collect();
            if s.len() > app.len() + 1 {
                p.version = s.trim().to_string();
            }
            p.app = if app == "Nuendo" { "Nuendo" } else { "Cubase" };
            break;
        }
    }
    // Sound files are named in the pool as paths; pick out ones that end in a sound extension.
    let text = String::from_utf8_lossy(head);
    for w in text.split(|c: char| c.is_control()) {
        let lw = w.to_ascii_lowercase();
        if [".wav", ".aif", ".aiff", ".mp3", ".flac"].iter().any(|e| lw.ends_with(e)) && w.len() < 400 && (w.contains('/') || w.contains('\\')) {
            p.samples.push(w.trim().to_string());
        }
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fact<'a>(f: &'a [(&'static str, String)], k: &str) -> Option<&'a str> {
        f.iter().find(|x| x.0 == k).map(|x| x.1.as_str())
    }

    #[test]
    fn ableton_set() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Ableton MajorVersion="5" MinorVersion="11.0_433" Creator="Ableton Live 11.3.21"><LiveSet>
<Tracks>
 <AudioTrack><Name><EffectiveName Value="Vocals"/></Name><DeviceChain><MainSequencer><ClipSlotList><AudioClip><SampleRef><FileRef><Path Value="/nowhere/vox.wav"/></FileRef></SampleRef></AudioClip></ClipSlotList></MainSequencer>
   <Devices><PluginDevice><PluginDesc><Vst3PluginInfo><Name Value="Pro-Q 3"/></Vst3PluginInfo></PluginDesc></PluginDevice></Devices></DeviceChain></AudioTrack>
 <MidiTrack><Name><EffectiveName Value="Bass"/></Name><MidiClip/><MidiClip/></MidiTrack>
 <ReturnTrack><Name><EffectiveName Value="A-Reverb"/></Name></ReturnTrack>
</Tracks>
<MasterTrack><DeviceChain><Mixer><Tempo><Manual Value="124"/></Tempo></Mixer></DeviceChain></MasterTrack>
</LiveSet></Ableton>"#;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(xml.as_bytes()).unwrap();
        let mut p = ableton(&gz.finish().unwrap()).unwrap();
        p.title = "Song".into();
        let (blocks, facts) = p.render(None);
        assert_eq!(fact(&facts, "Saved with"), Some("Ableton Live 11.3.21"));
        assert_eq!(fact(&facts, "Tempo"), Some("124 BPM"));
        assert_eq!(fact(&facts, "Tracks"), Some("3"));
        assert_eq!(fact(&facts, "Clips"), Some("3"));
        assert_eq!(fact(&facts, "Plugins"), Some("1"));
        assert_eq!(fact(&facts, "Sound files"), Some("1 (1 not on this computer)"));
        let text = crate::officedoc::plain_text(&blocks);
        assert!(text.contains("Vocals  Audio") && text.contains("Bass  MIDI"), "{text}");
    }

    #[test]
    fn reaper_project() {
        let rpp = "<REAPER_PROJECT 0.1 \"7.22/macOS-arm64\" 1712345678\n  TEMPO 96.5 3 4\n  SAMPLERATE 48000 0 0\n  <TRACK {A}\n    NAME \"Drums\"\n    <FXCHAIN\n      <VST \"VST3: Pro-C 2 (FabFilter)\" Pro-C.vst3 0\n      >\n    >\n    <ITEM\n      <SOURCE WAVE\n        FILE \"kick.wav\"\n      >\n    >\n  >\n  <TRACK {B}\n  >\n>\n";
        let mut p = reaper(rpp).unwrap();
        p.title = "Beat".into();
        let (_, facts) = p.render(None);
        assert_eq!(fact(&facts, "Saved with"), Some("REAPER 7.22"));
        assert_eq!(fact(&facts, "Tempo"), Some("96.50 BPM"));
        assert_eq!(fact(&facts, "Time signature"), Some("3/4"));
        assert_eq!(fact(&facts, "Tracks"), Some("2"));
        assert_eq!(fact(&facts, "Plugins"), Some("1"));
        assert_eq!(fact(&facts, "Sample rate"), Some("48000 Hz"));
        assert!(reaper("hello").is_err());
    }

    #[test]
    fn fl_studio_events() {
        let mut b = b"FLhd".to_vec();
        b.extend(6u32.to_le_bytes());
        b.extend([0, 0, 4, 0, 96, 0]);
        let mut ev = Vec::new();
        let text = |id: u8, s: &[u8], ev: &mut Vec<u8>| {
            ev.push(id);
            ev.push(s.len() as u8);
            ev.extend(s);
        };
        text(199, b"21.2.3.4004\0", &mut ev);
        let title: Vec<u8> = "My tune\0".encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        text(194, &title, &mut ev);
        ev.push(156);
        ev.extend(140_000u32.to_le_bytes());
        ev.extend([64, 0, 0, 64, 1, 0]);
        ev.extend([65, 1, 0, 65, 2, 0, 65, 1, 0]);
        b.extend(b"FLdt");
        b.extend((ev.len() as u32).to_le_bytes());
        b.extend(ev);
        let p = fl(&b).unwrap();
        assert_eq!(p.title, "My tune");
        let (_, facts) = p.render(None);
        assert_eq!(fact(&facts, "Saved with"), Some("FL Studio 21.2.3.4004"));
        assert_eq!(fact(&facts, "Tempo"), Some("140 BPM"));
        assert_eq!(fact(&facts, "Channels"), Some("2"));
        assert_eq!(fact(&facts, "Patterns"), Some("2"));
        assert!(fl(b"FLhd").is_err());
        // Cut short anywhere: no panic.
        for n in 0..b.len() {
            let _ = fl(&b[..n]);
        }
    }

    #[test]
    fn cubase_version() {
        let mut b = b"RIFF\0\0\0\0NUNDROOT".to_vec();
        b.extend(b"\0\0\x05Cubase 13.0.30\0\0/Users/me/Audio/take1.wav\0");
        let (_, facts) = cubase(&b).unwrap().render(None);
        assert_eq!(fact(&facts, "App"), Some("Cubase"));
        assert_eq!(fact(&facts, "Saved with"), Some("Cubase 13.0.30"));
        assert_eq!(fact(&facts, "Sound files"), Some("1 (1 not on this computer)"));
    }

    #[test]
    fn logic_package() {
        let pkg = std::env::temp_dir().join(format!("fs-logic-{}/Song.logicx", std::process::id()));
        std::fs::create_dir_all(pkg.join("Alternatives/000")).unwrap();
        std::fs::create_dir_all(pkg.join("Resources")).unwrap();
        std::fs::create_dir_all(pkg.join("Media")).unwrap();
        std::fs::write(pkg.join("Media/a.wav"), b"RIFF").unwrap();
        let mut meta = plist::Dictionary::new();
        meta.insert("BeatsPerMinute".into(), 98.5.into());
        meta.insert("SongSignatureNumerator".into(), 6i64.into());
        meta.insert("SongSignatureDenominator".into(), 8i64.into());
        meta.insert("SongKey".into(), "C".into());
        meta.insert("SongGenderKey".into(), "major".into());
        meta.insert("NumberOfTracks".into(), 3i64.into());
        meta.insert("AudioFiles".into(), plist::Value::Array(vec!["Media/a.wav".into(), "Media/b.wav".into()]));
        plist::Value::Dictionary(meta).to_file_binary(pkg.join("Alternatives/000/MetaData.plist")).unwrap();
        let mut info = plist::Dictionary::new();
        info.insert("LastSavedFrom".into(), "Logic Pro 11.1".into());
        plist::Value::Dictionary(info).to_file_xml(pkg.join("Resources/ProjectInformation.plist")).unwrap();
        let (_, facts) = read(&pkg).unwrap();
        let f = |k: &str| facts.iter().find(|x| x.0 == k).map(|x| x.1.as_str());
        assert_eq!(f("Saved with"), Some("Logic Pro 11.1"));
        assert_eq!(f("Tempo"), Some("98.50 BPM"));
        assert_eq!(f("Time signature"), Some("6/8"));
        assert_eq!(f("Key"), Some("C major"));
        assert_eq!(f("Tracks"), Some("3"));
        assert_eq!(f("Sound files"), Some("2 (1 not on this computer)"));
        assert!(logic_window(&pkg).is_none());
        let _ = std::fs::remove_dir_all(pkg.parent().unwrap());
    }
}
