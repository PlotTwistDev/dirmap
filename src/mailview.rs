//! Email, contacts and calendars: a message (.eml, .emlx) with its headers, text and
//! attachments; a mailbox (.mbox) as a list of its messages; vCards (.vcf) as contact cards or
//! a list; iCalendar files (.ics) as events.
//!
//! Read here, without help from a mail program. Nothing in a message is run or fetched: HTML is
//! turned into plain text (pictures and links in it are never loaded), and attachments are only
//! listed.

use crate::help::doc::{Block, Span};
use std::io::{BufRead, Read};
use std::path::Path;

/// Messages, cards and calendars larger than this aren't read whole.
const READ_MAX: u64 = 32 << 20;
/// How much of a mailbox is scanned for its list of messages.
const MBOX_SCAN: u64 = 512 << 20;
/// Rows kept in a list of messages, contacts or events.
const ROWS_MAX: usize = 2000;
/// Nested parts followed in a message.
const DEPTH_MAX: usize = 8;
const PARTS_MAX: usize = 300;

/// What's read and shown.
pub struct Mail {
    pub blocks: Vec<Block>,
    pub facts: Vec<(&'static str, String)>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Message,
    Mailbox,
    Contacts,
    Calendar,
}

pub fn kind_of(name: &str) -> Option<Kind> {
    Some(match crate::scan::ext_of(name).to_ascii_lowercase().as_str() {
        "eml" | "emlx" => Kind::Message,
        "mbox" | "mbx" => Kind::Mailbox,
        "vcf" | "vcard" => Kind::Contacts,
        "ics" | "ical" | "ifb" => Kind::Calendar,
        _ => return None,
    })
}

pub fn read(path: &Path) -> Result<Mail, String> {
    let kind = path.file_name().and_then(|n| kind_of(&n.to_string_lossy())).ok_or("Not an email, contact or calendar file")?;
    match kind {
        Kind::Mailbox => mailbox(path),
        Kind::Message => {
            let raw = slurp(path)?;
            Ok(message(emlx_body(&raw)))
        }
        Kind::Contacts => Ok(contacts(&String::from_utf8_lossy(&slurp(path)?))),
        Kind::Calendar => Ok(calendar(&String::from_utf8_lossy(&slurp(path)?))),
    }
}

/// The first lines, for a tile on the map.
pub fn snippet(path: &Path) -> Option<String> {
    let m = read(path).ok()?;
    let mut out = String::new();
    for (k, v) in m.facts.iter().take(3) {
        out.push_str(&format!("{k}: {v}\n"));
    }
    out.push('\n');
    out.push_str(&crate::officedoc::plain_text(&m.blocks));
    Some(out)
}

fn slurp(path: &Path) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(READ_MAX).read_to_end(&mut buf)).map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Apple Mail's .emlx: a line with the message's length, the message, then a property list.
fn emlx_body(raw: &[u8]) -> &[u8] {
    let Some(nl) = raw.iter().take(24).position(|&b| b == b'\n') else { return raw };
    let first = String::from_utf8_lossy(&raw[..nl]);
    match first.trim().parse::<usize>() {
        Ok(n) if !first.trim().is_empty() => {
            let rest = &raw[nl + 1..];
            &rest[..n.min(rest.len())]
        }
        _ => raw,
    }
}

/// 12,345.
fn group(n: u64) -> String {
    crate::inspect::group(n)
}

fn plain(text: impl Into<String>) -> Vec<Span> {
    vec![Span { text: text.into(), ..Default::default() }]
}

fn bold(text: impl Into<String>) -> Span {
    Span { text: text.into(), bold: true, ..Default::default() }
}

// ---- Messages ----

type Headers = Vec<(String, String)>;

/// A message's (or part's) headers, unfolded, and its body.
fn split_head(raw: &[u8]) -> (Headers, &[u8]) {
    // The headers end at the first empty line.
    let mut end = raw.len();
    let mut body = raw.len();
    let mut i = 0;
    while i < raw.len() {
        let line_end = raw[i..].iter().position(|&b| b == b'\n').map_or(raw.len(), |p| i + p);
        let line = &raw[i..line_end];
        if line.is_empty() || line == b"\r" {
            end = i;
            body = (line_end + 1).min(raw.len());
            break;
        }
        i = line_end + 1;
    }
    let mut headers: Headers = Vec::new();
    for line in String::from_utf8_lossy(&raw[..end]).lines() {
        let line = line.trim_end_matches('\r');
        if line.starts_with([' ', '\t']) {
            if let Some((_, v)) = headers.last_mut() {
                v.push(' ');
                v.push_str(line.trim());
            }
        } else if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    (headers, &raw[body..])
}

fn header<'a>(h: &'a Headers, name: &str) -> Option<&'a str> {
    h.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty())
}

