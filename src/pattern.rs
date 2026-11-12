//! The Custom filter's pattern language: file-name wildcards plus frame-number tokens for
//! image sequences. Patterns are parsed once into tokens and matched without allocating.
//!
//! Syntax (see `HELP`):
//!
//! * Terms are separated by commas. A file matches when it matches any term (or all of them,
//!   with `Options::match_all`). A term starting with `!` excludes what it matches.
//! * A bare word is an extension: `exr`, `.exr` and `*.exr` all mean "names ending in .exr".
//! * `*` matches any run of characters, `?` exactly one.
//! * `#` is one digit; `####` is a frame number of exactly four digits.
//! * `{frame}` is a frame number of any length; `{frame:04}` exactly four digits;
//!   `{frame:1001-1100}` a number in that range at any padding; `{frame:1001-1100:04}` both.
//! * `[1001-1100]` is the same as `{frame:1001-1100}`.
//! * A frame number always covers a whole run of digits: `render.####.exr` doesn't match
//!   `render.00001.exr`.
//! * `[…]` is a range only when it holds exactly `digits-digits`, and `{…}` is special only as
//!   `{frame…}`; otherwise both are ordinary characters, so names such as `[draft] notes.txt`,
//!   `[2024-05-01].txt` or `{backup}.zip` need no escaping. `\` makes the next character
//!   literal: `\#`, `\*`, `\,`, `\!`, `\[1-2]`.
//! * Matching ignores case unless `Options::case_sensitive` is set.

use std::fmt;

/// Shown next to the input as a quick reference.
pub const HELP: &str = "\
Terms are separated by commas; a file matches any of them.
  exr  .exr  *.exr     files ending in .exr
  render_*             names starting with render_
  *diffuse*            names containing diffuse
  shot_????.exr        ? is exactly one character
  shot_####.exr        # is one digit: exactly four digits here
  {frame}              a frame number with any number of digits
  {frame:04}           exactly four digits
  {frame:1001-1100}    a frame number in that range, any padding
  {frame:1001-1100:04} in that range and exactly four digits
  [1001-1100]          short for {frame:1001-1100}
  !*preview*           leave out names containing preview
A frame number always takes a whole run of digits. [ ] is a range only around two whole
numbers, and { } only around frame; otherwise they're ordinary characters; put \\ before a character to take it literally (\\#  \\*  \\,  \\!).";

/// Matching options chosen next to the input.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Options {
    pub case_sensitive: bool,
    /// A file must match every (non-excluding) term instead of any one.
    pub match_all: bool,
}

/// A frame-number token.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct Num {
    /// Exact digit count, if fixed.
    width: Option<u8>,
    /// Inclusive range of values, if limited.
    range: Option<(u64, u64)>,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Tok {
    /// Literal bytes (already lowercased when matching ignores case).
    Lit(Vec<u8>),
    /// `?`
    One,
    /// `*`
    Star,
    Num(Num),
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Term {
    /// Name ends with "." + ext (lowercased when ignoring case). `bare` (typed without a dot)
    /// also matches a file named exactly `ext`, e.g. `Makefile`.
    Ext { ext: Box<[u8]>, bare: bool },
    Glob(Vec<Tok>),
}

/// A compiled pattern list. Cheap to clone behind an `Arc`, compared and hashed by content.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Filter {
    source: String,
    opts: Options,
    include: Vec<Term>,
    exclude: Vec<Term>,
}

/// What's wrong with a pattern, for the warning under the input.
#[derive(Clone, PartialEq, Debug)]
pub struct Error {
    /// The offending term as typed.
    pub term: String,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "“{}”: {}", self.term, self.message)
    }
}

impl Filter {
    /// Parse `source`. An empty or all-blank source gives a filter with no terms (`is_empty`).
    pub fn compile(source: &str, opts: Options) -> Result<Filter, Error> {
        let mut include = Vec::new();
        let mut exclude = Vec::new();
        for raw in split_terms(source) {
            let text = raw.trim();
            if text.is_empty() {
                continue;
            }
            let err = |message: &str| Error { term: text.to_string(), message: message.to_string() };
            let (neg, body) = match text.strip_prefix('!') {
                Some(rest) => (true, rest.trim_start()),
                None => (false, text),
            };
            if body.is_empty() {
                return Err(err("“!” needs a pattern after it, e.g. !*preview*"));
            }
            let term = parse_term(body, opts.case_sensitive).map_err(|m| err(&m))?;
            if neg { exclude.push(term) } else { include.push(term) }
        }
        Ok(Filter { source: source.to_string(), opts, include, exclude })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// No terms at all: matches everything, and isn't worth applying.
    pub fn is_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }

