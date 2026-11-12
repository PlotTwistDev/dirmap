//! The help pages as data: Markdown parsed into blocks, the table of contents from
//! `SUMMARY.md`, link resolution and search. No drawing here, so it can all be tested.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::collections::HashMap;

mod bundle {
    include!(concat!(env!("OUT_DIR"), "/help_bundle.rs"));
}

/// The page shown when nothing more specific is asked for.
pub const HOME: &str = "index.md";

/// A run of text with one style.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    /// Link target as written (relative path, `#anchor` or URL).
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading { level: u8, spans: Vec<Span>, id: String },
    Para(Vec<Span>),
    List { start: Option<u64>, items: Vec<Vec<Block>> },
    Code { lang: String, text: String },
    Quote(Vec<Block>),
    Table { head: Vec<Vec<Span>>, rows: Vec<Vec<Vec<Span>>> },
    Image { src: String, alt: String },
    Rule,
}

pub struct Page {
    /// Path under `docs/src`, e.g. `views/treemap.md`.
    pub path: String,
    pub title: String,
    /// The part of the table of contents it's in ("Getting Started"); empty for the home page.
    pub section: String,
    pub blocks: Vec<Block>,
    /// (level, text, id) of every heading, in order.
    pub headings: Vec<(u8, String, String)>,
    /// Lowercased plain text of the body, for search.
    text: String,
    lower_title: String,
}

/// A part of the table of contents.
pub struct Section {
    pub title: String,
    pub pages: Vec<String>,
}

pub struct Docs {
    pub pages: Vec<Page>,
    by_path: HashMap<String, usize>,
    pub sections: Vec<Section>,
    /// Every page in table-of-contents order (for Previous / Next).
    pub order: Vec<String>,
}

