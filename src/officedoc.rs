//! Word, OpenDocument and PowerPoint files, formatted: headings, paragraphs with bold and
//! italic, lists and tables (Word, OpenDocument text), and each slide's title and points
//! (PowerPoint, OpenDocument presentations). Read from the XML inside the zip with roxmltree,
//! and drawn with the Help Centre's renderer. Nothing in the file is run, and nothing it links
//! to is fetched.

use crate::help::doc::{Block, Span};
use roxmltree::{Document, Node, ParsingOptions};
use std::io::Read;
use std::path::Path;

/// XML parts larger than this aren't read.
const XML_MAX: u64 = 32 << 20;
/// Blocks kept from a long document.
const BLOCKS_MAX: usize = 3000;

/// What a file is, for reading it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Word,
    OdText,
    PowerPoint,
    OdPresentation,
}

pub fn kind_of(name: &str) -> Option<Kind> {
    let ext = crate::scan::ext_of(name).to_ascii_lowercase();
    Some(match ext.as_str() {
        "docx" | "docm" | "dotx" => Kind::Word,
        "odt" | "ott" => Kind::OdText,
        "pptx" | "pptm" | "ppsx" | "potx" => Kind::PowerPoint,
        "odp" | "otp" => Kind::OdPresentation,
        _ => return None,
    })
}

fn part(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Result<String, String> {
    let f = zip.by_name(name).map_err(|_| format!("{name} is missing: the file may be damaged."))?;
    if f.size() > XML_MAX {
        return Err("Too big to preview.".into());
    }
    let mut s = String::new();
    f.take(XML_MAX).read_to_string(&mut s).map_err(|e| format!("Couldn't read it: {e}"))?;
    Ok(s)
}

fn parse(xml: &str) -> Result<Document<'_>, String> {
    Document::parse_with_options(xml, ParsingOptions { allow_dtd: false, ..Default::default() }).map_err(|e| format!("Couldn't read the document: {e}"))
}

/// A document's contents as blocks. Runs on worker threads.
pub fn read(path: &Path, kind: Kind) -> Result<Vec<Block>, String> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).map_err(|e| format!("Couldn't open it: {e}"))?).map_err(|_| "Not a document Filescape can read (it isn't a zip).".to_string())?;
    let mut blocks = match kind {
        Kind::Word => word(&part(&mut zip, "word/document.xml")?)?,
        Kind::OdText | Kind::OdPresentation => opendoc(&part(&mut zip, "content.xml")?)?,
        Kind::PowerPoint => {
            // Slides in their order: slide1.xml, slide2.xml… (numbered as PowerPoint saves them).
            let mut slides: Vec<(u32, String)> = zip
                .file_names()
                .filter_map(|n| n.ok())
                .filter_map(|n| n.strip_prefix("ppt/slides/slide").and_then(|r| r.strip_suffix(".xml")).and_then(|k| k.parse().ok()).map(|k| (k, n.to_string())))
                .collect();
            slides.sort();
            let mut out = Vec::new();
            for (i, (_, name)) in slides.iter().enumerate() {
                out.extend(slide(&part(&mut zip, name)?, i + 1)?);
                if out.len() > BLOCKS_MAX {
                    break;
                }
            }
            out
        }
    };
    blocks.truncate(BLOCKS_MAX);
    if blocks.is_empty() {
        blocks.push(Block::Para(vec![Span { text: "This document has no text.".into(), italic: true, ..Default::default() }]));
    }
    Ok(blocks)
}

fn el<'a>(n: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

/// A Word on/off property (`<w:b/>`, `<w:b w:val="0"/>`).
fn on(rpr: Option<Node>, name: &str) -> bool {
    rpr.and_then(|r| el(r, name)).is_some_and(|b| !matches!(b.attributes().find(|a| a.name() == "val").map(|a| a.value()), Some("0" | "false" | "off")))
}

fn attr<'a>(n: Node<'a, 'a>, name: &str) -> Option<&'a str> {
    n.attributes().find(|a| a.name() == name).map(|a| a.value())
}

