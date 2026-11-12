//! Optional detection of numbered file sequences (render frames, simulation caches):
//! `shot010_comp.1001.exr`, `shot010_comp.1002.exr`, … become one entry,
//! `shot010_comp.####.exr — 3 frames`. Only run when asked for.
//!
//! Rules, chosen so unrelated numbered files don't get lumped together:
//! * Only files in the same folder are grouped.
//! * The frame number is the last run of digits, right before the extension
//!   (`name.1001.exr`, `name_1001.exr`, `name1001.exr`); everything before it must be identical,
//!   so `shot010_v1.1001.exr` and `shot010_v2.1001.exr` are two sequences, and numbered names
//!   like `rock_12_diffuse.tif` aren't treated as frames.
//! * Padded numbers (with leading zeros) are grouped by digit count; unpadded ones of varying
//!   length (`a.9.png`, `a.10.png`) form one sequence.
//! * There must be at least `min_frames` frames, and the frames must be reasonably dense on
//!   their step (a camera's timestamped photos aren't a sequence).

use crate::scan::Node;
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    /// The folder holding it, as a path of child indices from the scan root.
    pub folder: Vec<u32>,
    pub prefix: String,
    pub suffix: String,
    /// Digit count for padded sequences; None when the numbers aren't padded.
    pub padding: Option<u8>,
    pub frames: u64,
    pub first: u64,
    pub last: u64,
    /// Frame step (1 normally; 2 when rendered on twos, …).
    pub step: u64,
    pub bytes: u64,
    /// Frames missing between first and last, on the step.
    pub missing: u64,
    /// The first few runs of missing frames, inclusive.
    pub gaps: Vec<(u64, u64)>,
}

impl Sequence {
    /// `shot010_comp.####.exr`, or `a.{frame}.png` when unpadded.
    pub fn display(&self) -> String {
        format!("{}{}{}", self.prefix, self.token(), self.suffix)
    }

    /// A Custom filter pattern matching exactly this sequence's file names.
    pub fn pattern(&self) -> String {
        format!("{}{}{}", escape(&self.prefix), self.token(), escape(&self.suffix))
    }

    fn token(&self) -> String {
        match self.padding {
            Some(w) => "#".repeat(w as usize),
            None => "{frame}".into(),
        }
    }

    pub fn extension(&self) -> &str {
        self.suffix.rsplit_once('.').map_or("", |(_, e)| e)
    }

    pub fn average(&self) -> u64 {
        self.bytes / self.frames.max(1)
    }

    /// "1001-1100", "1001-1100 (+2)" for a step of 2.
    pub fn range_text(&self) -> String {
        let step = if self.step > 1 { format!(" by {}", self.step) } else { String::new() };
        format!("{}–{}{step}", self.first, self.last)
    }

    /// "1004, 1010–1012, …" for the gaps.
    pub fn gaps_text(&self) -> String {
        let mut s: Vec<String> =
            self.gaps.iter().map(|&(a, b)| if a == b { a.to_string() } else { format!("{a}–{b}") }).collect();
        if (self.gaps.len() as u64) < self.missing && s.len() == MAX_GAPS {
            s.push("…".into());
        }
        s.join(", ")
    }
}

const MAX_GAPS: usize = 20;

/// Make a literal name part safe to use in a pattern.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        if matches!(c, '\\' | '*' | '?' | '#' | ',' | '[' | '{') || (i == 0 && c == '!') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Find the sequences under `root`, largest first. None if cancelled.
pub fn detect(root: &Node, min_frames: usize, cancel: &AtomicBool) -> Option<Vec<Sequence>> {
    let mut out = Vec::new();
    walk(root, &[], min_frames.max(2), cancel, &mut out)?;
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.display().cmp(&b.display())));
    Some(out)
}

fn walk(dir: &Node, path: &[u32], min: usize, cancel: &AtomicBool, out: &mut Vec<Sequence>) -> Option<()> {
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    out.extend(in_folder(dir, path, min));
    let subdirs: Vec<(u32, &Node)> =
        dir.children.iter().enumerate().filter(|(_, c)| c.is_dir).map(|(i, c)| (i as u32, c)).collect();
    let found: Vec<Vec<Sequence>> = subdirs
        .par_iter()
        .map(|&(i, c)| {
            let mut p = path.to_vec();
            p.push(i);
            let mut v = Vec::new();
            walk(c, &p, min, cancel, &mut v).map(|_| v)
        })
        .collect::<Option<_>>()?;
    out.extend(found.into_iter().flatten());
    Some(())
}