/// Text in a character set, by its name.
fn decode_charset(bytes: &[u8], charset: &str) -> String {
    match encoding_rs::Encoding::for_label(charset.trim().as_bytes()) {
        Some(enc) => enc.decode_without_bom_handling(bytes).0.into_owned(),
        None => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// `=?charset?B?...?=` and `=?charset?Q?...?=` words in a header (RFC 2047).
fn decode_words(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    let mut last_was_word = false;
    while let Some(start) = rest.find("=?") {
        let before = &rest[..start];
        let after = &rest[start + 2..];
        // charset ? encoding ? text ?=
        let parsed = (|| {
            let (charset, r) = after.split_once('?')?;
            let (enc, r) = r.split_once('?')?;
            let end = r.find("?=")?;
            let text = &r[..end];
            let bytes = match enc {
                "B" | "b" => base64(text.as_bytes()),
                "Q" | "q" => quoted_printable(text.replace('_', " ").as_bytes()),
                _ => return None,
            };
            // A language can follow the charset after '*' (RFC 2231).
            let charset = charset.split('*').next().unwrap_or(charset);
            Some((decode_charset(&bytes, charset), charset.len() + enc.len() + end + 4))
        })();
        match parsed {
            Some((text, used)) => {
                // Space between two encoded words is dropped.
                if !(last_was_word && before.trim().is_empty()) {
                    out.push_str(before);
                }
                out.push_str(&text);
                rest = &after[used..];
                last_was_word = true;
            }
            None => {
                out.push_str(before);
                out.push_str("=?");
                rest = after;
                last_was_word = false;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Base64, skipping line breaks and anything else that isn't in its alphabet.
fn base64(s: &[u8]) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for &c in s {
        if c == b'=' {
            break;
        }
        let Some(v) = val(c) else { continue };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

fn quoted_printable(s: &[u8]) -> Vec<u8> {
    let hex = |c: u8| (c as char).to_digit(16);
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'=' {
            // A soft line break.
            if s.get(i + 1) == Some(&b'\n') {
                i += 2;
                continue;
            }
            if s.get(i + 1) == Some(&b'\r') && s.get(i + 2) == Some(&b'\n') {
                i += 3;
                continue;
            }
            if let (Some(a), Some(b)) = (s.get(i + 1).and_then(|&c| hex(c)), s.get(i + 2).and_then(|&c| hex(c))) {
                out.push((a * 16 + b) as u8);
                i += 3;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

/// `type/subtype; key=value; ...`: the lower-case value and its parameters (keys lower case,
/// RFC 2231 continuations and encodings joined and decoded).
fn params(v: &str) -> (String, Vec<(String, String)>) {
    let mut parts = Vec::new();
    let (mut cur, mut quoted) = (String::new(), false);
    for c in v.chars() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    parts.push(cur);
    let value = parts[0].trim().to_ascii_lowercase();
    // name*0*=..., name*1=... are pieces of one value.
    let mut pieces: Vec<(String, usize, bool, String)> = Vec::new();
    for p in &parts[1..] {
        let Some((k, v)) = p.split_once('=') else { continue };
        let k = k.trim().to_ascii_lowercase();
        let (k, encoded) = match k.strip_suffix('*') {
            Some(k) => (k.to_string(), true),
            None => (k, false),
        };
        let (name, n) = match k.rsplit_once('*') {
            Some((name, n)) if n.parse::<usize>().is_ok() => (name.to_string(), n.parse().unwrap_or(0)),
            _ => (k, 0),
        };
        pieces.push((name, n, encoded, v.trim().to_string()));
    }
    pieces.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    let mut out: Vec<(String, String)> = Vec::new();
    let mut charset = String::new();
    for (name, n, encoded, v) in pieces {
        let text = if encoded {
            // charset'language'percent-encoded, the charset on the first piece only.
            let raw = if n == 0 {
                let mut it = v.splitn(3, '\'');
                match (it.next(), it.next(), it.next()) {
                    (Some(cs), Some(_), Some(rest)) => {
                        charset = cs.to_string();
                        rest.to_string()
                    }
                    _ => v.clone(),
                }
            } else {
                v.clone()
            };
            decode_charset(&percent(&raw), if charset.is_empty() { "utf-8" } else { &charset })
        } else {
            decode_words(&v)
        };
        match out.last_mut() {
            Some((k, val)) if *k == name && n > 0 => val.push_str(&text),
            _ => out.push((name, text)),
        }
    }
    (value, out)
}

fn percent(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let (Some(a), Some(c)) = (b.get(i + 1).and_then(|&c| (c as char).to_digit(16)), b.get(i + 2).and_then(|&c| (c as char).to_digit(16)))
        {
            out.push((a * 16 + c) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

fn param<'a>(p: &'a [(String, String)], name: &str) -> Option<&'a str> {
    p.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty())
}

/// What's found walking a message's parts.
#[derive(Default)]
struct Walk {
    plain: Option<String>,
    html: Option<String>,
    /// Name, decoded size, type.
    attachments: Vec<(String, usize, String)>,
    parts: usize,
}

fn walk(headers: &Headers, body: &[u8], depth: usize, w: &mut Walk) {
    w.parts += 1;
    if w.parts > PARTS_MAX || depth > DEPTH_MAX {
        return;
    }
    let (ctype, cparams) = params(header(headers, "content-type").unwrap_or("text/plain"));
    let ctype = if ctype.is_empty() { "text/plain".to_string() } else { ctype };
    let (disp, dparams) = params(header(headers, "content-disposition").unwrap_or(""));
    let name = param(&dparams, "filename").or_else(|| param(&cparams, "name")).map(str::to_string);
    if ctype.starts_with("multipart/") {
        let Some(boundary) = param(&cparams, "boundary") else { return };
        for part in split_parts(body, boundary) {
            let (h, b) = split_head(part);
            walk(&h, b, depth + 1, w);
        }
        return;
    }
    let decoded = || match header(headers, "content-transfer-encoding").map(|e| e.trim().to_ascii_lowercase()).as_deref() {
        Some("base64") => base64(body),
        Some("quoted-printable") => quoted_printable(body),
        _ => body.to_vec(),
    };
    let attached = disp == "attachment" || name.is_some();
    let text = |b: Vec<u8>| decode_charset(&b, param(&cparams, "charset").unwrap_or("utf-8"));
    if !attached && ctype == "text/plain" && w.plain.is_none() {
        w.plain = Some(text(decoded()));
    } else if !attached && ctype == "text/html" && w.html.is_none() {
        w.html = Some(text(decoded()));
    } else if ctype == "message/rfc822" && !attached {
        let (h, _) = split_head(body);
        let subject = header(&h, "subject").map(decode_words).unwrap_or_else(|| "message".into());
        w.attachments.push((format!("{subject}.eml"), body.len(), ctype));
    } else if attached || !ctype.starts_with("text/") {
        let size = decoded().len();
        let name = name.unwrap_or_else(|| format!("(unnamed {ctype})"));
        w.attachments.push((name, size, ctype));
    }
}

/// The parts between `--boundary` lines.
fn split_parts<'a>(body: &'a [u8], boundary: &str) -> Vec<&'a [u8]> {
    let delim = format!("--{boundary}");
    let d = delim.as_bytes();
    let mut starts = Vec::new();
    let mut i = 0;
    // Delimiters start a line.
    while i + d.len() <= body.len() {
        if (i == 0 || body[i - 1] == b'\n') && &body[i..i + d.len()] == d {
            starts.push(i);
            i += d.len();
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    for pair in starts.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let line_end = body[a..b].iter().position(|&c| c == b'\n').map_or(b, |p| a + p + 1);
        if body[a + d.len()..].starts_with(b"--") {
            break;
        }
        // The line break before the next delimiter belongs to it.
        let mut end = b;
        if end > line_end && body[end - 1] == b'\n' {
            end -= 1;
            if end > line_end && body[end - 1] == b'\r' {
                end -= 1;
            }
        }
        out.push(&body[line_end..end.max(line_end)]);
    }
    out
}

/// HTML as plain text: tags dropped, blocks on their own lines, and nothing loaded.
fn html_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let mut skip_until: Option<&str> = None;
    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            if skip_until.is_none() {
                out.push_str(rest);
            }
            break;
        };
        if skip_until.is_none() {
            out.push_str(&rest[..lt]);
        }
        rest = &rest[lt..];
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map_or("", |e| &rest[e + 3..]);
            continue;
        }
        let gt = rest.find('>').unwrap_or(rest.len() - 1);
        let tag = rest[1..gt].trim().to_ascii_lowercase();
        rest = &rest[(gt + 1).min(rest.len())..];
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if let Some(until) = skip_until {
            if tag.starts_with('/') && name == until {
                skip_until = None;
            }
            continue;
        }
        match name.as_str() {
            "style" | "script" | "head" | "title" if !tag.starts_with('/') => {
                skip_until = Some(match name.as_str() {
                    "style" => "style",
                    "script" => "script",
                    "head" => "head",
                    _ => "title",
                })
            }
            "br" => out.push('\n'),
            "li" if !tag.starts_with('/') => out.push_str("\n• "),
            "p" | "div" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "ul" | "ol" | "table" | "hr" => out.push_str("\n\n"),
            "td" | "th" if tag.starts_with('/') => out.push_str("  "),
            _ => {}
        }
    }
    let text = entities(&out);
    // Spaces squeezed within lines, and no more than one empty line in a row.
    let mut tidy = String::new();
    let mut blank = 0;
    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        tidy.push_str(&line);
        tidy.push('\n');
    }
    tidy.trim().to_string()
}

fn entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[..rest.len().min(12)].find(';');
        let Some(end) = end else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let ch = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "hellip" => Some('…'),
            "rsquo" => Some('’'),
            "lsquo" => Some('‘'),
            "rdquo" => Some('”'),
            "ldquo" => Some('“'),
            "copy" => Some('©'),
            "reg" => Some('®'),
            "trade" => Some('™'),
            "euro" => Some('€'),
            "pound" => Some('£'),
            "bull" => Some('•'),
            "middot" => Some('·'),
            "eacute" => Some('é'),
            "egrave" => Some('è'),
            "ecirc" => Some('ê'),
            "aacute" => Some('á'),
            "agrave" => Some('à'),
            "acirc" => Some('â'),
            "iacute" => Some('í'),
            "oacute" => Some('ó'),
            "uacute" => Some('ú'),
            "ccedil" => Some('ç'),
            "ntilde" => Some('ñ'),
            "auml" => Some('ä'),
            "ouml" => Some('ö'),
            "uuml" => Some('ü'),
            "szlig" => Some('ß'),
            "Eacute" => Some('É'),
            "Auml" => Some('Ä'),
            "Ouml" => Some('Ö'),
            "Uuml" => Some('Ü'),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A message's text as paragraphs, with quoted replies (lines starting with ">") set apart.
fn text_blocks(text: &str, out: &mut Vec<Block>) {
    let text = text.replace("\r\n", "\n");
    let mut para: Vec<&str> = Vec::new();
    let mut quote: Vec<String> = Vec::new();
    let flush_para = |para: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Para(plain(para.join("\n"))));
            para.clear();
        }
    };
    let flush_quote = |quote: &mut Vec<String>, out: &mut Vec<Block>| {
        if !quote.is_empty() {
            let inner = std::mem::take(quote).join("\n");
            let mut blocks = Vec::new();
            text_blocks(&inner, &mut blocks);
            out.push(Block::Quote(blocks));
        }
    };
    for line in text.lines() {
        if out.len() > 3000 {
            break;
        }
        let line = line.trim_end();
        if let Some(q) = line.strip_prefix('>') {
            flush_para(&mut para, out);
            quote.push(q.strip_prefix(' ').unwrap_or(q).to_string());
        } else if line.is_empty() {
            flush_para(&mut para, out);
            if !quote.is_empty() {
                quote.push(String::new());
            }
        } else {
            flush_quote(&mut quote, out);
            para.push(line);
        }
    }
    flush_para(&mut para, out);
    flush_quote(&mut quote, out);
}

/// A name and address, as people write them: `Ann Smith <ann@example.com>`.
fn person(v: &str) -> String {
    decode_words(v).replace('"', "")
}

fn message(raw: &[u8]) -> Mail {
    let (headers, body) = split_head(raw);
    let mut w = Walk::default();
    walk(&headers, body, 0, &mut w);
    let subject = header(&headers, "subject").map(decode_words).unwrap_or_else(|| "(No subject)".into());
    let mut blocks = vec![Block::Heading { level: 1, spans: plain(subject.clone()), id: String::new() }];
    for (label, key) in [("From", "from"), ("To", "to"), ("Cc", "cc"), ("Date", "date")] {
        if let Some(v) = header(&headers, key) {
            let v = if key == "date" { pretty_date(v) } else { person(v) };
            blocks.push(Block::Para(vec![bold(format!("{label}: ")), Span { text: v, ..Default::default() }]));
        }
    }
    blocks.push(Block::Rule);
    let (text, format) = match (&w.plain, &w.html) {
        (Some(p), _) if !p.trim().is_empty() => (p.clone(), "Plain text"),
        (_, Some(h)) => (html_text(h), "HTML, shown as plain text"),
        (Some(p), None) => (p.clone(), "Plain text"),
        (None, None) => (String::new(), "No text"),
    };
    text_blocks(&text, &mut blocks);
    let total: usize = w.attachments.iter().map(|a| a.1).sum();
    if !w.attachments.is_empty() {
        blocks.push(Block::Heading { level: 2, spans: plain("Attachments"), id: String::new() });
        let items = w.attachments.iter().map(|(n, s, _)| vec![Block::Para(plain(format!("{n} ({})", crate::scan::human(*s as u64))))]).collect();
        blocks.push(Block::List { start: None, items });
    }
    let mut facts = Vec::new();
    if let Some(v) = header(&headers, "from") {
        facts.push(("From", person(v)));
    }
    if let Some(v) = header(&headers, "to") {
        facts.push(("To", person(v)));
    }
    if let Some(v) = header(&headers, "cc") {
        facts.push(("Cc", person(v)));
    }
    if let Some(v) = header(&headers, "date") {
        facts.push(("Sent", pretty_date(v)));
    }
    facts.push(("Subject", subject));
    if !w.attachments.is_empty() {
        facts.push(("Attachments", format!("{} ({})", w.attachments.len(), crate::scan::human(total as u64))));
    }
    facts.push(("Text", format.into()));
    if let Some(v) = header(&headers, "x-mailer").or_else(|| header(&headers, "user-agent")) {
        facts.push(("Sent with", v.to_string()));
    }
    Mail { blocks, facts }
}

/// An email date (RFC 2822) written shortly; as written when it doesn't parse.
fn pretty_date(v: &str) -> String {
    // Some mailers add "(UTC)" or similar after the zone.
    let clean = v.split(" (").next().unwrap_or(v).trim();
    match chrono::DateTime::parse_from_rfc2822(clean) {
        Ok(d) => d.format("%a %-d %b %Y, %H:%M").to_string(),
        Err(_) => v.to_string(),
    }
}

fn mailbox(path: &Path) -> Result<Mail, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut r = std::io::BufReader::new(f).take(MBOX_SCAN);
    let mut rows: Vec<Vec<Vec<Span>>> = Vec::new();
    let mut count = 0usize;
    let mut first: Option<chrono::DateTime<chrono::FixedOffset>> = None;
    let mut last: Option<chrono::DateTime<chrono::FixedOffset>> = None;
    let mut line = Vec::new();
    let mut prev_blank = true;
    // In the headers of a message: its date, sender and subject so far.
    let mut head: Option<Headers> = None;
    let mut folded: Option<usize> = None;
    let finish = |h: Headers, rows: &mut Vec<Vec<Vec<Span>>>, first: &mut Option<_>, last: &mut Option<_>| {
        let date = header(&h, "date").map(|d| d.split(" (").next().unwrap_or(d).trim().to_string());
        let parsed = date.as_deref().and_then(|d| chrono::DateTime::parse_from_rfc2822(d).ok());
        if let Some(p) = parsed {
            if first.is_none_or(|f| p < f) {
                *first = Some(p);
            }
            if last.is_none_or(|l| p > l) {
                *last = Some(p);
            }
        }
        if rows.len() < ROWS_MAX {
            let when = parsed.map(|p| p.format("%-d %b %Y %H:%M").to_string()).or(date).unwrap_or_default();
            rows.push(vec![plain(when), plain(header(&h, "from").map(person).unwrap_or_default()), plain(header(&h, "subject").map(decode_words).unwrap_or_default())]);
        }
    };
    loop {
        line.clear();
        if r.read_until(b'\n', &mut line).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\n', '\r']);
        if prev_blank && text.starts_with("From ") {
            if let Some(h) = head.take() {
                finish(h, &mut rows, &mut first, &mut last);
            }
            count += 1;
            head = Some(Vec::new());
            folded = None;
        } else if let Some(h) = head.as_mut() {
            if text.is_empty() {
                let h = head.take().unwrap_or_default();
                finish(h, &mut rows, &mut first, &mut last);
            } else if text.starts_with([' ', '\t']) {
                if let Some(i) = folded {
                    h[i].1.push(' ');
                    h[i].1.push_str(text.trim());
                }
            } else if let Some((k, v)) = text.split_once(':') {
                let k = k.trim().to_ascii_lowercase();
                folded = matches!(k.as_str(), "date" | "from" | "subject").then_some(h.len());
                if folded.is_some() {
                    h.push((k, v.trim().to_string()));
                }
            }
        }
        prev_blank = text.is_empty();
    }
    if let Some(h) = head.take() {
        finish(h, &mut rows, &mut first, &mut last);
    }
    if count == 0 {
        return Err("No messages found: this doesn't look like an mbox mailbox".into());
    }
    let partial = r.limit() == 0;
    let n = if partial { format!("At least {}", group(count as u64)) } else { group(count as u64) };
    let mut facts = vec![("Messages", n)];
    if let (Some(a), Some(b)) = (first, last) {
        facts.push(("Earliest", a.format("%-d %b %Y").to_string()));
        facts.push(("Latest", b.format("%-d %b %Y").to_string()));
    }
    let mut blocks = vec![Block::Heading { level: 1, spans: plain(format!("{} {}", group(count as u64), if count == 1 { "message" } else { "messages" })), id: String::new() }];
    if count > rows.len() {
        blocks.push(Block::Para(plain(format!("The first {} are listed.", group(rows.len() as u64)))));
    }
    blocks.push(Block::Table { head: vec![plain("Date"), plain("From"), plain("Subject")], rows });
    Ok(Mail { blocks, facts })
}