/// Join neighbouring spans that look the same.
fn push(spans: &mut Vec<Span>, s: Span) {
    if s.text.is_empty() {
        return;
    }
    match spans.last_mut() {
        Some(l) if l.bold == s.bold && l.italic == s.italic && l.link == s.link => l.text.push_str(&s.text),
        _ => spans.push(s),
    }
}

/// A Word paragraph's runs.
fn word_spans(p: Node) -> Vec<Span> {
    let mut spans = Vec::new();
    for r in p.descendants().filter(|n| n.is_element() && n.tag_name().name() == "r") {
        let rpr = el(r, "rPr");
        let (bold, italic) = (on(rpr, "b"), on(rpr, "i"));
        let mut text = String::new();
        for c in r.children().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "t" => text.push_str(c.text().unwrap_or("")),
                "tab" => text.push('\t'),
                "br" | "cr" => text.push(' '),
                _ => {}
            }
        }
        push(&mut spans, Span { text, bold, italic, ..Default::default() });
    }
    spans
}

fn word(xml: &str) -> Result<Vec<Block>, String> {
    let doc = parse(xml)?;
    let body = doc.descendants().find(|n| n.is_element() && n.tag_name().name() == "body").ok_or("The document has no body.")?;
    let mut out: Vec<Block> = Vec::new();
    for n in body.children().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "p" => {
                let ppr = el(n, "pPr");
                let style = ppr.and_then(|p| el(p, "pStyle")).and_then(|s| attr(s, "val")).unwrap_or("").to_ascii_lowercase();
                let spans = word_spans(n);
                let level = if style == "title" {
                    Some(1)
                } else if style == "subtitle" {
                    Some(2)
                } else {
                    style.strip_prefix("heading").and_then(|d| d.trim().parse::<u8>().ok()).map(|d| d.clamp(1, 6))
                };
                if spans.iter().all(|s| s.text.trim().is_empty()) {
                    continue;
                }
                if let Some(level) = level {
                    out.push(Block::Heading { level, spans, id: String::new() });
                } else if ppr.and_then(|p| el(p, "numPr")).is_some() || style.contains("list") {
                    // A list item: joins the list just before it.
                    match out.last_mut() {
                        Some(Block::List { items, .. }) => items.push(vec![Block::Para(spans)]),
                        _ => out.push(Block::List { start: None, items: vec![vec![Block::Para(spans)]] }),
                    }
                } else {
                    out.push(Block::Para(spans));
                }
            }
            "tbl" => {
                let rows: Vec<Vec<Vec<Span>>> = n
                    .children()
                    .filter(|r| r.is_element() && r.tag_name().name() == "tr")
                    .map(|r| {
                        r.children()
                            .filter(|c| c.is_element() && c.tag_name().name() == "tc")
                            .map(|c| {
                                let mut spans = Vec::new();
                                for p in c.descendants().filter(|p| p.is_element() && p.tag_name().name() == "p") {
                                    if !spans.is_empty() {
                                        push(&mut spans, Span { text: " ".into(), ..Default::default() });
                                    }
                                    word_spans(p).into_iter().for_each(|s| push(&mut spans, s));
                                }
                                spans
                            })
                            .collect()
                    })
                    .collect();
                if let Some((head, rest)) = rows.split_first() {
                    out.push(Block::Table { head: head.clone(), rows: rest.to_vec() });
                }
            }
            _ => {}
        }
        if out.len() > BLOCKS_MAX {
            break;
        }
    }
    Ok(out)
}

/// The text of an OpenDocument paragraph.
fn od_text(p: Node) -> String {
    let mut s = String::new();
    for d in p.descendants() {
        if d.is_text() {
            s.push_str(d.text().unwrap_or(""));
        } else if d.is_element() {
            match d.tag_name().name() {
                "s" => s.push_str(&" ".repeat(attr(d, "c").and_then(|c| c.parse().ok()).unwrap_or(1))),
                "tab" => s.push('\t'),
                "line-break" => s.push(' '),
                _ => {}
            }
        }
    }
    s
}

fn plain(text: String) -> Vec<Span> {
    vec![Span { text, ..Default::default() }]
}