/// mdBook's heading ids: lowercase, spaces to dashes, other punctuation dropped.
pub fn slug(text: &str) -> String {
    text.trim()
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                Some(c.to_ascii_lowercase())
            } else if c.is_whitespace() {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

pub fn plain(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

/// Resolve a link written on page `from` to (page path, anchor). None for web links.
pub fn resolve(from: &str, link: &str) -> Option<(String, Option<String>)> {
    if link.contains("://") || link.starts_with("mailto:") {
        return None;
    }
    let (path, anchor) = match link.split_once('#') {
        Some((p, a)) => (p, Some(a.to_string())),
        None => (link, None),
    };
    if path.is_empty() {
        return Some((from.to_string(), anchor));
    }
    Some((join(from, path), anchor))
}

/// `path` relative to the folder of `from`, normalised (`..` removed).
pub fn join(from: &str, path: &str) -> String {
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

impl Docs {
    /// Parse every bundled page and the table of contents.
    pub fn load() -> Docs {
        Self::from_sources(bundle::PAGES)
    }

    pub fn from_sources(sources: &[(&str, &str)]) -> Docs {
        let summary = sources.iter().find(|(p, _)| *p == "SUMMARY.md").map(|(_, s)| *s).unwrap_or("");
        let (sections, order) = parse_summary(summary);
        let section_of: HashMap<&str, &str> =
            sections.iter().flat_map(|s| s.pages.iter().map(move |p| (p.as_str(), s.title.as_str()))).collect();
        let mut pages = Vec::new();
        for (path, src) in sources {
            if *path == "SUMMARY.md" {
                continue;
            }
            let blocks = parse(src);
            let mut headings = Vec::new();
            collect_headings(&blocks, &mut headings);
            let title = headings.iter().find(|h| h.0 == 1).map(|h| h.1.clone()).unwrap_or_else(|| path.to_string());
            let mut text = String::new();
            collect_text(&blocks, &mut text);
            pages.push(Page {
                path: path.to_string(),
                lower_title: title.to_lowercase(),
                title,
                section: section_of.get(path).map(|s| s.to_string()).unwrap_or_default(),
                blocks,
                headings,
                text: text.to_lowercase(),
            });
        }
        let by_path = pages.iter().enumerate().map(|(i, p)| (p.path.clone(), i)).collect();
        Docs { pages, by_path, sections, order }
    }

    pub fn page(&self, path: &str) -> Option<&Page> {
        self.by_path.get(path).map(|&i| &self.pages[i])
    }

    pub fn title<'a>(&'a self, path: &'a str) -> &'a str {
        self.page(path).map_or(path, |p| p.title.as_str())
    }

    /// Previous and next pages in the table of contents.
    pub fn neighbours(&self, path: &str) -> (Option<&str>, Option<&str>) {
        let Some(i) = self.order.iter().position(|p| p == path) else { return (None, None) };
        let prev = i.checked_sub(1).map(|j| self.order[j].as_str());
        (prev, self.order.get(i + 1).map(String::as_str))
    }

    /// Pages matching every word of `query`, best first: (path, anchor of the best heading, snippet).
    pub fn search(&self, query: &str) -> Vec<Hit> {
        let terms: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<(u32, Hit)> = Vec::new();
        for p in &self.pages {
            let mut score = 0u32;
            let mut all = true;
            for t in &terms {
                let in_title = p.lower_title.contains(t.as_str());
                let in_heads = p.headings.iter().filter(|h| h.1.to_lowercase().contains(t.as_str())).count() as u32;
                let in_body = p.text.matches(t.as_str()).count().min(8) as u32;
                if !in_title && in_heads == 0 && in_body == 0 {
                    all = false;
                    break;
                }
                score += in_title as u32 * 20 + in_heads.min(3) * 6 + in_body;
            }
            if !all {
                continue;
            }
            // Land on the first heading below the title that mentions a term.
            let anchor = p
                .headings
                .iter()
                .skip(1)
                .find(|h| terms.iter().any(|t| h.1.to_lowercase().contains(t.as_str())))
                .map(|h| h.2.clone());
            let snippet = snippet(&p.text, &terms[0]);
            hits.push((score, Hit { path: p.path.clone(), anchor, snippet }));
        }
        hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.path.cmp(&b.1.path)));
        hits.into_iter().map(|h| h.1).collect()
    }

    pub fn image(&self, path: &str) -> Option<&'static [u8]> {
        bundle::IMAGES.iter().find(|(p, _)| *p == path).map(|(_, b)| *b)
    }

    #[cfg(test)]
    pub fn image_paths() -> impl Iterator<Item = &'static str> {
        bundle::IMAGES.iter().map(|(p, _)| *p)
    }
}

pub struct Hit {
    pub path: String,
    pub anchor: Option<String>,
    pub snippet: String,
}

/// Some words either side of the first place `term` appears.
fn snippet(text: &str, term: &str) -> String {
    let Some(at) = text.find(term) else { return String::new() };
    let mut start = at.saturating_sub(60);
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (at + term.len() + 90).min(text.len());
    while !text.is_char_boundary(end) {
        end += 1;
    }
    let mut s = text[start..end].split_whitespace().collect::<Vec<_>>().join(" ");
    if start > 0 {
        s.insert_str(0, "… ");
    }
    if end < text.len() {
        s.push_str(" …");
    }
    s
}

/// `SUMMARY.md`: `# Part` lines start sections, `- [Title](path.md)` lines are pages, and a
/// `[Title](path.md)` line before the first part is the home page.
fn parse_summary(src: &str) -> (Vec<Section>, Vec<String>) {
    let mut sections: Vec<Section> = Vec::new();
    let mut order = Vec::new();
    for line in src.lines() {
        let t = line.trim();
        if let Some(title) = t.strip_prefix("# ") {
            if title != "Summary" {
                sections.push(Section { title: title.to_string(), pages: Vec::new() });
            }
            continue;
        }
        let Some(open) = t.find("](") else { continue };
        let Some(close) = t[open..].find(')') else { continue };
        let path = t[open + 2..open + close].to_string();
        order.push(path.clone());
        if let Some(s) = sections.last_mut() {
            s.pages.push(path);
        }
    }
    (sections, order)
}