    /// Does a file with this name pass?
    pub fn matches(&self, name: &str) -> bool {
        // Case folding: ASCII names (nearly all) are folded byte by byte while matching;
        // anything else is lowercased once up front.
        let folded;
        let (bytes, fold) = if self.opts.case_sensitive {
            (name.as_bytes(), false)
        } else if name.is_ascii() {
            (name.as_bytes(), true)
        } else {
            folded = name.to_lowercase();
            (folded.as_bytes(), false)
        };
        let hit = |t: &Term| term_matches(t, bytes, fold);
        if self.exclude.iter().any(hit) {
            return false;
        }
        if self.include.is_empty() {
            return true;
        }
        if self.opts.match_all { self.include.iter().all(hit) } else { self.include.iter().any(hit) }
    }

    /// The extension being typed in the last term, for autocomplete: `"ex"` for `*.png, *.ex`,
    /// `".ex"`, or a bare `ex`. None when the last term isn't an extension.
    pub fn typing_ext(source: &str) -> Option<(usize, &str)> {
        let start = source.rfind(',').map_or(0, |i| i + 1);
        let last = &source[start..];
        let lead = last.len() - last.trim_start().len();
        let t = last.trim_start();
        let t = t.strip_prefix('!').unwrap_or(t);
        let word = t.strip_prefix("*.").or_else(|| t.strip_prefix('.')).unwrap_or(t);
        let ok = word.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        ok.then_some((start + lead + (last.trim_start().len() - word.len()), word))
    }
}

/// Split on commas that aren't escaped.
fn split_terms(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut esc = false;
    for (i, c) in s.char_indices() {
        if esc {
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == ',' {
            out.push(&s[start..i]);
            start = i + 1;
        }
    }
    out.push(&s[start..]);
    out
}

fn is_word(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'+' || b == b'~')
}

fn parse_term(body: &str, case_sensitive: bool) -> Result<Term, String> {
    let fold = |s: &str| if case_sensitive { s.to_string() } else { s.to_lowercase() };
    // Extension shorthands: exr, .exr, *.exr
    if is_word(body) {
        return Ok(Term::Ext { ext: fold(body).into_bytes().into(), bare: true });
    }
    if let Some(w) = body.strip_prefix("*.").or_else(|| body.strip_prefix('.')).filter(|w| is_word(w)) {
        return Ok(Term::Ext { ext: fold(w).into_bytes().into(), bare: false });
    }

    let chars: Vec<char> = body.chars().collect();
    let mut toks: Vec<Tok> = Vec::new();
    let mut lit = String::new();
    let flush = |lit: &mut String, toks: &mut Vec<Tok>| {
        if !lit.is_empty() {
            toks.push(Tok::Lit(fold(lit).into_bytes()));
            lit.clear();
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                let Some(&next) = chars.get(i + 1) else {
                    return Err("“\\” at the end has nothing to make literal".into());
                };
                lit.push(next);
                i += 2;
                continue;
            }
            '*' => {
                flush(&mut lit, &mut toks);
                if toks.last() != Some(&Tok::Star) {
                    toks.push(Tok::Star);
                }
            }
            '?' => {
                flush(&mut lit, &mut toks);
                toks.push(Tok::One);
            }
            '#' => {
                flush(&mut lit, &mut toks);
                let mut n = 0;
                while chars.get(i + n) == Some(&'#') {
                    n += 1;
                }
                if n > 18 {
                    return Err("At most 18 # in a row".into());
                }
                toks.push(Tok::Num(Num { width: Some(n as u8), range: None }));
                i += n;
                continue;
            }
            '[' => {
                // A range only when the brackets hold exactly digits-dash-digits; any other
                // bracket ([draft], [1080p], [2024-05-01]) is an ordinary character.
                let close = chars[i..].iter().position(|&c| c == ']').map(|p| p + i);
                let parsed = close.and_then(|e| parse_range(&chars[i + 1..e].iter().collect::<String>()));
                match parsed {
                    Some(range) => {
                        flush(&mut lit, &mut toks);
                        toks.push(Tok::Num(Num { width: None, range: Some(range?) }));
                        i = close.unwrap() + 1;
                        continue;
                    }
                    None => lit.push(c),
                }
            }
            '{' if starts_frame(&chars[i + 1..]) => {
                let Some(e) = chars[i..].iter().position(|&c| c == '}').map(|p| p + i) else {
                    return Err("“{frame” has no closing “}”".into());
                };
                let inner: String = chars[i + 1 + 5..e].iter().collect();
                flush(&mut lit, &mut toks);
                toks.push(Tok::Num(parse_frame_spec(&inner)?));
                i = e + 1;
                continue;
            }
            _ => lit.push(c),
        }
        i += 1;
    }
    flush(&mut lit, &mut toks);

    // A frame number takes a whole run of digits, so it can never sit right next to a literal
    // digit or another frame number. Say so instead of silently matching nothing.
    for w in toks.windows(2) {
        let bad = match (&w[0], &w[1]) {
            (Tok::Num(_), Tok::Num(_)) => true,
            (Tok::Lit(l), Tok::Num(_)) => l.last().is_some_and(u8::is_ascii_digit),
            (Tok::Num(_), Tok::Lit(l)) => l.first().is_some_and(u8::is_ascii_digit),
            _ => false,
        };
        if bad {
            return Err("A frame number can't touch another digit; it always takes the whole run of digits".into());
        }
    }
    Ok(Term::Glob(toks))
}

