//! References in Ask AI's answers, such as `[config.yaml:12]`, checked against what the model was
//! actually given: the file must be one of the items asked about (or inside a folder asked about),
//! and a line must be one the model was sent. Only references that pass are offered as links; the
//! rest are shown as not found, so a made-up file or line never looks like a fact.

use std::path::{Path, PathBuf};

/// How far a reference checked out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// The file is real and among what was asked about, and the line (if any) was sent.
    Verified,
    /// The file is real and among what was asked about, but the line can't be checked: the
    /// file's contents weren't sent, or the line is past what was sent.
    FileOnly,
    /// No such file among what was asked about.
    NotFound,
}

#[derive(Clone, Debug)]
pub struct Ref {
    /// As the answer wrote it, without the brackets.
    pub text: String,
    pub path: Option<PathBuf>,
    /// First and last line, from 1.
    pub lines: Option<(usize, usize)>,
    pub check: Check,
    /// The line itself, read from the file, for a verified line reference.
    pub quote: Option<String>,
    /// Why it isn't verified, for the user.
    pub why: String,
}

/// The references in `answer`: bracketed names of files, with an optional `:line` or
/// `:first-last`. `asked` are the files and folders the question was about; `sent` the files
/// whose numbered lines were sent, with how many.
pub fn find(answer: &str, asked: &[PathBuf], sent: &[(PathBuf, Vec<(usize, usize)>)]) -> Vec<Ref> {
    let mut out: Vec<Ref> = Vec::new();
    let mut rest = answer;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else { break };
        let inner = after[..close].trim().trim_matches('`');
        let next = &after[close + 1..];
        rest = next;
        // A Markdown link's text, a footnote number or a checkbox isn't a file reference.
        if next.starts_with('(') || inner.is_empty() || inner.len() > 200 || inner.contains('\n') {
            continue;
        }
        if out.iter().any(|o| o.text == inner) || out.len() >= 30 {
            continue;
        }
        // Asked about one file, models often cite its lines alone: [3], [63-78], [25-26, 72].
        if let [(only, _)] = sent
            && let Some(ranges) = bare_lines(inner)
        {
            let name = only.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            for (a, b) in ranges {
                let text = if a == b { format!("{name}:{a}") } else { format!("{name}:{a}-{b}") };
                if !out.iter().any(|o| o.text == text) {
                    out.push(check_at(text, Some(only.clone()), Some((a, b)), sent));
                }
            }
            continue;
        }
        match parse(inner) {
            Some(r) => out.push(check(inner, r, asked, sent)),
            // A name without an extension (LICENSE, Makefile) counts only when it's really there,
            // so ordinary words in brackets aren't flagged as made-up files.
            None => {
                if let Some(r) = parse_bare(inner).map(|r| check(inner, r, asked, sent))
                    && r.check != Check::NotFound
                {
                    out.push(r);
                }
            }
        }
    }
    out
}

/// A name and its lines, if `s` looks like a file reference: the name has an extension or a
/// slash. "notes.txt", "notes.txt:12", "src/app.rs:10-20", "notes.txt line 12".
fn parse(s: &str) -> Option<(String, Option<(usize, usize)>)> {
    let (name, lines) = match s.rsplit_once(':').or_else(|| s.rsplit_once(" line ")).or_else(|| s.rsplit_once(", line ")) {
        Some((n, l)) if !l.trim().is_empty() && l.trim().chars().all(|c| c.is_ascii_digit() || c == '-' || c == '–' || c == ' ') => {
            let l = l.trim();
            let mut parts = l.split(['-', '–']).map(|x| x.trim().parse::<usize>());
            let a = parts.next()?.ok()?;
            let b = match parts.next() {
                Some(b) => b.ok()?,
                None => a,
            };
            (n.trim(), (a >= 1 && b >= a).then_some((a, b)))
        }
        _ => (s, None),
    };
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let looks = name.contains(['/', '\\']) || base.rfind('.').is_some_and(|i| i > 0 && i + 1 < base.len() && base[i + 1..].chars().all(|c| c.is_ascii_alphanumeric()));
    (looks && name.len() < 120).then(|| (name.to_string(), lines))
}