// ---- vCard and iCalendar ----

/// One `NAME;PARAM=x:value` line, unfolded.
struct Prop {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Prop {
    fn param(&self, k: &str) -> Option<&str> {
        self.params.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
}

/// The lines of a vCard or iCalendar file, unfolded (a line starting with a space or tab
/// continues the one before).
fn props(text: &str) -> Vec<Prop> {
    let mut lines: Vec<String> = Vec::new();
    let mut qp_soft = false;
    for raw in text.lines() {
        let raw = raw.trim_end_matches('\r');
        if (raw.starts_with([' ', '\t']) || qp_soft) && !lines.is_empty() {
            let last = lines.last_mut().unwrap();
            if qp_soft {
                last.pop(); // the '='
                last.push_str(raw);
            } else {
                last.push_str(&raw[1..]);
            }
        } else {
            lines.push(raw.to_string());
        }
        // vCard 2.1 quoted-printable lines end in '=' to continue.
        qp_soft = lines.last().is_some_and(|l| l.to_ascii_uppercase().contains("QUOTED-PRINTABLE") && l.ends_with('='));
    }
    let mut out = Vec::new();
    for line in lines {
        // The value starts at the first ':' outside quotes.
        let mut quoted = false;
        let Some(colon) = line.char_indices().find(|&(_, c)| {
            if c == '"' {
                quoted = !quoted;
            }
            c == ':' && !quoted
        }) else {
            continue;
        };
        let (left, value) = (&line[..colon.0], &line[colon.0 + 1..]);
        let mut bits = left.split(';');
        let name = bits.next().unwrap_or("").to_ascii_uppercase();
        // "item1.EMAIL" (Apple's groups): the name is after the dot.
        let name = name.rsplit('.').next().unwrap_or(&name).to_string();
        let params: Vec<(String, String)> = bits
            .map(|p| match p.split_once('=') {
                Some((k, v)) => (k.to_ascii_uppercase(), v.trim_matches('"').to_string()),
                None => ("TYPE".into(), p.to_string()), // vCard 2.1: ;WORK;VOICE
            })
            .collect();
        let mut prop = Prop { name, params, value: value.to_string() };
        if prop.param("ENCODING").is_some_and(|e| e.eq_ignore_ascii_case("QUOTED-PRINTABLE")) {
            prop.value = decode_charset(&quoted_printable(prop.value.as_bytes()), prop.param("CHARSET").unwrap_or("utf-8"));
        }
        out.push(prop);
    }
    out
}

/// `\n`, `\,`, `\;` and `\\` in a value.
fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(c) => out.push(c),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A structured value's parts (`;`-separated, escapes allowed), the empty ones left out.
fn parts(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut esc = false;
    for c in v.chars() {
        if esc {
            cur.push('\\');
            cur.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == ';' {
            out.push(unescape(&std::mem::take(&mut cur)));
        } else {
            cur.push(c);
        }
    }
    out.push(unescape(&cur));
    out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

#[derive(Default)]
struct Card {
    name: String,
    org: String,
    title: String,
    emails: Vec<(String, String)>,
    phones: Vec<(String, String)>,
    addresses: Vec<(String, String)>,
    urls: Vec<String>,
    birthday: String,
    note: String,
    photo: bool,
}

/// A TYPE parameter for people: "work", "home, mobile".
fn type_of(p: &Prop) -> String {
    let t: Vec<String> = p
        .params
        .iter()
        .filter(|(k, _)| k == "TYPE")
        .flat_map(|(_, v)| v.split(','))
        .map(|t| t.trim().to_ascii_lowercase())
        .filter(|t| !matches!(t.as_str(), "internet" | "pref" | "voice" | "x400"))
        .map(|t| if t == "cell" { "mobile".into() } else { t })
        .collect();
    t.join(", ")
}

fn contacts(text: &str) -> Mail {
    let mut cards: Vec<Card> = Vec::new();
    let mut cur: Option<Card> = None;
    for p in props(text) {
        match (p.name.as_str(), cur.as_mut()) {
            ("BEGIN", _) if p.value.eq_ignore_ascii_case("VCARD") => cur = Some(Card::default()),
            ("END", Some(_)) if p.value.eq_ignore_ascii_case("VCARD") => cards.extend(cur.take()),
            ("FN", Some(c)) => c.name = unescape(&p.value),
            ("N", Some(c)) if c.name.is_empty() => {
                // Family; given; middle; prefix; suffix.
                let n: Vec<String> = p.value.split(';').map(unescape).collect();
                let order = [3, 1, 2, 0, 4];
                c.name = order.iter().filter_map(|&i| n.get(i)).map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
            }
            ("ORG", Some(c)) => c.org = parts(&p.value).join(", "),
            ("TITLE", Some(c)) => c.title = unescape(&p.value),
            ("EMAIL", Some(c)) => c.emails.push((type_of(&p), unescape(&p.value))),
            ("TEL", Some(c)) => c.phones.push((type_of(&p), unescape(p.value.trim_start_matches("tel:")))),
            ("ADR", Some(c)) => c.addresses.push((type_of(&p), parts(&p.value).join(", "))),
            ("URL", Some(c)) => c.urls.push(unescape(&p.value)),
            ("BDAY", Some(c)) => c.birthday = pretty_ical(&p.value, None).unwrap_or_else(|| p.value.clone()),
            ("NOTE", Some(c)) => c.note = unescape(&p.value),
            ("PHOTO", Some(c)) => c.photo = true,
            _ => {}
        }
    }
    let mut facts = Vec::new();
    let mut blocks = Vec::new();
    match &cards[..] {
        [] => blocks.push(Block::Para(plain("No contact cards in it."))),
        [c] => {
            blocks.push(Block::Heading { level: 1, spans: plain(if c.name.is_empty() { "(No name)" } else { &c.name }), id: String::new() });
            let job = [c.title.as_str(), c.org.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ");
            if !job.is_empty() {
                blocks.push(Block::Para(plain(job)));
            }
            let row = |label: &str, kind: &str, v: &str| {
                let label = if kind.is_empty() { format!("{label}: ") } else { format!("{label} ({kind}): ") };
                Block::Para(vec![bold(label), Span { text: v.to_string(), ..Default::default() }])
            };
            for (t, v) in &c.emails {
                blocks.push(row("Email", t, v));
            }
            for (t, v) in &c.phones {
                blocks.push(row("Phone", t, v));
            }
            for (t, v) in &c.addresses {
                blocks.push(row("Address", t, v));
            }
            for v in &c.urls {
                blocks.push(row("Web", "", v));
            }
            if !c.birthday.is_empty() {
                blocks.push(row("Birthday", "", &c.birthday));
            }
            if !c.note.is_empty() {
                blocks.push(Block::Rule);
                text_blocks(&c.note, &mut blocks);
            }
            facts.push(("Name", c.name.clone()));
            if !c.org.is_empty() {
                facts.push(("Organisation", c.org.clone()));
            }
            if let Some((_, e)) = c.emails.first() {
                facts.push(("Email", e.clone()));
            }
            if let Some((_, t)) = c.phones.first() {
                facts.push(("Phone", t.clone()));
            }
            if c.photo {
                facts.push(("Photo", "Yes".into()));
            }
        }
        many => {
            blocks.push(Block::Heading { level: 1, spans: plain(format!("{} contacts", group(many.len() as u64))), id: String::new() });
            let rows = many
                .iter()
                .take(ROWS_MAX)
                .map(|c| {
                    let first = |v: &[(String, String)]| v.first().map(|x| x.1.clone()).unwrap_or_default();
                    vec![plain(c.name.clone()), plain(first(&c.emails)), plain(first(&c.phones)), plain(c.org.clone())]
                })
                .collect();
            blocks.push(Block::Table { head: vec![plain("Name"), plain("Email"), plain("Phone"), plain("Organisation")], rows });
            facts.push(("Contacts", group(many.len() as u64)));
            facts.push(("With an email", group(many.iter().filter(|c| !c.emails.is_empty()).count() as u64)));
            facts.push(("With a phone", group(many.iter().filter(|c| !c.phones.is_empty()).count() as u64)));
        }
    }
    Mail { blocks, facts }
}

/// An iCalendar date or date-time (`20261126`, `20261126T090000Z`) written for people, with the
/// place its time zone is named after.
fn pretty_ical(v: &str, tzid: Option<&str>) -> Option<String> {
    let v = v.trim();
    let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
    let date = chrono::NaiveDate::parse_from_str(digits.get(..8)?, "%Y%m%d").ok()?;
    if digits.len() < 12 {
        return Some(date.format("%a %-d %b %Y").to_string());
    }
    let time = chrono::NaiveTime::parse_from_str(&digits[8..12], "%H%M").ok()?;
    let zone = if v.ends_with('Z') {
        " UTC".to_string()
    } else {
        tzid.map(|t| format!(" ({})", t.rsplit('/').next().unwrap_or(t).replace('_', " "))).unwrap_or_default()
    };
    Some(format!("{}, {}{zone}", date.format("%a %-d %b %Y"), time.format("%H:%M")))
}

#[derive(Default, Clone)]
struct Event {
    summary: String,
    /// Digits of the start, for sorting.
    key: String,
    start: String,
    end: String,
    location: String,
    description: String,
    repeats: String,
    organiser: String,
    attendees: usize,
}

fn repeats(rrule: &str) -> String {
    let freq = rrule.split(';').find_map(|p| p.strip_prefix("FREQ=")).unwrap_or("");
    match freq {
        "DAILY" => "Daily",
        "WEEKLY" => "Weekly",
        "MONTHLY" => "Monthly",
        "YEARLY" => "Yearly",
        "" => "",
        _ => "Yes",
    }
    .into()
}

fn calendar(text: &str) -> Mail {
    let mut events: Vec<Event> = Vec::new();
    let mut todos: Vec<String> = Vec::new();
    let mut name = String::new();
    // The components we're inside, innermost last.
    let mut stack: Vec<String> = Vec::new();
    let mut cur = Event::default();
    for p in props(text) {
        let inside = stack.last().map(String::as_str);
        match p.name.as_str() {
            "BEGIN" => {
                let comp = p.value.to_ascii_uppercase();
                if comp == "VEVENT" || comp == "VTODO" {
                    cur = Event::default();
                }
                stack.push(comp);
            }
            "END" => {
                match stack.pop().as_deref() {
                    Some("VEVENT") => events.push(std::mem::take(&mut cur)),
                    Some("VTODO") => todos.push(std::mem::take(&mut cur).summary),
                    _ => {}
                }
            }
            "X-WR-CALNAME" if inside == Some("VCALENDAR") => name = unescape(&p.value),
            _ if !matches!(inside, Some("VEVENT" | "VTODO")) => {}
            "SUMMARY" => cur.summary = unescape(&p.value),
            "DTSTART" => {
                cur.key = p.value.chars().filter(|c| c.is_ascii_digit()).collect();
                cur.start = pretty_ical(&p.value, p.param("TZID")).unwrap_or_else(|| p.value.clone());
            }
            "DTEND" | "DUE" => cur.end = pretty_ical(&p.value, p.param("TZID")).unwrap_or_else(|| p.value.clone()),
            "LOCATION" => cur.location = unescape(&p.value),
            "DESCRIPTION" => cur.description = unescape(&p.value),
            "RRULE" => cur.repeats = repeats(&p.value),
            "ORGANIZER" => cur.organiser = p.param("CN").map(str::to_string).unwrap_or_else(|| p.value.trim_start_matches("mailto:").trim_start_matches("MAILTO:").to_string()),
            "ATTENDEE" => cur.attendees += 1,
            _ => {}
        }
    }
    events.sort_by(|a, b| a.key.cmp(&b.key));
    let mut facts = Vec::new();
    let mut blocks = Vec::new();
    if !name.is_empty() {
        facts.push(("Calendar", name.clone()));
    }
    match &events[..] {
        [e] => {
            blocks.push(Block::Heading { level: 1, spans: plain(if e.summary.is_empty() { "(No title)" } else { &e.summary }), id: String::new() });
            let mut row = |label: &'static str, v: &str| {
                if !v.is_empty() {
                    blocks.push(Block::Para(vec![bold(format!("{label}: ")), Span { text: v.to_string(), ..Default::default() }]));
                    facts.push((label, v.to_string()));
                }
            };
            row("When", &e.start);
            row("Ends", &e.end);
            row("Place", &e.location);
            row("Repeats", &e.repeats);
            row("Organiser", &e.organiser);
            if e.attendees > 0 {
                row("Invited", &group(e.attendees as u64));
            }
            if !e.description.is_empty() {
                blocks.push(Block::Rule);
                text_blocks(&e.description, &mut blocks);
            }
        }
        many => {
            if !many.is_empty() {
                blocks.push(Block::Heading { level: 1, spans: plain(format!("{} events", group(many.len() as u64))), id: String::new() });
                let rows = many.iter().take(ROWS_MAX).map(|e| vec![plain(e.start.clone()), plain(e.summary.clone()), plain(e.location.clone())]).collect();
                blocks.push(Block::Table { head: vec![plain("When"), plain("What"), plain("Where")], rows });
                facts.push(("Events", group(many.len() as u64)));
                let dated: Vec<&Event> = many.iter().filter(|e| !e.key.is_empty()).collect();
                if let (Some(a), Some(b)) = (dated.first(), dated.last()) {
                    facts.push(("First", pretty_ical(&a.key[..8.min(a.key.len())], None).unwrap_or_default()));
                    facts.push(("Last", pretty_ical(&b.key[..8.min(b.key.len())], None).unwrap_or_default()));
                }
                let recurring = many.iter().filter(|e| !e.repeats.is_empty()).count();
                if recurring > 0 {
                    facts.push(("Repeating", group(recurring as u64)));
                }
            }
        }
    }
    if !todos.is_empty() {
        blocks.push(Block::Heading { level: 2, spans: plain("To-dos"), id: String::new() });
        let items = todos.iter().take(ROWS_MAX).map(|t| vec![Block::Para(plain(if t.is_empty() { "(No title)" } else { t }))]).collect();
        blocks.push(Block::List { start: None, items });
        facts.push(("To-dos", group(todos.len() as u64)));
    }
    if blocks.is_empty() {
        blocks.push(Block::Para(plain("No events or to-dos in it.")));
    }
    Mail { blocks, facts }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact<'a>(m: &'a Mail, k: &str) -> Option<&'a str> {
        m.facts.iter().find(|f| f.0 == k).map(|f| f.1.as_str())
    }

    fn all_text(m: &Mail) -> String {
        crate::officedoc::plain_text(&m.blocks)
    }

    #[test]
    fn a_multipart_message_with_an_attachment() {
        let raw = b"From: =?UTF-8?Q?Ren=C3=A9e?= <renee@example.com>\r\n\
To: Bob <bob@example.com>\r\n\
Subject: =?utf-8?B?SGVsbG8g4pyT?=\r\n =?utf-8?Q?_again?=\r\n\
Date: Thu, 26 Nov 2026 09:15:00 +0000\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"XYZ\"\r\n\
\r\n\
preamble\r\n\
--XYZ\r\n\
Content-Type: multipart/alternative; boundary=ALT\r\n\
\r\n\
--ALT\r\n\
Content-Type: text/plain; charset=iso-8859-1\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
Caf=E9 at ten?\r\n\
\r\n\
> earlier=\r\n\
 text\r\n\
--ALT\r\n\
Content-Type: text/html\r\n\
\r\n\
<p>ignored</p>\r\n\
--ALT--\r\n\
--XYZ\r\n\
Content-Type: application/pdf; name=\"plan.pdf\"\r\n\
Content-Disposition: attachment; filename*=UTF-8''pl%C3%A4n.pdf\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
SGVsbG8s\r\nIHdvcmxk\r\n\
--XYZ--\r\n";
        let m = message(raw);
        assert_eq!(fact(&m, "Subject"), Some("Hello ✓ again"));
        assert_eq!(fact(&m, "From"), Some("Renée <renee@example.com>"));
        assert_eq!(fact(&m, "Sent"), Some("Thu 26 Nov 2026, 09:15"));
        assert_eq!(fact(&m, "Attachments"), Some("1 (12 B)"));
        assert_eq!(fact(&m, "Text"), Some("Plain text"));
        let text = all_text(&m);
        assert!(text.contains("Café at ten?"), "{text}");
        assert!(text.contains("plän.pdf"), "{text}");
        assert!(m.blocks.iter().any(|b| matches!(b, Block::Quote(_))));
    }

    #[test]
    fn html_only_mail_becomes_text_and_loads_nothing() {
        let raw = b"Subject: News\nContent-Type: text/html; charset=utf-8\n\n<html><head><style>p{}</style></head><body><p>Hi&nbsp;there &amp; welcome</p><img src=\"http://tracker.example/x.gif\"><ul><li>One</li><li>Two</li></ul></body></html>";
        let m = message(raw);
        let text = all_text(&m);
        assert!(text.contains("Hi there & welcome"), "{text}");
        assert!(text.contains("• One"), "{text}");
        assert!(!text.contains("tracker") && !text.contains("p{}"), "{text}");
        assert_eq!(fact(&m, "Text"), Some("HTML, shown as plain text"));
    }

    #[test]
    fn emlx_length_line_is_skipped() {
        let raw = b"28\nSubject: Apple\n\nBody here\n<?xml version=\"1.0\"?><plist/>";
        let m = message(emlx_body(raw));
        assert_eq!(fact(&m, "Subject"), Some("Apple"));
        assert!(!all_text(&m).contains("plist"));
    }

    #[test]
    fn mailbox_lists_messages() {
        let dir = std::env::temp_dir().join(format!("filescape-mbox-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("Inbox.mbox");
        std::fs::write(
            &p,
            "From a@x Thu Nov 26 09:00:00 2026\nFrom: Ann <a@x>\nSubject: First\nDate: Wed, 25 Nov 2026 09:00:00 +0000\n\nFrom the body, not a new message? It follows text.\n\nFrom b@x Thu Nov 26 10:00:00 2026\nFrom: Bob <b@x>\nSubject: =?utf-8?Q?Second_=E2=9C=93?=\nDate: Thu, 26 Nov 2026 10:00:00 +0000\n\nHi\n",
        )
        .unwrap();
        let m = read(&p).unwrap();
        // "From the body" follows a blank line, so mbox readers count it as a message too; it
        // has no headers.
        assert_eq!(fact(&m, "Messages"), Some("3"));
        assert_eq!(fact(&m, "Earliest"), Some("25 Nov 2026"));
        assert_eq!(fact(&m, "Latest"), Some("26 Nov 2026"));
        assert!(all_text(&m).contains("Second ✓"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_contact_and_many() {
        let one = contacts("BEGIN:VCARD\r\nVERSION:3.0\r\nN:Smith;Ann;;Dr;\r\nFN:Dr Ann Smith\r\nORG:Acme\\, Inc;Labs\r\nTITLE:Chemist\r\nitem1.EMAIL;TYPE=INTERNET,WORK:ann@acme.example\r\nTEL;TYPE=CELL:+44 7700 900123\r\nADR;TYPE=HOME:;;1 High St;Oxford;;OX1 1AA;UK\r\nNOTE:Met at the\r\n  conference\\nBring samples\r\nEND:VCARD\r\n");
        assert_eq!(fact(&one, "Name"), Some("Dr Ann Smith"));
        assert_eq!(fact(&one, "Organisation"), Some("Acme, Inc, Labs"));
        let text = all_text(&one);
        assert!(text.contains("Phone (mobile): +44 7700 900123"), "{text}");
        assert!(text.contains("Address (home): 1 High St, Oxford, OX1 1AA, UK"), "{text}");
        assert!(text.contains("Met at the conference\nBring samples"), "{text}");
        let many = contacts("BEGIN:VCARD\nVERSION:2.1\nN:Jones;Bob\nTEL;WORK;VOICE:123\nEND:VCARD\nBEGIN:VCARD\nFN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Ren=C3=A9e\nEMAIL:r@x\nEND:VCARD\n");
        assert_eq!(fact(&many, "Contacts"), Some("2"));
        let text = all_text(&many);
        assert!(text.contains("Bob Jones") && text.contains("Renée"), "{text}");
    }

    #[test]
    fn calendar_events() {
        let one = calendar("BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/London\r\nBEGIN:STANDARD\r\nDTSTART:19701025T020000\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nSUMMARY:Planning\\, Q1\r\nDTSTART;TZID=Europe/London:20261201T093000\r\nDTEND;TZID=Europe/London:20261201T103000\r\nLOCATION:Room 4\r\nRRULE:FREQ=WEEKLY;BYDAY=TU\r\nORGANIZER;CN=Ann:mailto:ann@x\r\nATTENDEE:mailto:bob@x\r\nATTENDEE:mailto:cy@x\r\nDESCRIPTION:Agenda:\\n1. Budget\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n");
        assert_eq!(fact(&one, "When"), Some("Tue 1 Dec 2026, 09:30 (London)"));
        assert_eq!(fact(&one, "Repeats"), Some("Weekly"));
        assert_eq!(fact(&one, "Organiser"), Some("Ann"));
        assert_eq!(fact(&one, "Invited"), Some("2"));
        assert!(all_text(&one).contains("Planning, Q1"));
        let many = calendar("BEGIN:VCALENDAR\nX-WR-CALNAME:Home\nBEGIN:VEVENT\nSUMMARY:Later\nDTSTART;VALUE=DATE:20270105\nEND:VEVENT\nBEGIN:VEVENT\nSUMMARY:Sooner\nDTSTART:20261224T180000Z\nEND:VEVENT\nBEGIN:VTODO\nSUMMARY:Buy tree\nEND:VTODO\nEND:VCALENDAR\n");
        assert_eq!(fact(&many, "Calendar"), Some("Home"));
        assert_eq!(fact(&many, "Events"), Some("2"));
        assert_eq!(fact(&many, "First"), Some("Thu 24 Dec 2026"));
        assert_eq!(fact(&many, "To-dos"), Some("1"));
        let text = all_text(&many);
        assert!(text.find("Sooner").unwrap() < text.find("Later").unwrap(), "{text}");
        assert!(text.contains("Thu 24 Dec 2026, 18:00 UTC"), "{text}");
    }

    #[test]
    fn broken_input_does_not_panic() {
        for raw in [&b""[..], b"\n\n", b"Content-Type: multipart/mixed; boundary=\n\n--\n--", b"Subject: =?x?Q?", b"=?utf-8?B?!!!?="] {
            let _ = message(raw);
        }
        let _ = contacts("BEGIN:VCARD\nN:;\nEND:VCARD");
        let _ = calendar("END:VEVENT\nBEGIN:VEVENT\nDTSTART:2\n");
        assert_eq!(decode_words("plain =?nope"), "plain =?nope");
    }
}