/// `{` followed by `frame}` or `frame:` (any case)?
fn starts_frame(rest: &[char]) -> bool {
    rest.len() >= 5
        && rest[..5].iter().collect::<String>().eq_ignore_ascii_case("frame")
        && rest.get(5).is_none_or(|c| matches!(c, '}' | ':'))
}

/// "1001-1100" -> Some(Ok((1001, 1100))). None if it isn't digits-dash-digits at all.
fn parse_range(s: &str) -> Option<Result<(u64, u64), String>> {
    let (a, b) = s.split_once('-')?;
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() || b.is_empty() || !a.bytes().all(|c| c.is_ascii_digit()) || !b.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (Ok(lo), Ok(hi)) = (a.parse::<u64>(), b.parse::<u64>()) else {
        return Some(Err("Frame numbers that large aren't supported".into()));
    };
    if lo > hi {
        return Some(Err(format!("The range {lo}-{hi} runs backwards; write {hi}-{lo}")));
    }
    Some(Ok((lo, hi)))
}

/// The part of `{frame…}` after "frame": "", ":04", ":1001-1100", ":1001-1100:04".
fn parse_frame_spec(s: &str) -> Result<Num, String> {
    let mut num = Num { width: None, range: None };
    if s.is_empty() {
        return Ok(num);
    }
    let Some(rest) = s.strip_prefix(':') else {
        return Err(format!("Unexpected “{s}” in {{frame…}}"));
    };
    let parts: Vec<&str> = rest.split(':').map(str::trim).collect();
    if parts.len() > 2 {
        return Err("{frame} takes at most a range and a width, e.g. {frame:1001-1100:04}".into());
    }
    for p in parts {
        if p.contains('-') {
            if num.range.is_some() || num.width.is_some() {
                return Err("Put the range before the width, once: {frame:1001-1100:04}".into());
            }
            num.range = Some(parse_range(p).ok_or_else(|| format!("“{p}” isn't a range like 1001-1100"))??);
        } else if !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()) {
            if num.width.is_some() {
                return Err("Only one width, e.g. {frame:04}".into());
            }
            let w: u32 = p.parse().map_err(|_| "Width too large".to_string())?;
            if !(1..=18).contains(&w) {
                return Err("The width must be between 1 and 18 digits".into());
            }
            num.width = Some(w as u8);
        } else {
            return Err(format!("“{p}” isn't a width (04) or a range (1001-1100)"));
        }
    }
    if let (Some(w), Some((lo, _))) = (num.width, num.range)
        && digits(lo) > w as u32
    {
        return Err(format!("{lo} doesn't fit in {w} digits"));
    }
    Ok(num)
}

