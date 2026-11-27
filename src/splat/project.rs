//! Gaussian splat training projects: the folders trainers write and read, told apart by their
//! layout, with what's in them and the scenes saved as training went on (the checkpoint
//! timeline).
//!
//! Everything comes from names, sizes and file headers. Training state saved by PyTorch
//! (`.pth`, `.pt`, `.ckpt`) is a Python pickle, which can run code when loaded, so it's never
//! opened: only its name and size are used.

use super::ply;
use crate::help::doc::{Block, Span};
use std::path::{Path, PathBuf};

/// Which trainer's layout a folder has.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    /// graphdeco-inria/gaussian-splatting and the many trainers built on it: `cfg_args`,
    /// `cameras.json`, `input.ply` and `point_cloud/iteration_N/point_cloud.ply`.
    Inria,
    /// A Nerfstudio run (`outputs/<data>/<method>/<time>/`): `config.yml` and
    /// `nerfstudio_models/step-N.ckpt`.
    NerfstudioRun,
    /// EA SEED's PERF-GS run: `outputs/train/<time>/output.ply`.
    PerfGs,
    /// Photos with COLMAP's camera solve (`sparse/0`), ready to train from.
    Colmap,
    /// Photos with Nerfstudio's `transforms.json`, ready to train from.
    NerfstudioData,
}

impl Layout {
    pub fn name(self) -> &'static str {
        match self {
            Layout::Inria => "Gaussian splat training run (3DGS layout)",
            Layout::NerfstudioRun => "Nerfstudio training run",
            Layout::PerfGs => "PERF-GS training run",
            Layout::Colmap => "Capture for training (COLMAP camera solve)",
            Layout::NerfstudioData => "Capture for training (Nerfstudio transforms.json)",
        }
    }
}

fn is_file(p: PathBuf) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file())
}

fn is_dir(p: PathBuf) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
}

/// The layout of `dir`, from a few looks at what's in it (cheap enough for every frame).
pub fn layout(dir: &Path) -> Option<Layout> {
    if is_file(dir.join("cfg_args")) && is_dir(dir.join("point_cloud")) {
        return Some(Layout::Inria);
    }
    if is_file(dir.join("config.yml")) && is_dir(dir.join("nerfstudio_models")) {
        return Some(Layout::NerfstudioRun);
    }
    if is_file(dir.join("output.ply"))
        && dir
            .parent()
            .and_then(|p| p.file_name())
            .is_some_and(|n| n == "train")
    {
        return Some(Layout::PerfGs);
    }
    if ["cameras.bin", "cameras.txt"]
        .iter()
        .any(|f| is_file(dir.join("sparse").join("0").join(f)))
    {
        return Some(Layout::Colmap);
    }
    if is_file(dir.join("transforms.json")) {
        return Some(Layout::NerfstudioData);
    }
    None
}

/// A scene or training state saved during training.
#[derive(Clone, Debug, PartialEq)]
pub struct Checkpoint {
    /// The training step it was saved at.
    pub step: u64,
    /// Relative to the project folder.
    pub path: String,
    pub bytes: u64,
    /// Splats in it, for scenes; `None` for training state (never opened).
    pub splats: Option<u64>,
}

/// What a project folder holds.
#[derive(Debug, Default)]
pub struct Project {
    pub layout: Option<Layout>,
    /// Scenes and training state, oldest first.
    pub timeline: Vec<Checkpoint>,
    /// The scene to open: the last one saved.
    pub newest: Option<PathBuf>,
    pub cameras: Option<u64>,
    /// Points the camera solve found (the starting point cloud).
    pub points: Option<u64>,
    /// Settings the trainer wrote down (from `cfg_args` or `config.yml`).
    pub settings: Vec<(String, String)>,
    /// (part, bytes, files, what it is), biggest first.
    pub parts: Vec<(&'static str, u64, usize, &'static str)>,
    /// The walk stopped early: sizes are at least these.
    pub partial: bool,
}

/// Files looked at at most when adding up a project's parts.
const WALK_MAX: usize = 200_000;
/// Text files read at most this far (settings, camera lists).
const TEXT_MAX: u64 = 32 << 20;

