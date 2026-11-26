//! After Effects projects (.aep, and the XML .aepx): the version that saved them, their
//! compositions and the footage files they use, read with `craft-aep` (from craft-libs, MIT or
//! Apache-2.0), a clean-room reader. Nothing is run, and footage is only looked up, not opened.

use crate::savedpic::Saved;
use std::path::Path;

/// Projects larger than this aren't read (the whole file is held while it's read).
const FILE_MAX: u64 = 256 << 20;
/// Compositions listed by name.
const NAMES: usize = 12;

pub fn is_project(name: &str) -> bool {
    matches!(crate::scan::ext_of(name).to_ascii_lowercase().as_str(), "aep" | "aepx")
}

pub fn read(path: &Path) -> Result<Saved, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("Couldn't read it: {e}"))?.len();
    if size > FILE_MAX {
        return Err(format!("Too big to read ({})", crate::scan::human(size)));
    }
    let data = std::fs::read(path).map_err(|e| format!("Couldn't read it: {e}"))?;
    let mut out = details(&data)?;
    let named: Vec<String> = footage_paths(&data);
    if !named.is_empty() {
        let refs = crate::projrefs::check("Footage files", named, path.parent());
        out.facts.push(("Footage files", refs.fact()));
        out.refs = Some(refs);
    }
    Ok(out)
}

fn parse(data: &[u8]) -> Result<craft_aep::AepFile<'_>, String> {
    let limits = craft_aep::Limits { max_chunks: 2_000_000, ..Default::default() };
    let file = if craft_aep::is_aepx(data) { craft_aep::read_aepx_with_limits(data, limits) } else { craft_aep::parse_with_limits(data, limits) };
    file.map_err(|_| "Not an After Effects project, or a damaged one".to_string())
}

fn version(v: craft_aep::AeVersion) -> String {
    format!("After Effects {}.{}.{}", v.major, v.minor, v.bugfix)
}

fn details(data: &[u8]) -> Result<Saved, String> {
    let file = parse(data)?;
    let project = craft_aep::Project::new(&file).ok_or("Not an After Effects project")?;
    let mut facts = vec![("App", "After Effects".to_string())];
    let saved = file.saved_by_version();
    if let Some(v) = saved {
        facts.push(("Saved with", version(v)));
    }
    // A copy saved for an older version says which.
    if let Some(v) = file.file_version().filter(|f| saved.is_none_or(|s| s.major != f.major || s.minor != f.minor)) {
        facts.push(("Saved for", version(v)));
    }
    if project.items_unreadable() {
        facts.push(("Items", "couldn't be read (the file may be damaged)".into()));
        return Ok(Saved { picture: None, facts, note: Some(NOTE.into()), refs: None });
    }
    let items = project.all_items();
    let comps: Vec<_> = items.iter().filter_map(|i| i.comp()).collect();
    let footage: Vec<_> = items.iter().filter_map(|i| i.footage()).collect();
    let solids = footage.iter().filter(|f| f.is_solid).count();
    let folders = items.iter().filter(|i| i.kind() == Some(craft_aep::ItemKind::Folder)).count();
    facts.push(("Compositions", count(comps.len(), "composition", "compositions")));
    for c in comps.iter().take(NAMES) {
        let name = c.name().unwrap_or_else(|| "(unnamed)".into());
        let layers = count(c.layers().count(), "layer", "layers");
        let line = match c.header() {
            Some(h) => {
                let mut s = format!("{name}: {} × {}", h.width, h.height);
                if let Some(r) = h.frame_rate().filter(|r| r.is_finite() && *r > 0.0) {
                    s.push_str(&format!(", {}", fps(r)));
                }
                if let Some(d) = h.duration_seconds().filter(|d| d.is_finite() && *d >= 0.0) {
                    s.push_str(&format!(", {}", seconds(d)));
                }
                format!("{s}, {layers}")
            }
            None => format!("{name}: {layers}"),
        };
        facts.push(("Composition", line));
    }
    if comps.len() > NAMES {
        facts.push(("Composition", format!("and {} more", comps.len() - NAMES)));
    }
    let files = footage.len() - solids;
    if files > 0 || solids > 0 {
        let mut s = count(files, "file", "files");
        if solids > 0 {
            s.push_str(&format!(" and {}", count(solids, "solid", "solids")));
        }
        facts.push(("Footage", s));
    }
    if folders > 0 {
        facts.push(("Folders", folders.to_string()));
    }
    Ok(Saved { picture: None, facts, note: Some(NOTE.into()), refs: None })
}

const NOTE: &str = "After Effects doesn't save a picture inside its projects, and Filescape doesn't draw them itself.";

/// The files footage items point at (an image sequence's folder for a sequence).
fn footage_paths(data: &[u8]) -> Vec<String> {
    let Ok(file) = parse(data) else { return Vec::new() };
    let Some(project) = craft_aep::Project::new(&file) else { return Vec::new() };
    project.all_items().iter().filter_map(|i| i.footage()).filter(|f| !f.is_solid).filter_map(|f| f.path).collect()
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", crate::inspect::group(n as u64), if n == 1 { one } else { many })
}