/// Like [`parse`], for a name without an extension: one word, with an optional line.
fn parse_bare(s: &str) -> Option<(String, Option<(usize, usize)>)> {
    let (name, lines) = match s.rsplit_once(':') {
        Some((n, l)) => (n, Some(l.trim().parse::<usize>().ok().filter(|&l| l >= 1).map(|l| (l, l))?)),
        None => (s, None),
    };
    (!name.is_empty() && name.len() < 64 && !name.contains(char::is_whitespace) && !name.chars().all(|c| c.is_ascii_digit())).then(|| (name.to_string(), lines))
}

/// Line numbers alone, as ranges: "3", "63-78", "25-26, 72", "lines 4 and 9". None if `s` is
/// anything else.
fn bare_lines(s: &str) -> Option<Vec<(usize, usize)>> {
    let s = s.trim();
    let s = s.strip_prefix("lines").or_else(|| s.strip_prefix("line")).or_else(|| s.strip_prefix("L")).unwrap_or(s);
    let mut out = Vec::new();
    for part in s.split([',', ';']).flat_map(|p| p.split(" and ")) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let mut ends = part.split(['-', '–']).map(|x| x.trim().trim_start_matches('L').parse::<usize>());
        let a = ends.next()?.ok()?;
        let b = match ends.next() {
            Some(b) => b.ok()?,
            None => a,
        };
        if ends.next().is_some() || a == 0 || b < a {
            return None;
        }
        out.push((a, b));
    }
    (!out.is_empty() && out.len() <= 10).then_some(out)
}

fn check(text: &str, (name, lines): (String, Option<(usize, usize)>), asked: &[PathBuf], sent: &[(PathBuf, Vec<(usize, usize)>)]) -> Ref {
    check_at(text.to_string(), resolve(&name, asked), lines, sent)
}

fn check_at(text: String, path: Option<PathBuf>, lines: Option<(usize, usize)>, sent: &[(PathBuf, Vec<(usize, usize)>)]) -> Ref {
    let mut r = Ref { text, path, lines, check: Check::NotFound, quote: None, why: String::new() };
    let Some(p) = &r.path else {
        r.why = "No file by this name is among what the model was given; it may be made up.".into();
        return r;
    };
    let Some((a, b)) = lines else {
        r.check = Check::Verified;
        return r;
    };
    match sent.iter().find(|(s, _)| s == p) {
        Some((_, runs)) if runs.iter().any(|&(x, y)| x <= a && b <= y) => {
            r.check = Check::Verified;
            r.quote = line_of(p, a);
        }
        Some((_, runs)) => {
            r.check = Check::FileOnly;
            let given = crate::askpick::Picked { text: String::new(), ranges: runs.clone(), total: 0, cut: false }.describe();
            let given = given.trim_end_matches(" of 0");
            r.why = format!("The model was given {given} of this file, so line {b} can't be checked.");
        }
        None => {
            r.check = Check::FileOnly;
            r.why = "The model wasn't given this file's lines, so the line number can't be checked.".into();
        }
    }
    r
}

/// The asked-about item `name` refers to: one of them by name or path, or something inside a
/// folder among them (directly or by a relative path). It must exist.
fn resolve(name: &str, asked: &[PathBuf]) -> Option<PathBuf> {
    let name = name.trim_start_matches("./");
    let rel = Path::new(name);
    if rel.is_absolute() {
        return asked.iter().find(|a| rel.starts_with(a) && rel.exists()).map(|_| rel.to_path_buf());
    }
    // An item asked about, by its name or by its path's tail.
    if let Some(a) = asked.iter().find(|a| a.ends_with(rel)) {
        return Some(a.clone());
    }
    // Inside a folder asked about, or beside a file asked about.
    for a in asked {
        let dir = if a.is_dir() { a.as_path() } else { a.parent()? };
        let p = dir.join(rel);
        if !rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) && p.exists() {
            return Some(p);
        }
    }
    None
}