fn digits(mut v: u64) -> u32 {
    let mut n = 1;
    while v >= 10 {
        v /= 10;
        n += 1;
    }
    n
}

fn eq_bytes(a: &[u8], b: &[u8], fold: bool) -> bool {
    if fold { a.eq_ignore_ascii_case(b) } else { a == b }
}

fn term_matches(t: &Term, name: &[u8], fold: bool) -> bool {
    match t {
        Term::Ext { ext, bare } => {
            let n = ext.len();
            (name.len() > n && name[name.len() - n - 1] == b'.' && eq_bytes(&name[name.len() - n..], ext, fold))
                || (*bare && eq_bytes(name, ext, fold))
        }
        Term::Glob(toks) => glob_matches(toks, name, fold),
    }
}

/// Length of the UTF-8 character starting at `b`.
fn char_len(b: u8) -> usize {
    match b {
        0xF0.. => 4,
        0xE0.. => 3,
        0xC0.. => 2,
        _ => 1,
    }
}

/// Match a whole name. Every token except `*` consumes a length fixed by its start position
/// (a frame number takes the whole digit run), so the classic "retry from the last star"
/// algorithm is exact and runs in O(name × pattern) at worst, with no recursion.
fn glob_matches(toks: &[Tok], name: &[u8], fold: bool) -> bool {
    let (mut ti, mut ni) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    loop {
        let advanced = match toks.get(ti) {
            None => {
                if ni == name.len() {
                    return true;
                }
                None
            }
            Some(Tok::Star) => {
                star = Some((ti + 1, ni));
                ti += 1;
                continue;
            }
            Some(Tok::Lit(l)) => {
                (name.len() - ni >= l.len() && eq_bytes(&name[ni..ni + l.len()], l, fold)).then_some(ni + l.len())
            }
            Some(Tok::One) => (ni < name.len()).then(|| (ni + char_len(name[ni])).min(name.len())),
            Some(Tok::Num(n)) => num_at(n, name, ni),
        };
        match advanced {
            Some(next) => {
                ni = next;
                ti += 1;
            }
            None => {
                // Let the last star swallow one more character and try again.
                let Some((st, sn)) = star else { return false };
                if sn >= name.len() {
                    return false;
                }
                let sn = sn + char_len(name[sn]);
                star = Some((st, sn));
                ti = st;
                ni = sn;
            }
        }
    }
}