fn fps(r: f64) -> String {
    let s = format!("{r:.3}");
    format!("{} fps", s.trim_end_matches('0').trim_end_matches('.'))
}

fn seconds(s: f64) -> String {
    let t = s.round() as u64;
    if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60) } else { format!("{}:{:02}", t / 60, t % 60) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_aep::{AepFile, Chunk};

    fn utf8(s: &str) -> Chunk<'static> {
        Chunk::data(*b"Utf8", s.as_bytes().to_vec())
    }

    fn idta(kind: u16, id: u32) -> Chunk<'static> {
        let mut b = vec![0u8; 64];
        b[..2].copy_from_slice(&kind.to_be_bytes());
        b[16..20].copy_from_slice(&id.to_be_bytes());
        Chunk::data(*b"idta", b)
    }

    /// A 1920 × 1080 composition at 25 fps, 5 seconds long, with `layers` layers.
    fn comp(name: &str, id: u32, layers: usize) -> Chunk<'static> {
        let mut cdta = vec![0u8; 184];
        cdta[4..8].copy_from_slice(&1u32.to_be_bytes());
        cdta[8..12].copy_from_slice(&25u32.to_be_bytes());
        cdta[36..40].copy_from_slice(&u32::MAX.to_be_bytes());
        cdta[44..48].copy_from_slice(&125u32.to_be_bytes());
        cdta[140..142].copy_from_slice(&1920u16.to_be_bytes());
        cdta[142..144].copy_from_slice(&1080u16.to_be_bytes());
        cdta[144..148].copy_from_slice(&1u32.to_be_bytes());
        cdta[148..152].copy_from_slice(&1u32.to_be_bytes());
        let mut kids = vec![idta(4, id), utf8(name), Chunk::data(*b"cdta", cdta)];
        kids.extend((0..layers).map(|_| Chunk::list(*b"Layr", vec![Chunk::data(*b"ldta", vec![0u8; 128])])));
        Chunk::list(*b"Item", kids)
    }

    fn footage(id: u32, path: Option<&str>) -> Chunk<'static> {
        let sspc = vec![0u8; 212];
        let mut opti = vec![0u8; 320];
        let mut pin = vec![Chunk::data(*b"sspc", sspc)];
        match path {
            Some(p) => {
                opti[58..58 + 4].copy_from_slice(b"clip");
                pin.push(Chunk::data(*b"opti", opti));
                let alas = format!(r#"{{"fullpath":"{p}","target_is_folder":false}}"#);
                pin.push(Chunk::list(*b"Als2", vec![Chunk::data(*b"alas", alas.into_bytes())]));
            }
            None => {
                opti[..4].copy_from_slice(b"Soli");
                pin.push(Chunk::data(*b"opti", opti));
            }
        }
        Chunk::list(*b"Item", vec![idta(7, id), Chunk::list(*b"Pin ", pin)])
    }

    fn project(items: Vec<Chunk<'static>>) -> Vec<u8> {
        let mut head = vec![0u8; 20];
        head[4..8].copy_from_slice(&0x0f0b_0661u32.to_be_bytes());
        let file = AepFile {
            chunks: vec![Chunk::data(*b"head", head), Chunk::data(*b"svap", 0x0f11_0e02u32.to_be_bytes().to_vec()), Chunk::list(*b"Fold", items)],
            trailer: Vec::new().into(),
        };
        craft_aep::write(&file).unwrap()
    }

    #[test]
    fn compositions_footage_and_versions() {
        let dir = std::env::temp_dir().join(format!("filescape-aep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("clip.mov");
        std::fs::write(&clip, [0u8; 100]).unwrap();
        let folder = Chunk::list(*b"Item", vec![idta(1, 9), utf8("Footage"), Chunk::list(*b"Sfdr", vec![footage(3, Some(&clip.display().to_string().replace('\\', "/"))), footage(4, None)])]);
        let data = project(vec![comp("Main", 2, 3), folder, footage(5, Some("/nowhere/gone.wav"))]);
        let path = dir.join("Promo.aep");
        std::fs::write(&path, &data).unwrap();
        let s = read(&path).unwrap();
        let f = |k: &str| s.facts.iter().find(|x| x.0 == k).map(|x| x.1.as_str());
        assert_eq!(f("Saved with"), Some("After Effects 26.2.1"));
        assert_eq!(f("Saved for"), Some("After Effects 25.6.0"));
        assert_eq!(f("Compositions"), Some("1 composition"));
        assert_eq!(f("Composition"), Some("Main: 1920 × 1080, 25 fps, 0:05, 3 layers"));
        assert_eq!(f("Footage"), Some("2 files and 1 solid"));
        assert_eq!(f("Folders"), Some("1"));
        let r = s.refs.as_ref().unwrap();
        assert_eq!(r.items.len(), 2);
        assert_eq!(r.count(crate::projrefs::State::Missing), 1);
        assert_eq!(r.total, 100);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn other_files_are_refused() {
        assert!(details(b"RIFF\0\0\0\x04WAVE").is_err());
        assert!(details(b"").is_err());
        let mut cut = project(vec![comp("Main", 2, 1)]);
        cut.truncate(cut.len() / 2);
        let _ = details(&cut); // an error or what could be read, never a panic
    }
}