/// The first run of digits in a name, as a number: `iteration_30000` → 30000.
fn digits(name: &str) -> Option<u64> {
    let start = name.find(|c: char| c.is_ascii_digit())?;
    let end = name[start..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(name.len(), |e| start + e);
    name[start..end].parse().ok()
}

fn read_text(p: &Path) -> Option<String> {
    use std::io::Read;
    let mut s = String::new();
    std::fs::File::open(p)
        .ok()?
        .take(TEXT_MAX)
        .read_to_string(&mut s)
        .ok()?;
    Some(s)
}

fn size(p: &Path) -> u64 {
    std::fs::metadata(p).map_or(0, |m| m.len())
}

fn ply_count(p: &Path) -> Option<u64> {
    ply::Header::read(p).ok().map(|h| h.vertices())
}

/// The settings in INRIA's `cfg_args`: `Namespace(sh_degree=3, source_path='/data/garden', …)`.
pub fn parse_cfg_args(text: &str) -> Vec<(String, String)> {
    let body = text
        .trim()
        .strip_prefix("Namespace(")
        .and_then(|b| b.strip_suffix(')'))
        .unwrap_or("");
    let mut out = Vec::new();
    let (mut depth, mut quote, mut start) = (0i32, None::<char>, 0usize);
    let mut push = |part: &str| {
        if let Some((k, v)) = part.split_once('=') {
            out.push((
                k.trim().to_string(),
                v.trim().trim_matches(|c| c == '\'' || c == '"').to_string(),
            ));
        }
    };
    for (i, c) in body.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '(' | '[' | '{') => depth += 1,
            (None, ')' | ']' | '}') => depth -= 1,
            (None, ',') if depth == 0 => {
                push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    push(&body[start..]);
    out
}

/// The first number in a COLMAP binary file: how many cameras, images or points it holds.
fn colmap_count(p: &Path) -> Option<u64> {
    use std::io::Read;
    let mut b = [0u8; 8];
    std::fs::File::open(p).ok()?.read_exact(&mut b).ok()?;
    Some(u64::from_le_bytes(b))
}

/// Read what a project folder holds. `None` when it isn't one.
pub fn read(dir: &Path) -> Option<Project> {
    let layout = layout(dir)?;
    let mut p = Project {
        layout: Some(layout),
        ..Default::default()
    };
    match layout {
        Layout::Inria => {
            if let Ok(rd) = std::fs::read_dir(dir.join("point_cloud")) {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    let ply = e.path().join("point_cloud.ply");
                    if let (Some(step), true) = (digits(&name), ply.is_file()) {
                        p.timeline.push(Checkpoint {
                            step,
                            path: format!("point_cloud/{name}/point_cloud.ply"),
                            bytes: size(&ply),
                            splats: ply_count(&ply),
                        });
                    }
                }
            }
            if let Ok(rd) = std::fs::read_dir(dir) {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    let lower = name.to_ascii_lowercase();
                    if lower.starts_with("chkpnt")
                        && (lower.ends_with(".pth") || lower.ends_with(".pt"))
                    {
                        if let Some(step) = digits(&name) {
                            p.timeline.push(Checkpoint {
                                step,
                                path: name,
                                bytes: size(&e.path()),
                                splats: None,
                            });
                        }
                    }
                }
            }
            if let Some(t) = read_text(&dir.join("cfg_args")) {
                p.settings = parse_cfg_args(&t)
                    .into_iter()
                    .filter(|(k, _)| {
                        [
                            "source_path",
                            "sh_degree",
                            "images",
                            "resolution",
                            "white_background",
                            "eval",
                            "antialiasing",
                        ]
                        .contains(&k.as_str())
                    })
                    .collect();
            }
            if let Some(Ok(serde_json::Value::Array(cams))) =
                read_text(&dir.join("cameras.json")).map(|t| serde_json::from_str(&t))
            {
                p.cameras = Some(cams.len() as u64);
            }
            p.points = ply_count(&dir.join("input.ply"));
        }
        Layout::NerfstudioRun => {
            if let Ok(rd) = std::fs::read_dir(dir.join("nerfstudio_models")) {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.ends_with(".ckpt") {
                        if let Some(step) = digits(&name) {
                            p.timeline.push(Checkpoint {
                                step,
                                path: format!("nerfstudio_models/{name}"),
                                bytes: size(&e.path()),
                                splats: None,
                            });
                        }
                    }
                }
            }
            if let Some(t) = read_text(&dir.join("config.yml")) {
                for line in t.lines() {
                    let line = line.trim();
                    for key in [
                        "method_name",
                        "experiment_name",
                        "data",
                        "max_num_iterations",
                    ] {
                        if let Some(v) = line.strip_prefix(key).and_then(|r| r.strip_prefix(':')) {
                            let v = v.trim();
                            if !v.is_empty()
                                && !v.starts_with('!')
                                && !p.settings.iter().any(|(k, _)| k == key)
                            {
                                p.settings.push((
                                    key.to_string(),
                                    v.trim_matches(|c| c == '\'' || c == '"').to_string(),
                                ));
                            }
                        }
                    }
                }
            }
        }
        Layout::PerfGs => {
            let out = dir.join("output.ply");
            p.timeline.push(Checkpoint {
                step: 0,
                path: "output.ply".into(),
                bytes: size(&out),
                splats: ply_count(&out),
            });
        }
        Layout::Colmap => {}
        Layout::NerfstudioData => {
            if let Some(Ok(v)) = read_text(&dir.join("transforms.json"))
                .map(|t| serde_json::from_str::<serde_json::Value>(&t))
            {
                p.cameras = v
                    .get("frames")
                    .and_then(|f| f.as_array())
                    .map(|f| f.len() as u64);
                if let Some(m) = v.get("camera_model").and_then(|m| m.as_str()) {
                    p.settings.push(("camera_model".into(), m.to_string()));
                }
            }
        }
    }
    // A COLMAP solve, in a capture or beside a run's data.
    let sparse = dir.join("sparse").join("0");
    if p.cameras.is_none() {
        p.cameras = colmap_count(&sparse.join("images.bin"));
    }
    if p.points.is_none() {
        p.points = colmap_count(&sparse.join("points3D.bin"));
    }
    p.timeline.sort_by(|a, b| {
        a.step
            .cmp(&b.step)
            .then(a.splats.is_none().cmp(&b.splats.is_none()))
    });
    p.newest = p
        .timeline
        .iter()
        .rev()
        .find(|c| c.splats.is_some())
        .map(|c| dir.join(&c.path));
    let (parts, partial) = parts(dir);
    p.parts = parts;
    p.partial = partial;
    Some(p)
}