/// If a frame number matching `n` starts at `at`, where it ends.
fn num_at(n: &Num, name: &[u8], at: usize) -> Option<usize> {
    if at >= name.len() || !name[at].is_ascii_digit() || (at > 0 && name[at - 1].is_ascii_digit()) {
        return None;
    }
    let end = at + name[at..].iter().take_while(|b| b.is_ascii_digit()).count();
    let len = end - at;
    if n.width.is_some_and(|w| w as usize != len) {
        return None;
    }
    if let Some((lo, hi)) = n.range {
        let sig = &name[at..end];
        let sig = &sig[sig.iter().take_while(|&&b| b == b'0').count().min(sig.len() - 1)..];
        if sig.len() > 19 {
            return None;
        }
        let v: u64 = std::str::from_utf8(sig).ok()?.parse().ok()?;
        if v < lo || v > hi {
            return None;
        }
    }
    Some(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(p: &str) -> Filter {
        Filter::compile(p, Options::default()).unwrap_or_else(|e| panic!("{p}: {e}"))
    }
    fn m(p: &str, name: &str) -> bool {
        f(p).matches(name)
    }

    #[test]
    fn extensions_with_and_without_dot() {
        for p in ["exr", ".exr", "*.exr", "EXR", "*.EXR", " exr "] {
            assert!(m(p, "beauty.0001.exr"), "{p}");
            assert!(m(p, "BEAUTY.EXR"), "{p}");
            assert!(!m(p, "beauty.exr.bak"), "{p}");
            assert!(!m(p, "exrfile.png"), "{p}");
            assert!(!m(p, "notexr"), "{p}");
        }
        // A bare word also matches a file of exactly that name (Makefile, README).
        assert!(m("makefile", "Makefile"));
        assert!(!m(".makefile", "Makefile"));
        // Multi-part extensions go through the glob path.
        assert!(m("*.tar.gz", "src.TAR.gz"));
        assert!(!m("*.tar.gz", "src.gz"));
    }

    #[test]
    fn case_sensitivity() {
        assert!(m("*Diffuse*", "rock_DIFFUSE_v2.tif"));
        let cs = Filter::compile("*Diffuse*, exr", Options { case_sensitive: true, ..Default::default() }).unwrap();
        assert!(cs.matches("rock_Diffuse.tif"));
        assert!(!cs.matches("rock_diffuse.tif"));
        assert!(cs.matches("a.exr"));
        assert!(!cs.matches("a.EXR"));
        // Non-ASCII names fold too.
        assert!(m("*ÉTÉ*", "photos_été.jpg"));
        assert!(m("caf?.txt", "café.txt"), "? is one character, not one byte");
    }

    #[test]
    fn wildcards() {
        assert!(m("render_*", "render_beauty.exr"));
        assert!(!m("render_*", "xrender_beauty.exr"));
        assert!(m("*diffuse*", "rock_diffuse_4k.png"));
        assert!(m("shot_????.exr", "shot_ab12.exr"));
        assert!(!m("shot_????.exr", "shot_abc.exr"));
        assert!(!m("shot_????.exr", "shot_abcde.exr"));
        assert!(m("shot_*.exr", "shot_.exr"));
        assert!(m("shot_*.exr", "shot_0042_v2.exr"));
        assert!(m("*a*b*c", "xxaxxbxxbxc"));
        assert!(!m("*a*b*c", "xxaxxbxxbxcd"));
        assert!(m("*", ""));
    }

    #[test]
    fn several_terms() {
        let p = f("*.exr, *.tx, *.tif");
        assert!(p.matches("a.exr") && p.matches("b.TX") && p.matches("c.tif"));
        assert!(!p.matches("d.tiff"));
        // Blank terms (trailing comma while typing) are ignored.
        assert!(m("*.exr, ", "a.exr"));
        assert!(f(" , ").is_empty());
    }

    #[test]
    fn exclusions() {
        let p = f("*.exr, !*preview*");
        assert!(p.matches("beauty.1001.exr"));
        assert!(!p.matches("beauty_preview.1001.exr"));
        assert!(!p.matches("beauty.1001.png"));
        // Only exclusions: everything else passes.
        let p = f("!*.tmp, !~$*");
        assert!(p.matches("a.doc"));
        assert!(!p.matches("a.TMP"));
        assert!(!p.matches("~$a.doc"));
    }

    #[test]
    fn match_all() {
        let p = Filter::compile("*.exr, *beauty*", Options { match_all: true, ..Default::default() }).unwrap();
        assert!(p.matches("shot_beauty.1001.exr"));
        assert!(!p.matches("shot_depth.1001.exr"));
        assert!(!p.matches("shot_beauty.1001.png"));
    }

    #[test]
    fn padding() {
        assert!(m("shot_####.exr", "shot_1001.exr"));
        assert!(m("shot_####.exr", "shot_0001.exr"));
        assert!(!m("shot_####.exr", "shot_001.exr"));
        assert!(!m("shot_####.exr", "shot_00001.exr"));
        assert!(!m("shot_####.exr", "shot_abcd.exr"));
        for (p, yes, no) in [
            ("render.####.exr", "render.0100.exr", "render.001001.exr"),
            ("render.######.exr", "render.001001.exr", "render.1001.exr"),
            ("render.{frame}.exr", "render.7.exr", "render..exr"),
            ("render.{frame:04}.exr", "render.1001.exr", "render.10001.exr"),
            ("render.{frame:4}.exr", "render.0002.exr", "render.2.exr"),
        ] {
            assert!(m(p, yes), "{p} should match {yes}");
            assert!(!m(p, no), "{p} should not match {no}");
        }
        assert!(m("render.{frame}.exr", "render.001001.exr"));
        assert!(m("render.*.exr", "render.whatever.exr"));
        // A frame number takes the whole digit run, even after a star.
        assert!(!m("*####.exr", "12345.exr"));
        assert!(m("*_####.exr", "v12_2345.exr"));
    }

    #[test]
    fn frame_ranges() {
        let p = f("render.{frame:1001-1100}.exr");
        assert!(p.matches("render.1001.exr"));
        assert!(p.matches("render.1100.exr"));
        assert!(!p.matches("render.1000.exr"));
        assert!(!p.matches("render.1101.exr"));
        assert!(p.matches("render.001050.exr"), "any padding");
        let p = f("render.{frame:1001-1100:04}.exr");
        assert!(p.matches("render.1050.exr"));
        assert!(!p.matches("render.001050.exr"));
        let p = f("shot_[1001-1100].exr");
        assert!(p.matches("shot_1001.exr") && p.matches("shot_1100.exr"));
        assert!(!p.matches("shot_1101.exr") && !p.matches("shot_999.exr"));
        // 0 and single-frame ranges.
        assert!(m("f[0-0].png", "f0000.png"));
        assert!(m("f[7-7].png", "f7.png"));
        assert!(!m("f[7-7].png", "f8.png"));
        // Very long digit runs never panic or overflow.
        assert!(!m("f[1-9].png", "f123456789012345678901234567890.png"));
        assert!(m("f{frame}.png", "f123456789012345678901234567890.png"));
        assert!(m("render.{frame:1001-1200:04}.exr", "render.1200.exr"));
    }

    #[test]
    fn multiple_numeric_groups() {
        let p = f("shot010_comp.####.exr");
        assert!(p.matches("shot010_comp.1001.exr"));
        assert!(!p.matches("shot011_comp.1001.exr"));
        let p = f("shot###_comp_v##.{frame:1001-1003}.exr");
        assert!(p.matches("shot010_comp_v02.1002.exr"));
        assert!(!p.matches("shot010_comp_v02.1004.exr"));
        assert!(!p.matches("shot10_comp_v02.1002.exr"));
        let p = f("*.[1001-1100].exr");
        assert!(p.matches("shot010_v002.1050.exr"));
        assert!(!p.matches("shot010_v002.0050.exr"));
    }

    #[test]
    fn ordinary_brackets_and_braces_are_literal() {
        assert!(m("[draft]*", "[draft] notes.txt"));
        assert!(m("{backup}*", "{backup} 2024.zip"));
        assert!(m("{framework}*", "{framework}.md"));
        assert!(m("*(1).jpg", "IMG (1).jpg"));
        assert!(m("*[final]*", "cut [final].mov"));
        assert!(!m("*[final]*", "cut final.mov"));
        assert!(m("[1080p]*", "[1080p] film.mkv"));
        assert!(m("[2024-05-01]*", "[2024-05-01] notes.txt"));
        assert!(m("a[1001-].txt", "a[1001-].txt"));
        assert!(m("a[1001-1100.txt", "a[1001-1100.txt"));
        // Escapes.
        assert!(m("track \\#1.mp3", "Track #1.mp3"));
        assert!(!m("track \\#1.mp3", "Track 51.mp3"));
        assert!(m("a\\,b.txt", "a,b.txt"));
        assert!(m("\\[1-2].txt", "[1-2].txt"));
        assert!(m("\\!important.txt", "!important.txt"));
        assert!(m("what\\?.txt", "what?.txt"));
        assert!(!m("what\\?.txt", "whatx.txt"));
    }

    #[test]
    fn invalid_patterns() {
        for p in [
            "shot_[1100-1001].exr",
            "{frame",
            "{frame:abc}",
            "{frame:04:1001-1100}",
            "{frame:1001-1100:03}",
            "{frame:0}",
            "{frame:1-2:3:4}",
            "{frame}{frame}",
            "v1####",
            "##2",
            "!",
            "*.exr, !",
            "abc\\",
        ] {
            assert!(Filter::compile(p, Options::default()).is_err(), "{p} should be rejected");
        }
        let e = Filter::compile("*.exr, shot_[1100-1001].exr", Options::default()).unwrap_err();
        assert_eq!(e.term, "shot_[1100-1001].exr");
        assert!(e.message.contains("backwards"));
    }

    #[test]
    fn typing_ext() {
        assert_eq!(Filter::typing_ext("ex"), Some((0, "ex")));
        assert_eq!(Filter::typing_ext("*.png, *.ex"), Some((9, "ex")));
        assert_eq!(Filter::typing_ext("*.png, .e"), Some((8, "e")));
        assert_eq!(Filter::typing_ext("!*.tm"), Some((3, "tm")));
        assert_eq!(Filter::typing_ext("render_*"), None);
    }
}