fn collect_headings(blocks: &[Block], out: &mut Vec<(u8, String, String)>) {
    for b in blocks {
        match b {
            Block::Heading { level, spans, id } => out.push((*level, plain(spans), id.clone())),
            Block::List { items, .. } => items.iter().for_each(|i| collect_headings(i, out)),
            Block::Quote(inner) => collect_headings(inner, out),
            _ => {}
        }
    }
}

fn collect_text(blocks: &[Block], out: &mut String) {
    for b in blocks {
        match b {
            Block::Heading { spans, .. } | Block::Para(spans) => {
                out.push_str(&plain(spans));
                out.push('\n');
            }
            Block::List { items, .. } => items.iter().for_each(|i| collect_text(i, out)),
            Block::Quote(inner) => collect_text(inner, out),
            Block::Code { text, .. } => {
                out.push_str(text);
                out.push('\n');
            }
            Block::Table { head, rows } => {
                for cell in head.iter().chain(rows.iter().flatten()) {
                    out.push_str(&plain(cell));
                    out.push(' ');
                }
                out.push('\n');
            }
            Block::Image { alt, .. } => {
                out.push_str(alt);
                out.push('\n');
            }
            Block::Rule => {}
        }
    }
}

/// Every link and image target on a page, for checking: (target, is image).
#[cfg(test)]
pub fn links(blocks: &[Block]) -> Vec<(String, bool)> {
    fn spans(s: &[Span], out: &mut Vec<(String, bool)>) {
        out.extend(s.iter().filter_map(|s| s.link.clone()).map(|l| (l, false)));
    }
    let mut out = Vec::new();
    for b in blocks {
        match b {
            Block::Heading { spans: s, .. } | Block::Para(s) => spans(s, &mut out),
            Block::List { items, .. } => items.iter().for_each(|i| out.extend(links(i))),
            Block::Quote(inner) => out.extend(links(inner)),
            Block::Table { head, rows } => head.iter().chain(rows.iter().flatten()).for_each(|c| spans(c, &mut out)),
            Block::Image { src, .. } => out.push((src.clone(), true)),
            Block::Code { .. } | Block::Rule => {}
        }
    }
    out
}

// ---------- Markdown to blocks ----------

#[derive(Default)]
struct Style {
    bold: u32,
    italic: u32,
    link: Vec<String>,
}

/// Where blocks are being collected: the page, a list item, a quote.
enum Frame {
    Blocks(Vec<Block>),
    List { start: Option<u64>, items: Vec<Vec<Block>> },
}

