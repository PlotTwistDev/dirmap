//! Syntax colouring for code in previews: syntect with bat's grammars and themes (two-face),
//! pure Rust (fancy-regex, no Oniguruma).
//!
//! Colours are worked out on the preview worker, for the dark and the light theme both, so
//! switching themes needs no re-reading. Work is bounded: very long lines (minified code) aren't
//! coloured, and colouring stops after a time limit, leaving the rest plain.

use std::sync::OnceLock;
use std::time::{Duration, Instant};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use two_face::theme::EmbeddedThemeName;

/// A run of text in one colour: byte range, RGB, bold, italic.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub rgb: [u8; 3],
    pub bold: bool,
    pub italic: bool,
}

/// The colours of a text in both themes, and the language's name. Text past the last span
/// is plain.
#[derive(Debug)]
pub struct Colours {
    pub language: String,
    pub dark: Vec<Span>,
    pub light: Vec<Span>,
}

/// Lines longer than this aren't coloured (and nor is anything after them).
const LINE_MAX: usize = 4000;
/// Time allowed for each theme.
const BUDGET: Duration = Duration::from_millis(1500);

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

fn themes() -> &'static (Theme, Theme) {
    static THEMES: OnceLock<(Theme, Theme)> = OnceLock::new();
    THEMES.get_or_init(|| {
        let set = two_face::theme::extra();
        (set.get(EmbeddedThemeName::OneHalfDark).clone(), set.get(EmbeddedThemeName::OneHalfLight).clone())
    })
}

/// The grammar for a file, from its name or its first line (`#!/bin/sh`), if it's code worth
/// colouring (not plain text).
fn syntax_for(name: &str, first_line: &str) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let lower = name.to_ascii_lowercase();
    let ext = crate::scan::ext_of(&lower);
    // A name with no extension: its first line (`#!/bin/bash`) says more than the name
    // ("build" is also an extension), then the whole name (Dockerfile, Makefile).
    let s = if ext.is_empty() {
        set.find_syntax_by_first_line(first_line).or_else(|| set.find_syntax_by_extension(&lower))
    } else {
        set.find_syntax_by_extension(ext).or_else(|| set.find_syntax_by_first_line(first_line))
    }?;
    (s.name != "Plain Text").then_some(s)
}

/// The colours of `text` from the file `name`, or None for text that isn't code.
pub fn colour(name: &str, text: &str) -> Option<Colours> {
    let syntax = syntax_for(name, text.lines().next().unwrap_or(""))?;
    let (dark, light) = themes();
    let run = |theme: &Theme| -> Vec<Span> {
        let mut h = HighlightLines::new(syntax, theme);
        let mut out = Vec::new();
        let started = Instant::now();
        let mut at = 0;
        for (n, line) in syntect::util::LinesWithEndings::from(text).enumerate() {
            if line.len() > LINE_MAX || n % 64 == 0 && started.elapsed() > BUDGET {
                break;
            }
            let Ok(ranges) = h.highlight_line(line, syntaxes()) else { break };
            for (style, piece) in ranges {
                let end = at + piece.len();
                let c = style.foreground;
                let span = Span { start: at, end, rgb: [c.r, c.g, c.b], bold: style.font_style.contains(FontStyle::BOLD), italic: style.font_style.contains(FontStyle::ITALIC) };
                // Join runs of the same look.
                match out.last_mut() {
                    Some(Span { end: e, rgb, bold, italic, .. }) if *e == at && *rgb == span.rgb && *bold == span.bold && *italic == span.italic => *e = end,
                    _ => out.push(span),
                }
                at = end;
            }
        }
        out
    };
    Some(Colours { language: syntax.name.clone(), dark: run(dark), light: run(light) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_code_not_prose() {
        let src = "fn main() {\n    let x = \"hi\"; // note\n}\n";
        let c = colour("main.rs", src).unwrap();
        assert_eq!(c.language, "Rust");
        assert_eq!(c.dark.last().unwrap().end, src.len(), "every byte is covered");
        assert!(c.dark.windows(2).all(|w| w[0].end == w[1].start));
        let colours: std::collections::HashSet<_> = c.dark.iter().map(|s| s.rgb).collect();
        assert!(colours.len() >= 3, "keywords, strings and comments differ");
        assert_ne!(c.dark[0].rgb, c.light[0].rgb);
        assert!(colour("notes.txt", "just words").is_none());
        assert_eq!(colour("build", "#!/bin/bash\necho hi\n").unwrap().language, "Bourne Again Shell (bash)");
        assert_eq!(colour("Dockerfile", "FROM alpine\n").unwrap().language, "Dockerfile");
        // Minified code: not coloured past the long line.
        let long = format!("var a = 1;\n{}\n", "x".repeat(LINE_MAX + 1));
        assert_eq!(colour("app.min.js", &long).unwrap().dark.last().unwrap().end, 11);
    }
}
