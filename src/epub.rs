//! EPUB books: the chapters in reading order, formatted (headings, paragraphs with bold and
//! italic, lists, quotes, tables), the cover picture, and the title and author. Our own small
//! reader over zip and roxmltree (the `epub` crate is GPL). Nothing in the book is run and nothing
//! it links to is fetched.

use crate::help::doc::{Block, Span};
use roxmltree::{Document, Node, ParsingOptions};
use std::io::Read;
use std::path::Path;

/// Parts larger than this aren't read.
const PART_MAX: u64 = 8 << 20;
/// Blocks kept from a long book.
const BLOCKS_MAX: usize = 4000;

pub struct Book {
    pub blocks: Vec<Block>,
    pub facts: Vec<(&'static str, String)>,
    /// Where the cover picture is in the zip.
    cover: Option<String>,
}

type Zip = zip::ZipArchive<std::fs::File>;

fn part(zip: &mut Zip, name: &str) -> Option<Vec<u8>> {
    let f = zip.by_name(name).ok()?;
    if f.size() > PART_MAX {
        return None;
    }
    let mut buf = Vec::new();
    f.take(PART_MAX).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// XHTML with the named entities HTML allows and XML doesn't know, written as numbers.
fn xml_text(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    if !s.contains('&') {
        return s.into_owned();
    }
    const NAMED: [(&str, &str); 14] = [
        ("&nbsp;", "&#160;"),
        ("&mdash;", "&#8212;"),
        ("&ndash;", "&#8211;"),
        ("&hellip;", "&#8230;"),
        ("&lsquo;", "&#8216;"),
        ("&rsquo;", "&#8217;"),
        ("&ldquo;", "&#8220;"),
        ("&rdquo;", "&#8221;"),
        ("&copy;", "&#169;"),
        ("&reg;", "&#174;"),
        ("&trade;", "&#8482;"),
        ("&eacute;", "&#233;"),
        ("&shy;", "&#173;"),
        ("&bull;", "&#8226;"),
    ];
    let mut s = s.into_owned();
    for (a, b) in NAMED {
        if s.contains(a) {
            s = s.replace(a, b);
        }
    }
    s
}

fn parse(text: &str) -> Option<Document<'_>> {
    Document::parse_with_options(text, ParsingOptions { allow_dtd: true, ..Default::default() }).ok()
}

fn el<'a>(n: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    n.descendants().find(|c| c.is_element() && c.tag_name().name() == name)
}

fn attr<'a>(n: Node<'a, 'a>, name: &str) -> Option<&'a str> {
    n.attributes().find(|a| a.name() == name).map(|a| a.value())
}

/// A path in the zip relative to the file that names it, with `..` and `%20` worked out.
fn join(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or("");
    let mut parts: Vec<&str> = base.rsplit_once('/').map(|(d, _)| d.split('/').collect()).unwrap_or_default();
    for p in href.split('/') {
        match p {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            p => parts.push(p),
        }
    }
    let joined = parts.join("/");
    if joined.contains('%') { percent_decode(&joined) } else { joined }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%' && i + 2 < b.len() && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn read(path: &Path) -> Result<Book, String> {
    let b = open(path, BLOCKS_MAX)?;
    if b.blocks.is_empty() {
        return Err("This book has no text Filescape can show.".into());
    }
    Ok(b)
}

/// The book's details, cover and first `limit` blocks or so (whole chapters).
pub fn open(path: &Path, limit: usize) -> Result<Book, String> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).map_err(|e| format!("Couldn't open it: {e}"))?).map_err(|_| "Not a book Filescape can read (it isn't a zip).".to_string())?;
    let container = part(&mut zip, "META-INF/container.xml").ok_or("This book has no table of contents (META-INF/container.xml is missing).")?;
    let container = xml_text(&container);
    let opf_path = parse(&container).and_then(|d| el(d.root(), "rootfile").and_then(|r| attr(r, "full-path")).map(str::to_string)).ok_or("This book's contents list couldn't be read.")?;
    let opf = xml_text(&part(&mut zip, &opf_path).ok_or("This book's contents list is missing.")?);
    let opf = parse(&opf).ok_or("This book's contents list couldn't be read.")?;
    let root = opf.root();

    // Title, author and so on.
    let mut facts = Vec::new();
    for (k, tag) in [("Title", "title"), ("Author", "creator"), ("Publisher", "publisher"), ("Language", "language")] {
        if let Some(t) = el(root, tag).and_then(|n| n.text()).map(str::trim).filter(|t| !t.is_empty()) {
            facts.push((k, t.to_string()));
        }
    }

    // id -> (href, media type, properties)
    let manifest: Vec<(&str, &str, &str, &str)> = root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "item")
        .map(|n| (attr(n, "id").unwrap_or(""), attr(n, "href").unwrap_or(""), attr(n, "media-type").unwrap_or(""), attr(n, "properties").unwrap_or("")))
        .collect();
    let by_id = |id: &str| manifest.iter().find(|m| m.0 == id);
    // The cover: EPUB 3 marks it; EPUB 2 names it in a <meta name="cover">.
    let cover = manifest
        .iter()
        .find(|m| m.3.split_whitespace().any(|p| p == "cover-image"))
        .or_else(|| {
            let id = root.descendants().find(|n| n.is_element() && n.tag_name().name() == "meta" && attr(*n, "name") == Some("cover")).and_then(|n| attr(n, "content"))?;
            by_id(id)
        })
        .filter(|m| m.2.starts_with("image/"))
        .map(|m| join(&opf_path, m.1));

    let spine: Vec<String> = root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "itemref" && attr(*n, "linear") != Some("no"))
        .filter_map(|n| by_id(attr(n, "idref")?))
        .filter(|m| m.2.contains("html"))
        .map(|m| join(&opf_path, m.1))
        .collect();
    facts.push(("Chapters", spine.len().to_string()));

    let mut blocks = Vec::new();
    for name in &spine {
        if blocks.len() >= limit {
            break;
        }
        let Some(raw) = part(&mut zip, name) else { continue };
        let text = xml_text(&raw);
        match parse(&text) {
            Some(d) => {
                if let Some(body) = el(d.root(), "body") {
                    walk(body, &mut blocks);
                }
            }
            // Not well-formed: its words without the markup.
            None => blocks.extend(strip_tags(&text).lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| Block::Para(plain(l.to_string())))),
        }
    }
    blocks.truncate(limit);
    Ok(Book { blocks, facts, cover })
}