/// Line `n` (from 1) of the file at `p`, trimmed, at most 200 characters.
fn line_of(p: &Path, n: usize) -> Option<String> {
    use std::io::{BufRead, BufReader, Read};
    let f = std::fs::File::open(p).ok()?;
    let line = BufReader::new(f.take(8 << 20)).lines().nth(n - 1)?.ok()?;
    Some(line.trim().chars().take(200).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_names_and_lines_against_what_was_sent() {
        let d = std::env::temp_dir().join(format!("filescape-refs-{}", std::process::id()));
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("config.yaml"), "a: 1\nport: 8080\nc: 3\n").unwrap();
        std::fs::write(d.join("src/main.rs"), "fn main() {}\n").unwrap();
        let asked = vec![d.clone()];
        let sent = vec![(d.join("config.yaml"), vec![(1, 3)]), (d.join("other.txt"), vec![(1, 1)])];
        std::fs::write(d.join("LICENSE"), "MIT\n").unwrap();
        let answer = "The port is set in [config.yaml:2], see also [src/main.rs], [missing.txt:4], \
                      [config.yaml:9], [src/main.rs:1], a [link](http://x), a footnote [1], [LICENSE] and [optional].";
        let r = find(answer, &asked, &sent);
        let got: Vec<(&str, Check)> = r.iter().map(|r| (r.text.as_str(), r.check)).collect();
        assert_eq!(
            got,
            vec![
                ("config.yaml:2", Check::Verified),
                ("src/main.rs", Check::Verified),
                ("missing.txt:4", Check::NotFound),
                ("config.yaml:9", Check::FileOnly),
                ("src/main.rs:1", Check::FileOnly),
                ("LICENSE", Check::Verified),
            ]
        );
        assert_eq!(r[0].quote.as_deref(), Some("port: 8080"));
        assert_eq!(r[0].path.as_deref(), Some(d.join("config.yaml").as_path()));
        // Nothing outside what was asked about, even if it exists.
        assert!(find("[../etc/passwd]", &asked, &sent).iter().all(|r| r.check == Check::NotFound));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Asked about one file, bare line numbers are that file's lines.
    #[test]
    fn bare_lines_belong_to_the_only_file() {
        let d = std::env::temp_dir().join(format!("filescape-refs-bare-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let lic = d.join("LICENSE");
        std::fs::write(&lic, (1..=80).map(|i| format!("line {i}\n")).collect::<String>()).unwrap();
        let sent = vec![(lic.clone(), vec![(1, 60), (63, 80)])];
        let r = find("CC0 [3]. Waived [63-78] for anyone [25-26, 72], see [99] and [optional].", std::slice::from_ref(&lic), &sent);
        let got: Vec<(&str, Check)> = r.iter().map(|r| (r.text.as_str(), r.check)).collect();
        assert_eq!(
            got,
            vec![
                ("LICENSE:3", Check::Verified),
                ("LICENSE:63-78", Check::Verified),
                ("LICENSE:25-26", Check::Verified),
                ("LICENSE:72", Check::Verified),
                ("LICENSE:99", Check::FileOnly),
            ]
        );
        // A line from a part left out can't be checked.
        let gap = find("see [61]", std::slice::from_ref(&lic), &sent);
        assert_eq!(gap[0].check, Check::FileOnly);
        assert_eq!(gap[0].why, "The model was given lines 1–60 and 63–80 of this file, so line 61 can't be checked.");
        assert_eq!(r[0].quote.as_deref(), Some("line 3"));
        assert_eq!(bare_lines("lines 4 and 9"), Some(vec![(4, 4), (9, 9)]));
        assert_eq!(bare_lines("a-b"), None);
        assert_eq!(bare_lines("1-2-3"), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn line_forms() {
        assert_eq!(parse("notes.txt:12"), Some(("notes.txt".into(), Some((12, 12)))));
        assert_eq!(parse("a/b.rs:3-7"), Some(("a/b.rs".into(), Some((3, 7)))));
        assert_eq!(parse("notes.txt line 4"), Some(("notes.txt".into(), Some((4, 4)))));
        assert_eq!(parse("optional"), None);
        assert_eq!(parse("1"), None);
        assert_eq!(parse("x"), None);
    }
}