/// What a file in a project is, from where it is and its name.
fn part_of(rel: &Path) -> (&'static str, &'static str) {
    let s = rel
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    let top = s.split('/').next().unwrap_or("");
    let name = s.rsplit('/').next().unwrap_or("");
    if name.ends_with(".pth") || name.ends_with(".pt") || name.ends_with(".ckpt") {
        (
            "Training state",
            "Saved to resume training; not needed to view the scenes",
        )
    } else if top == "point_cloud"
        || name == "output.ply"
        || (top == "exports" && name.ends_with(".ply"))
    {
        (
            "Trained scenes",
            "The splat scenes saved as training went on",
        )
    } else if top.starts_with("images") || top == "input" || top == "distorted" {
        ("Photos", "The capture's photos (and smaller copies)")
    } else if top == "sparse"
        || top == "stereo"
        || name == "database.db"
        || name == "transforms.json"
        || name == "cameras.json"
        || name == "input.ply"
    {
        (
            "Camera solve",
            "Where each photo was taken from, and the starting points",
        )
    } else if top == "events" || name.starts_with("events.out") || name.ends_with(".log") {
        ("Logs", "Training logs")
    } else {
        ("Other", "Settings and everything else")
    }
}

/// The project's parts and their sizes, biggest first, and whether the walk stopped early.
fn parts(dir: &Path) -> (Vec<(&'static str, u64, usize, &'static str)>, bool) {
    let mut acc: Vec<(&'static str, u64, usize, &'static str)> = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    let mut seen = 0usize;
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                stack.push(e.path());
            } else if md.is_file() {
                let rel = e
                    .path()
                    .strip_prefix(dir)
                    .map(Path::to_path_buf)
                    .unwrap_or_default();
                let (part, about) = part_of(&rel);
                match acc.iter_mut().find(|a| a.0 == part) {
                    Some(a) => {
                        a.1 += md.len();
                        a.2 += 1;
                    }
                    None => acc.push((part, md.len(), 1, about)),
                }
                seen += 1;
                if seen >= WALK_MAX {
                    acc.sort_by(|a, b| b.1.cmp(&a.1));
                    return (acc, true);
                }
            }
        }
    }
    acc.sort_by(|a, b| b.1.cmp(&a.1));
    (acc, false)
}

fn plain(t: &str) -> Vec<Span> {
    vec![Span {
        text: t.to_string(),
        ..Default::default()
    }]
}

/// The id of the first heading of a project's preview, so the preview can tell it apart.
pub const PREVIEW_ID: &str = "splat-project";