impl Book {
    /// The cover picture, if the book has one.
    pub fn cover(&self, path: &Path) -> Option<image::DynamicImage> {
        let name = self.cover.as_ref()?;
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
        image::load_from_memory(&part(&mut zip, name)?).ok()
    }
}

/// The cover of the book at `path`, for thumbnails.
pub fn cover(path: &Path) -> Option<image::DynamicImage> {
    open(path, 0).ok()?.cover(path)
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in s.chars() {
        match c {
            '<' => inside = true,
            '>' => {
                inside = false;
                out.push(' ');
            }
            c if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

fn plain(text: String) -> Vec<Span> {
    vec![Span { text, ..Default::default() }]
}

/// Join neighbouring spans that look the same, and runs of white space.
fn push(spans: &mut Vec<Span>, s: Span) {
    if s.text.is_empty() {
        return;
    }
    match spans.last_mut() {
        Some(l) if l.bold == s.bold && l.italic == s.italic && l.code == s.code && l.link == s.link => l.text.push_str(&s.text),
        _ => spans.push(s),
    }
}

/// The words in an element, with bold, italic and code kept.
fn spans(n: Node) -> Vec<Span> {
    fn go(n: Node, bold: bool, italic: bool, code: bool, out: &mut Vec<Span>) {
        for c in n.children() {
            if c.is_text() {
                let t: String = c.text().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
                let lead = c.text().is_some_and(|t| t.starts_with(char::is_whitespace));
                let trail = c.text().is_some_and(|t| t.ends_with(char::is_whitespace));
                let mut text = String::new();
                if lead && !out.last().is_some_and(|s: &Span| s.text.ends_with(' ')) {
                    text.push(' ');
                }
                text.push_str(&t);
                if trail && !t.is_empty() {
                    text.push(' ');
                }
                push(out, Span { text, bold, italic, code, ..Default::default() });
            } else if c.is_element() {
                match c.tag_name().name() {
                    "b" | "strong" => go(c, true, italic, code, out),
                    "i" | "em" | "cite" => go(c, bold, true, code, out),
                    "code" | "tt" | "kbd" => go(c, bold, italic, true, out),
                    "br" => push(out, Span { text: " ".into(), ..Default::default() }),
                    "script" | "style" | "img" | "svg" | "math" => {}
                    _ => go(c, bold, italic, code, out),
                }
            }
        }
    }
    let mut out = Vec::new();
    go(n, false, false, false, &mut out);
    if let Some(f) = out.first_mut() {
        f.text = f.text.trim_start().to_string();
    }
    if let Some(l) = out.last_mut() {
        l.text = l.text.trim_end().to_string();
    }
    out.retain(|s| !s.text.is_empty());
    out
}

const BLOCK_TAGS: [&str; 16] = ["p", "div", "section", "article", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "blockquote", "table", "pre", "hr"];

fn has_blocks(n: Node) -> bool {
    n.children().any(|c| c.is_element() && (BLOCK_TAGS.contains(&c.tag_name().name()) || matches!(c.tag_name().name(), "header" | "footer" | "aside" | "figure" | "nav")))
}

fn walk(n: Node, out: &mut Vec<Block>) {
    for c in n.children() {
        if out.len() > BLOCKS_MAX {
            return;
        }
        if c.is_text() {
            // Loose words between blocks.
            let t = c.text().unwrap_or("").trim();
            if !t.is_empty() {
                out.push(Block::Para(plain(t.to_string())));
            }
            continue;
        }
        if !c.is_element() {
            continue;
        }
        let tag = c.tag_name().name();
        match tag {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let s = spans(c);
                if !s.is_empty() {
                    out.push(Block::Heading { level: tag[1..].parse().unwrap_or(2), spans: s, id: String::new() });
                }
            }
            "ul" | "ol" => {
                let items: Vec<Vec<Block>> = c
                    .children()
                    .filter(|i| i.is_element() && i.tag_name().name() == "li")
                    .map(|i| {
                        if has_blocks(i) {
                            let mut b = Vec::new();
                            walk(i, &mut b);
                            b
                        } else {
                            let s = spans(i);
                            if s.is_empty() { Vec::new() } else { vec![Block::Para(s)] }
                        }
                    })
                    .filter(|b| !b.is_empty())
                    .collect();
                if !items.is_empty() {
                    let start = (tag == "ol").then(|| attr(c, "start").and_then(|s| s.parse().ok()).unwrap_or(1));
                    out.push(Block::List { start, items });
                }
            }
            "blockquote" => {
                let mut b = Vec::new();
                if has_blocks(c) {
                    walk(c, &mut b);
                } else {
                    let s = spans(c);
                    if !s.is_empty() {
                        b.push(Block::Para(s));
                    }
                }
                if !b.is_empty() {
                    out.push(Block::Quote(b));
                }
            }
            "pre" => {
                let t: String = c.descendants().filter(|d| d.is_text()).map(|d| d.text().unwrap_or("")).collect();
                out.push(Block::Code { lang: String::new(), text: t.trim_end().to_string() });
            }
            "table" => {
                let rows: Vec<Vec<Vec<Span>>> = c
                    .descendants()
                    .filter(|r| r.is_element() && r.tag_name().name() == "tr")
                    .map(|r| r.children().filter(|x| x.is_element() && matches!(x.tag_name().name(), "td" | "th")).map(spans).collect())
                    .collect();
                if let Some((head, rest)) = rows.split_first() {
                    out.push(Block::Table { head: head.clone(), rows: rest.to_vec() });
                }
            }
            "hr" => out.push(Block::Rule),
            "script" | "style" | "img" | "svg" | "math" | "nav" => {}
            _ if has_blocks(c) => walk(c, out),
            _ => {
                let s = spans(c);
                if !s.is_empty() {
                    out.push(Block::Para(s));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_a_book() {
        let dir = std::env::temp_dir().join(format!("filescape-epub-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("b.epub");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("mimetype", o).unwrap();
        z.write_all(b"application/epub+zip").unwrap();
        z.start_file("META-INF/container.xml", o).unwrap();
        z.write_all(br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
        z.start_file("OEBPS/content.opf", o).unwrap();
        z.write_all(br#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>A Tale</dc:title><dc:creator>Ann Author</dc:creator></metadata>
<manifest><item id="c1" href="Text/ch%201.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="Text/ch2.xhtml" media-type="application/xhtml+xml"/><item id="cv" href="cover.png" media-type="image/png" properties="cover-image"/></manifest>
<spine><itemref idref="c2"/><itemref idref="c1"/></spine></package>"#).unwrap();
        z.start_file("OEBPS/Text/ch 1.xhtml", o).unwrap();
        z.write_all(br#"<?xml version="1.0"?><!DOCTYPE html><html xmlns="http://www.w3.org/1999/xhtml"><body><h2>Two</h2><div><p>It was <em>late</em>&nbsp;and <b>dark</b>.</p><ul><li>one</li><li><p>two</p></li></ul></div></body></html>"#).unwrap();
        z.start_file("OEBPS/Text/ch2.xhtml", o).unwrap();
        z.write_all(br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>One</h1><blockquote>Said so.</blockquote></body></html>"#).unwrap();
        z.start_file("OEBPS/cover.png", o).unwrap();
        let mut png = Vec::new();
        image::DynamicImage::new_rgb8(3, 4).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        z.write_all(&png).unwrap();
        z.finish().unwrap();

        let b = read(&p).unwrap();
        assert!(b.facts.contains(&("Title", "A Tale".into())));
        assert!(b.facts.contains(&("Author", "Ann Author".into())));
        assert!(b.facts.contains(&("Chapters", "2".into())));
        // Spine order, not manifest order.
        assert!(matches!(&b.blocks[0], Block::Heading { level: 1, spans, .. } if spans[0].text == "One"));
        assert!(matches!(&b.blocks[1], Block::Quote(_)));
        assert!(matches!(&b.blocks[2], Block::Heading { level: 2, .. }));
        let Block::Para(s) = &b.blocks[3] else { panic!("{:?}", b.blocks[3]) };
        let text: String = s.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(text, "It was late and dark.");
        assert!(s.iter().any(|x| x.italic && x.text == "late"));
        assert!(s.iter().any(|x| x.bold && x.text == "dark"));
        assert!(matches!(&b.blocks[4], Block::List { start: None, items } if items.len() == 2));
        let c = b.cover(&p).unwrap();
        assert_eq!((c.width(), c.height()), (3, 4));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn paths_join() {
        assert_eq!(join("OEBPS/content.opf", "Text/a.xhtml#x"), "OEBPS/Text/a.xhtml");
        assert_eq!(join("OEBPS/Text/a.xhtml", "../Images/c%20d.jpg"), "OEBPS/Images/c d.jpg");
        assert_eq!(join("content.opf", "a.xhtml"), "a.xhtml");
    }
}