fn opendoc(xml: &str) -> Result<Vec<Block>, String> {
    let doc = parse(xml)?;
    let mut out = Vec::new();
    let body = doc.descendants().find(|n| n.is_element() && matches!(n.tag_name().name(), "text" | "presentation") && n.parent().is_some_and(|p| p.tag_name().name() == "body"));
    let Some(body) = body else { return Ok(out) };
    let mut page = 0;
    fn walk(n: Node, out: &mut Vec<Block>, page: &mut usize) {
        for c in n.children().filter(|c| c.is_element()) {
            if out.len() > BLOCKS_MAX {
                return;
            }
            match c.tag_name().name() {
                "h" => {
                    let level = attr(c, "outline-level").and_then(|l| l.parse().ok()).unwrap_or(1u8).clamp(1, 6);
                    out.push(Block::Heading { level, spans: plain(od_text(c)), id: String::new() });
                }
                "p" => {
                    let t = od_text(c);
                    if !t.trim().is_empty() {
                        out.push(Block::Para(plain(t)));
                    }
                }
                "list" => {
                    let items: Vec<Vec<Block>> = c
                        .children()
                        .filter(|i| i.is_element() && i.tag_name().name() == "list-item")
                        .map(|i| i.descendants().filter(|p| p.is_element() && p.tag_name().name() == "p").map(|p| Block::Para(plain(od_text(p)))).collect())
                        .filter(|v: &Vec<Block>| !v.is_empty())
                        .collect();
                    if !items.is_empty() {
                        out.push(Block::List { start: None, items });
                    }
                }
                "table" => {
                    let rows: Vec<Vec<Vec<Span>>> = c
                        .descendants()
                        .filter(|r| r.is_element() && r.tag_name().name() == "table-row")
                        .map(|r| r.children().filter(|x| x.is_element() && x.tag_name().name() == "table-cell").map(|x| plain(od_text(x))).collect())
                        .collect();
                    if let Some((head, rest)) = rows.split_first() {
                        out.push(Block::Table { head: head.clone(), rows: rest.to_vec() });
                    }
                }
                "page" => {
                    *page += 1;
                    // The title placeholder's words, or the page's own name when it isn't "pageN".
                    let title = c.children().find(|f| f.is_element() && attr(*f, "class") == Some("title")).map(|f| od_text(f).trim().to_string()).filter(|t| !t.is_empty());
                    let name = title.or_else(|| attr(c, "name").filter(|n| !n.starts_with("page")).map(str::to_string)).map(|n| format!(": {n}")).unwrap_or_default();
                    out.push(Block::Heading { level: 2, spans: plain(format!("Slide {page}{name}")), id: String::new() });
                    walk(c, out, page);
                }
                // A slide's title is in its heading already.
                "frame" if attr(c, "class") == Some("title") => {}
                "section" | "frame" | "text-box" | "g" | "custom-shape" | "rect" => walk(c, out, page),
                _ => {}
            }
        }
    }
    walk(body, &mut out, &mut page);
    Ok(out)
}

/// One PowerPoint slide: "Slide N: title", then its other text as points.
fn slide(xml: &str, n: usize) -> Result<Vec<Block>, String> {
    let doc = parse(xml)?;
    let mut title = String::new();
    let mut points = Vec::new();
    for sp in doc.descendants().filter(|s| s.is_element() && s.tag_name().name() == "sp") {
        let ph = sp.descendants().find(|p| p.is_element() && p.tag_name().name() == "ph").and_then(|p| attr(p, "type"));
        let paras: Vec<String> = sp
            .descendants()
            .filter(|p| p.is_element() && p.tag_name().name() == "p" && p.tag_name().namespace().is_some_and(|ns| ns.contains("drawingml")))
            .map(|p| p.descendants().filter(|t| t.is_element() && t.tag_name().name() == "t").filter_map(|t| t.text()).collect::<String>())
            .filter(|t| !t.trim().is_empty())
            .collect();
        if matches!(ph, Some("title" | "ctrTitle")) && title.is_empty() {
            title = paras.join(" ");
        } else {
            points.extend(paras);
        }
    }
    let mut out = vec![Block::Heading { level: 2, spans: plain(if title.is_empty() { format!("Slide {n}") } else { format!("Slide {n}: {title}") }), id: String::new() }];
    if !points.is_empty() {
        out.push(Block::List { start: None, items: points.into_iter().map(|p| vec![Block::Para(plain(p))]).collect() });
    }
    Ok(out)
}