/// The preview of a project folder: the details (free) and the blocks, whose first two (the
/// heading and the summary) are free and the rest (breakdown and timeline) Pro.
pub fn describe(dir: &Path, p: &Project) -> (Vec<Block>, Vec<(&'static str, String)>) {
    use crate::scan::human;
    let group = |n: u64| crate::inspect::group(n);
    let layout = p.layout.map_or("Gaussian splat project", |l| l.name());
    let mut facts: Vec<(&'static str, String)> = vec![("Holds", layout.to_string())];
    let scenes: Vec<&Checkpoint> = p.timeline.iter().filter(|c| c.splats.is_some()).collect();
    let states = p.timeline.iter().filter(|c| c.splats.is_none()).count();
    if let Some(last) = scenes.last() {
        let at = if p.layout == Some(Layout::PerfGs) {
            String::new()
        } else {
            format!(" (step {})", group(last.step))
        };
        facts.push((
            "Newest scene",
            format!(
                "{}{at}: {} splats",
                last.path,
                group(last.splats.unwrap_or(0))
            ),
        ));
    }
    if scenes.len() > 1 {
        facts.push(("Saved scenes", scenes.len().to_string()));
    }
    if states > 0 {
        facts.push((
            "Training state",
            format!("{states} saved (never opened by Filescape)"),
        ));
    }
    if let Some(c) = p.cameras {
        facts.push(("Photos used", group(c)));
    }
    if let Some(n) = p.points {
        facts.push(("Starting points", group(n)));
    }
    if let Some((_, v)) = p
        .settings
        .iter()
        .find(|(k, _)| k == "source_path" || k == "data")
    {
        facts.push(("Trained from", v.clone()));
    }
    let title = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Project".into());
    let mut blocks = vec![Block::Heading {
        level: 1,
        spans: plain(&title),
        id: PREVIEW_ID.into(),
    }];
    let total: u64 = p.parts.iter().map(|x| x.1).sum();
    let what = match p.layout {
        Some(Layout::Inria) => "A Gaussian splat training run in the 3DGS layout",
        Some(Layout::NerfstudioRun) => "A Nerfstudio training run",
        Some(Layout::PerfGs) => "A PERF-GS training run",
        Some(Layout::Colmap) => "Photos and their COLMAP camera solve, ready to train a scene from",
        Some(Layout::NerfstudioData) => {
            "Photos and Nerfstudio's transforms.json, ready to train a scene from"
        }
        None => "A Gaussian splat project",
    };
    let mut line = format!(
        "{what}{}.",
        if total > 0 {
            format!(", {} in all", human(total))
        } else {
            String::new()
        }
    );
    if !scenes.is_empty() {
        line.push_str(&format!(
            " {} saved {}.",
            scenes.len(),
            if scenes.len() == 1 { "scene" } else { "scenes" }
        ));
    }
    if p.partial {
        line.push_str(" It's very big, so sizes are at least these.");
    }
    blocks.push(Block::Para(plain(&line)));
    if !p.parts.is_empty() {
        blocks.push(Block::Heading {
            level: 2,
            spans: plain("What it takes"),
            id: String::new(),
        });
        let rows = p
            .parts
            .iter()
            .map(|(k, b, n, about)| {
                vec![
                    plain(k),
                    plain(&human(*b)),
                    plain(&format!(
                        "{} ({} {})",
                        about,
                        group(*n as u64),
                        if *n == 1 { "file" } else { "files" }
                    )),
                ]
            })
            .collect();
        blocks.push(Block::Table {
            head: vec![plain("Part"), plain("Size"), plain("What it is")],
            rows,
        });
    }
    if !p.timeline.is_empty() && p.layout != Some(Layout::PerfGs) {
        blocks.push(Block::Heading {
            level: 2,
            spans: plain("Timeline"),
            id: String::new(),
        });
        let mut prev: Option<u64> = None;
        let rows = p
            .timeline
            .iter()
            .map(|c| {
                let what = match c.splats {
                    Some(n) => {
                        let change = prev
                            .map(|q| {
                                if n >= q {
                                    format!(" (+{})", group(n - q))
                                } else {
                                    format!(" (−{})", group(q - n))
                                }
                            })
                            .unwrap_or_default();
                        prev = Some(n);
                        format!("{} splats{change}", group(n))
                    }
                    None => "Training state".to_string(),
                };
                vec![
                    plain(&group(c.step)),
                    plain(&what),
                    plain(&human(c.bytes)),
                    vec![Span {
                        text: c.path.clone(),
                        code: true,
                        ..Default::default()
                    }],
                ]
            })
            .collect();
        blocks.push(Block::Table {
            head: vec![plain("Step"), plain("Holds"), plain("Size"), plain("File")],
            rows,
        });
    }
    if !p.settings.is_empty() {
        blocks.push(Block::Heading {
            level: 2,
            spans: plain("Settings"),
            id: String::new(),
        });
        let rows = p
            .settings
            .iter()
            .map(|(k, v)| {
                vec![
                    vec![Span {
                        text: k.clone(),
                        code: true,
                        ..Default::default()
                    }],
                    plain(v),
                ]
            })
            .collect();
        blocks.push(Block::Table {
            head: vec![plain("Setting"), plain("Value")],
            rows,
        });
    }
    (blocks, facts)
}

/// The preview's blocks as someone without Filescape Professional sees them: the heading and
/// summary, then a note.
pub fn free_blocks(blocks: &[Block]) -> Vec<Block> {
    let mut out: Vec<Block> = blocks.iter().take(2).cloned().collect();
    if blocks.len() > 2 {
        out.push(Block::Para(plain("The breakdown of what it takes and the timeline of saved scenes are part of Filescape Professional. Opening and viewing its scenes is free.")));
    }
    out
}