pub fn parse(src: &str) -> Vec<Block> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let mut stack: Vec<Frame> = vec![Frame::Blocks(Vec::new())];
    let mut spans: Vec<Span> = Vec::new();
    let mut style = Style::default();
    let mut code: Option<(String, String)> = None;
    let mut image: Option<(String, String)> = None;
    let mut heading: Option<u8> = None;
    let mut table: Option<(Vec<Vec<Span>>, Vec<Vec<Vec<Span>>>, bool)> = None;
    let mut row: Vec<Vec<Span>> = Vec::new();

    fn push(stack: &mut [Frame], b: Block) {
        match stack.last_mut() {
            Some(Frame::Blocks(v)) => v.push(b),
            Some(Frame::List { items, .. }) => match items.last_mut() {
                Some(item) => item.push(b),
                None => items.push(vec![b]),
            },
            None => {}
        }
    }
    // Text gathered directly in a list item (a "tight" list has no paragraph events).
    fn flush(stack: &mut [Frame], spans: &mut Vec<Span>) {
        if !spans.is_empty() {
            let s = std::mem::take(spans);
            push(stack, Block::Para(s));
        }
    }

    for ev in Parser::new_ext(src, opts) {
        if let Some((_, text)) = &mut code {
            match ev {
                Event::Text(t) => text.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, mut text) = code.take().unwrap();
                    if text.ends_with('\n') {
                        text.pop();
                    }
                    push(&mut stack, Block::Code { lang, text });
                }
                _ => {}
            }
            continue;
        }
        if let Some((_, alt)) = &mut image {
            match ev {
                Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                Event::End(TagEnd::Image) => {
                    let (src, alt) = image.take().unwrap();
                    // An image on its own line becomes a block; inside text it's just its alt text.
                    if spans.iter().all(|s| s.text.trim().is_empty()) {
                        spans.clear();
                        push(&mut stack, Block::Image { src, alt });
                    } else {
                        spans.push(Span { text: alt, ..Default::default() });
                    }
                }
                _ => {}
            }
            continue;
        }
        match ev {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    flush(&mut stack, &mut spans);
                    heading = Some(match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        _ => 4,
                    });
                }
                Tag::Paragraph => flush(&mut stack, &mut spans),
                Tag::List(start) => {
                    flush(&mut stack, &mut spans);
                    stack.push(Frame::List { start, items: Vec::new() });
                }
                Tag::Item => {
                    flush(&mut stack, &mut spans);
                    if let Some(Frame::List { items, .. }) = stack.last_mut() {
                        items.push(Vec::new());
                    }
                }
                Tag::BlockQuote(_) => {
                    flush(&mut stack, &mut spans);
                    stack.push(Frame::Blocks(Vec::new()));
                }
                Tag::CodeBlock(kind) => {
                    flush(&mut stack, &mut spans);
                    let lang = match kind {
                        CodeBlockKind::Fenced(l) => l.split_whitespace().next().unwrap_or("").to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    code = Some((lang, String::new()));
                }
                Tag::Table(_) => table = Some((Vec::new(), Vec::new(), true)),
                Tag::TableHead => row.clear(),
                Tag::TableRow => row.clear(),
                Tag::TableCell => spans.clear(),
                Tag::Emphasis => style.italic += 1,
                Tag::Strong => style.bold += 1,
                Tag::Link { dest_url, .. } => style.link.push(dest_url.to_string()),
                Tag::Image { dest_url, .. } => image = Some((dest_url.to_string(), String::new())),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) => {
                    let s = std::mem::take(&mut spans);
                    let id = slug(&plain(&s));
                    push(&mut stack, Block::Heading { level: heading.take().unwrap_or(2), spans: s, id });
                }
                TagEnd::Paragraph => flush(&mut stack, &mut spans),
                TagEnd::Item => flush(&mut stack, &mut spans),
                TagEnd::List(_) => {
                    flush(&mut stack, &mut spans);
                    if let Some(Frame::List { start, items }) = stack.pop() {
                        push(&mut stack, Block::List { start, items });
                    }
                }
                TagEnd::BlockQuote(_) => {
                    flush(&mut stack, &mut spans);
                    if let Some(Frame::Blocks(inner)) = stack.pop() {
                        push(&mut stack, Block::Quote(inner));
                    }
                }
                TagEnd::TableCell => row.push(std::mem::take(&mut spans)),
                TagEnd::TableHead => {
                    if let Some(t) = &mut table {
                        t.0 = std::mem::take(&mut row);
                    }
                }
                TagEnd::TableRow => {
                    if let Some(t) = &mut table {
                        t.1.push(std::mem::take(&mut row));
                    }
                }
                TagEnd::Table => {
                    if let Some((head, rows, _)) = table.take() {
                        push(&mut stack, Block::Table { head, rows });
                    }
                }
                TagEnd::Emphasis => style.italic = style.italic.saturating_sub(1),
                TagEnd::Strong => style.bold = style.bold.saturating_sub(1),
                TagEnd::Link => {
                    style.link.pop();
                }
                _ => {}
            },
            Event::Text(t) => spans.push(span(&t, &style, false)),
            Event::Code(t) => spans.push(span(&t, &style, true)),
            Event::SoftBreak => spans.push(span(" ", &style, false)),
            Event::HardBreak => spans.push(span("\n", &style, false)),
            Event::Rule => push(&mut stack, Block::Rule),
            // HTML (comments used as markers for the checks) isn't shown.
            _ => {}
        }
    }
    flush(&mut stack, &mut spans);
    match stack.into_iter().next() {
        Some(Frame::Blocks(v)) => v,
        _ => Vec::new(),
    }
}

fn span(text: &str, style: &Style, code: bool) -> Span {
    Span { text: text.to_string(), bold: style.bold > 0, italic: style.italic > 0, code, link: style.link.last().cloned() }
}