/// The picture PowerPoint and LibreOffice save of the first slide or page, if there is one.
pub fn saved_thumbnail(path: &Path) -> Option<image::DynamicImage> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    for name in ["docProps/thumbnail.jpeg", "docProps/thumbnail.jpg", "docProps/thumbnail.png", "Thumbnails/thumbnail.png"] {
        if let Ok(f) = zip.by_name(name) {
            if f.size() > 8 << 20 {
                return None;
            }
            let mut buf = Vec::new();
            f.take(8 << 20).read_to_end(&mut buf).ok()?;
            return image::load_from_memory(&buf).ok();
        }
    }
    None
}

/// Blocks as plain text, for tiles.
pub fn plain_text(blocks: &[Block]) -> String {
    fn spans(s: &[Span]) -> String {
        s.iter().map(|x| x.text.as_str()).collect()
    }
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading { spans: s, .. } | Block::Para(s) => {
                out.push_str(&spans(s));
                out.push('\n');
            }
            Block::List { items, .. } => {
                for i in items {
                    out.push_str("• ");
                    out.push_str(&plain_text(i));
                }
            }
            Block::Table { head, rows } => {
                for r in std::iter::once(head).chain(rows) {
                    out.push_str(&r.iter().map(|c| spans(c)).collect::<Vec<_>>().join("  "));
                    out.push('\n');
                }
            }
            _ => {}
        }
        if out.len() > 64 << 10 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(path: &Path, files: &[(&str, &str)]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (n, body) in files {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn word_headings_lists_tables() {
        let p = std::env::temp_dir().join(format!("fs-office-{}.docx", std::process::id()));
        zip_with(&p, &[("word/document.xml", r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Plan</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Some </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>bold</w:t></w:r><w:r><w:rPr><w:b w:val="0"/></w:rPr><w:t> text</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/></w:numPr></w:pPr><w:r><w:t>one</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/></w:numPr></w:pPr><w:r><w:t>two</w:t></w:r></w:p>
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>2</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
</w:body></w:document>"#)]);
        let b = read(&p, Kind::Word);
        let _ = std::fs::remove_file(&p);
        let b = b.unwrap();
        assert!(matches!(&b[0], Block::Heading { level: 1, spans, .. } if spans[0].text == "Plan"));
        match &b[1] {
            Block::Para(s) => {
                assert_eq!(s.len(), 3);
                assert!(s[1].bold && s[1].text == "bold" && !s[2].bold);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&b[2], Block::List { items, .. } if items.len() == 2));
        assert!(matches!(&b[3], Block::Table { head, rows } if head.len() == 2 && rows.len() == 1));
        assert!(plain_text(&b).starts_with("Plan\nSome bold text\n• one\n• two\nA  B\n1  2"));
    }

    #[test]
    fn slides_in_order() {
        let p = std::env::temp_dir().join(format!("fs-office-{}.pptx", std::process::id()));
        let s = |title: &str, point: &str| {
            format!(r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree>
<p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>{title}</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:txBody><a:p><a:r><a:t>{point}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#)
        };
        let (s1, s2, s10) = (s("Intro", "Hello"), s("Middle", "More"), s("End", "Bye"));
        zip_with(&p, &[("ppt/slides/slide10.xml", &s10), ("ppt/slides/slide2.xml", &s2), ("ppt/slides/slide1.xml", &s1)]);
        let b = read(&p, Kind::PowerPoint);
        let _ = std::fs::remove_file(&p);
        let text = plain_text(&b.unwrap());
        assert_eq!(text, "Slide 1: Intro\n• Hello\nSlide 2: Middle\n• More\nSlide 3: End\n• Bye\n");
    }
}