/// One numbered file: (value, digit count, has leading zero, size).
type Frame = (u64, u8, bool, u64);

fn in_folder(dir: &Node, path: &[u32], min: usize) -> Vec<Sequence> {
    let files = dir.children.iter().filter(|c| !c.is_dir);
    if files.clone().count() < min {
        return Vec::new();
    }
    let mut groups: HashMap<(&str, &str), Vec<Frame>> = HashMap::new();
    for f in files {
        if let Some((pre, digits, suf)) = last_number(&f.name) {
            if digits.len() > 18 || !is_extension(suf) {
                continue;
            }
            let v: u64 = digits.parse().unwrap_or(0);
            groups.entry((pre, suf)).or_default().push((v, digits.len() as u8, digits.len() > 1 && digits.starts_with('0'), f.size));
        }
    }
    let mut out = Vec::new();
    for ((pre, suf), frames) in groups {
        if frames.len() < min {
            continue;
        }
        let same_width = frames.iter().all(|f| f.1 == frames[0].1);
        let padded = frames.iter().any(|f| f.2);
        if same_width {
            out.extend(build(path, pre, suf, Some(frames[0].1), &frames, min));
        } else if !padded {
            out.extend(build(path, pre, suf, None, &frames, min));
        } else {
            // Mixed: padded frames of different widths are different sequences.
            let mut by_width: HashMap<u8, Vec<Frame>> = HashMap::new();
            for f in frames {
                by_width.entry(f.1).or_default().push(f);
            }
            for (w, fs) in by_width {
                if fs.len() >= min {
                    out.extend(build(path, pre, suf, Some(w), &fs, min));
                }
            }
        }
    }
    out
}

fn build(path: &[u32], pre: &str, suf: &str, padding: Option<u8>, frames: &[Frame], min: usize) -> Option<Sequence> {
    let mut values: Vec<u64> = frames.iter().map(|f| f.0).collect();
    values.sort_unstable();
    values.dedup();
    if values.len() < min {
        return None;
    }
    let step = values.windows(2).map(|w| w[1] - w[0]).fold(0, gcd).max(1);
    let (first, last) = (values[0], *values.last().unwrap());
    let expected = (last - first) / step + 1;
    // Mostly holes, or a huge step: numbered files, but not a sequence (timestamps, ids).
    if expected > values.len() as u64 * 4 || step > 100 {
        return None;
    }
    let mut gaps = Vec::new();
    for w in values.windows(2) {
        if w[1] - w[0] > step && gaps.len() < MAX_GAPS {
            gaps.push((w[0] + step, w[1] - step));
        }
    }
    Some(Sequence {
        folder: path.to_vec(),
        prefix: pre.to_string(),
        suffix: suf.to_string(),
        padding,
        frames: frames.len() as u64,
        first,
        last,
        step,
        bytes: frames.iter().map(|f| f.3).sum(),
        missing: expected - values.len() as u64,
        gaps,
    })
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// ".exr", ".bgeo", ".tar" (empty too: a bare `frame0001`), but not "_diffuse.tif".
fn is_extension(s: &str) -> bool {
    s.is_empty() || s.strip_prefix('.').is_some_and(|e| !e.is_empty() && e.len() <= 10 && e.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// Split a name around its last run of ASCII digits.
fn last_number(name: &str) -> Option<(&str, &str, &str)> {
    let b = name.as_bytes();
    let end = b.iter().rposition(u8::is_ascii_digit)? + 1;
    let start = b[..end].iter().rposition(|c| !c.is_ascii_digit()).map_or(0, |i| i + 1);
    Some((&name[..start], &name[start..end], &name[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::{Filter, Options};

    fn dir(name: &str, files: &[&str]) -> Node {
        let children = files
            .iter()
            .map(|f| Node { name: (*f).into(), size: 10, files: 1, modified: 0, is_dir: false, children: Vec::new() })
            .collect::<Vec<_>>();
        Node { name: name.into(), size: 10 * files.len() as u64, files: files.len() as u64, modified: 0, is_dir: true, children }
    }

    fn detect_in(files: &[&str]) -> Vec<Sequence> {
        detect(&dir("d", files), 3, &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn groups_a_render() {
        let s = detect_in(&["shot010_comp.1001.exr", "shot010_comp.1002.exr", "shot010_comp.1003.exr", "notes.txt"]);
        assert_eq!(s.len(), 1);
        let s = &s[0];
        assert_eq!(s.display(), "shot010_comp.####.exr");
        assert_eq!((s.frames, s.first, s.last, s.missing, s.bytes, s.average()), (3, 1001, 1003, 0, 30, 10));
        assert_eq!(s.extension(), "exr");
        assert_eq!(s.padding, Some(4));
        // Its pattern finds exactly its own files.
        let f = Filter::compile(&s.pattern(), Options::default()).unwrap();
        assert!(f.matches("shot010_comp.1002.exr"));
        assert!(!f.matches("shot010_comp.10002.exr"));
        assert!(!f.matches("shot011_comp.1002.exr"));
    }

    #[test]
    fn missing_frames_and_steps() {
        let s = detect_in(&["a.0001.png", "a.0002.png", "a.0005.png", "a.0006.png", "a.0008.png"]);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].missing, s[0].gaps.clone()), (3, vec![(3, 4), (7, 7)]));
        assert_eq!(s[0].gaps_text(), "3–4, 7");
        let s = detect_in(&["b.0010.png", "b.0012.png", "b.0014.png", "b.0018.png"]);
        assert_eq!((s[0].step, s[0].missing), (2, 1));
    }

    #[test]
    fn keeps_unrelated_files_apart() {
        // Different versions, different extensions, too few frames, timestamps.
        let s = detect_in(&[
            "sh_v1.1001.exr",
            "sh_v1.1002.exr",
            "sh_v1.1003.exr",
            "sh_v2.1001.exr",
            "sh_v2.1002.exr",
            "sh_v2.1003.exr",
            "sh_v1.1001.png",
            "sh_v1.1002.png",
            "IMG_20240101_093012.jpg",
            "IMG_20240101_101544.jpg",
            "IMG_20240101_174405.jpg",
            "rock_1_diffuse.tif",
            "rock_2_diffuse.tif",
            "rock_3_diffuse.tif",
        ]);
        let mut names: Vec<String> = s.iter().map(|s| s.display()).collect();
        names.sort();
        assert_eq!(names, ["sh_v1.####.exr", "sh_v2.####.exr"]);
        // Same names in different folders are different sequences.
        let root = Node {
            name: "r".into(),
            size: 60,
            files: 6,
            modified: 0,
            is_dir: true,
            children: vec![dir("x", &["f.1.png", "f.2.png", "f.3.png"]), dir("y", &["f.1.png", "f.2.png", "f.3.png"])],
        };
        let s = detect(&root, 3, &AtomicBool::new(false)).unwrap();
        assert_eq!(s.len(), 2);
        assert_ne!(s[0].folder, s[1].folder);
    }

    #[test]
    fn padding_widths() {
        // Unpadded numbers of varying length are one sequence.
        let s = detect_in(&["a.8.png", "a.9.png", "a.10.png", "a.11.png"]);
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].padding, s[0].display()), (None, "a.{frame}.png".to_string()));
        // Padded to different widths: separate sequences.
        let mut s = detect_in(&["r.0001.exr", "r.0002.exr", "r.0003.exr", "r.000001.exr", "r.000002.exr", "r.000003.exr"]);
        s.sort_by_key(|s| s.padding);
        assert_eq!(s.iter().map(|s| s.display()).collect::<Vec<_>>(), ["r.####.exr", "r.######.exr"]);
    }

    #[test]
    fn escapes_special_characters() {
        let s = detect_in(&["[v2] take#1_001.wav", "[v2] take#1_002.wav", "[v2] take#1_003.wav"]);
        assert_eq!(s.len(), 1);
        let f = Filter::compile(&s[0].pattern(), Options::default()).unwrap();
        assert!(f.matches("[v2] take#1_002.wav"));
        assert!(!f.matches("[v2] take#2_002.wav"));
        assert_eq!(escape("!a,b"), "\\!a\\,b");
    }
}
