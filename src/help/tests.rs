//! Checks that keep the manual honest: every page is listed and reachable, every link and image
//! resolves, every setting and shortcut is documented, the generated reference tables are
//! current, and every filter example in the docs really behaves as the docs say.
//!
//! `DIRMAP_UPDATE_DOCS=1 cargo test help` rewrites the generated tables instead of failing.

use super::doc::{self, Block, Docs, Span};
use super::reference::{self, Group, KEYBOARD, MOUSE};
use super::{render, topics};
use crate::categories::{self, Category};
use crate::pattern::{Filter, Options};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn docs() -> Docs {
    Docs::load()
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("reading {rel}: {e}")).replace("\r\n", "\n")
}

/// Every table, code block and paragraph on every page, with the page it's on.
fn all_blocks(d: &Docs) -> Vec<(&str, &Block)> {
    fn walk<'a>(path: &'a str, blocks: &'a [Block], out: &mut Vec<(&'a str, &'a Block)>) {
        for b in blocks {
            out.push((path, b));
            match b {
                Block::List { items, .. } => items.iter().for_each(|i| walk(path, i, out)),
                Block::Quote(inner) => walk(path, inner, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for p in &d.pages {
        walk(&p.path, &p.blocks, &mut out);
    }
    out
}

fn code_spans(cell: &[Span]) -> Vec<String> {
    cell.iter().filter(|s| s.code).map(|s| s.text.clone()).collect()
}

fn cell_text(cell: &[Span]) -> String {
    doc::plain(cell).trim().to_string()
}

#[test]
fn every_page_parses_and_has_a_title() {
    let d = docs();
    assert!(d.pages.len() > 40, "only {} pages bundled", d.pages.len());
    for p in &d.pages {
        assert!(!p.blocks.is_empty(), "{} is empty", p.path);
        assert!(
            matches!(p.blocks.first(), Some(Block::Heading { level: 1, .. })),
            "{} must start with a `# Title`",
            p.path
        );
    }
}

#[test]
fn contents_lists_every_page_exactly_once() {
    let d = docs();
    let listed: Vec<&String> = d.order.iter().collect();
    let unique: HashSet<&String> = listed.iter().copied().collect();
    assert_eq!(unique.len(), listed.len(), "a page is listed twice in SUMMARY.md");
    for p in &listed {
        assert!(d.page(p).is_some(), "SUMMARY.md lists {p}, which doesn't exist");
    }
    for p in &d.pages {
        assert!(unique.contains(&p.path), "{} isn't listed in SUMMARY.md", p.path);
    }
}

#[test]
fn every_link_and_anchor_resolves() {
    let d = docs();
    let mut bad = Vec::new();
    for p in &d.pages {
        for (link, is_image) in doc::links(&p.blocks) {
            if is_image {
                continue;
            }
            if link.starts_with("http://") || link.starts_with("https://") || link.starts_with("mailto:") {
                continue;
            }
            match doc::resolve(&p.path, &link) {
                None => bad.push(format!("{}: can't resolve {link}", p.path)),
                Some((page, anchor)) => match d.page(&page) {
                    None => bad.push(format!("{}: {link} -> missing page {page}", p.path)),
                    Some(target) => {
                        if let Some(a) = anchor
                            && !target.headings.iter().any(|h| h.2 == a)
                        {
                            bad.push(format!("{}: {link} -> no heading #{a} on {page}", p.path));
                        }
                    }
                },
            }
        }
    }
    assert!(bad.is_empty(), "broken links:\n{}", bad.join("\n"));
}

#[test]
fn heading_ids_are_unique_on_each_page() {
    for p in &docs().pages {
        let mut seen = HashSet::new();
        for h in &p.headings {
            assert!(seen.insert(&h.2), "{} has two headings with the id #{}", p.path, h.2);
        }
    }
}

#[test]
fn every_image_exists_is_a_png_and_is_used() {
    let d = docs();
    let mut used = BTreeSet::new();
    for p in &d.pages {
        for (link, is_image) in doc::links(&p.blocks) {
            if !is_image {
                continue;
            }
            let path = doc::join(&p.path, &link);
            let bytes = d.image(&path).unwrap_or_else(|| panic!("{}: image {link} ({path}) isn't in docs/src", p.path));
            let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
                .unwrap_or_else(|e| panic!("{path} isn't a readable PNG: {e}"));
            assert!(img.width() >= 200 && img.height() >= 50, "{path} is suspiciously small");
            used.insert(path);
        }
    }
    for path in Docs::image_paths() {
        assert!(used.contains(path), "{path} isn't used by any page");
    }
}

#[test]
fn every_help_topic_used_by_the_app_exists() {
    let d = docs();
    for t in topics::ALL {
        let (page, anchor) = match t.split_once('#') {
            Some((p, a)) => (p, Some(a)),
            None => (*t, None),
        };
        let p = d.page(page).unwrap_or_else(|| panic!("topic {t}: no page {page}"));
        if let Some(a) = anchor {
            assert!(p.headings.iter().any(|h| h.2 == a), "topic {t}: no heading #{a}");
        }
    }
}

#[test]
fn every_topic_constant_is_in_the_list() {
    // A constant added to topics.rs but not to ALL would escape the check above.
    let src = read("src/help/topics.rs");
    let count = src.lines().filter(|l| l.starts_with("pub const ") && l.contains(": &str")).count();
    assert_eq!(count, topics::ALL.len(), "add every topic constant to topics::ALL");
}

/// Field names of a `pub struct` in a source file.
fn struct_fields(file: &str, header: &str) -> Vec<String> {
    let src = read(file);
    let start = src.find(header).unwrap_or_else(|| panic!("{header} not found in {file}"));
    let body = &src[start..];
    let body = &body[body.find('{').unwrap() + 1..body.find("\n}").unwrap()];
    body.lines()
        .filter_map(|l| {
            let l = l.trim();
            let l = l.strip_prefix("pub ")?;
            let (name, _) = l.split_once(':')?;
            Some(name.trim().to_string())
        })
        .collect()
}

fn markers(page: &str, kind: &str) -> BTreeSet<String> {
    let src = read(&format!("docs/src/{page}"));
    let tag = format!("<!-- {kind}:");
    src.match_indices(&tag)
        .map(|(i, _)| {
            let rest = &src[i + tag.len()..];
            rest[..rest.find("-->").unwrap()].trim().to_string()
        })
        .collect()
}

#[test]
fn every_setting_is_documented() {
    let fields: BTreeSet<String> = struct_fields("src/settings.rs", "pub struct Settings").into_iter().collect();
    assert!(fields.len() > 25, "couldn't read the Settings fields");
    let documented = markers(topics::SETTINGS, "setting");
    let missing: Vec<_> = fields.difference(&documented).collect();
    let stale: Vec<_> = documented.difference(&fields).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "settings/reference.md is out of date.\nNot documented (add a row with <!-- setting:NAME -->): {missing:?}\nNo longer settings: {stale:?}"
    );
}

#[test]
fn every_custom_filter_preference_is_documented() {
    let fields: BTreeSet<String> =
        struct_fields("src/app/custom.rs", "pub(super) struct CustomPrefs").into_iter().collect();
    assert!(fields.len() >= 4);
    let documented = markers(topics::SETTINGS, "pref");
    assert_eq!(fields, documented, "settings/reference.md: <!-- pref:NAME --> markers must match CustomPrefs");
}

#[test]
fn reference_bindings_match_the_code() {
    let mut bad = Vec::new();
    for g in KEYBOARD.iter().chain(MOUSE) {
        for b in g.bindings {
            assert!(!b.source.is_empty(), "{} has no source", b.keys);
            for (file, needle) in b.source {
                if !read(file).contains(needle) {
                    bad.push(format!("{} ({}): `{needle}` not found in {file}", b.keys, g.title));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "the keyboard/mouse reference describes code that has changed; update src/help/reference.rs:\n{}",
        bad.join("\n")
    );
}

fn check_generated(page: &str, groups: &[Group]) {
    let path = root().join("docs/src").join(page);
    // Git on Windows may check the page out with CRLF line endings.
    let src = std::fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    let start = src.find(reference::BEGIN).unwrap_or_else(|| panic!("{page}: no BEGIN GENERATED marker"));
    let end = src.find(reference::END).unwrap_or_else(|| panic!("{page}: no END GENERATED marker"));
    let want = format!("{}\n\n{}\n{}", reference::BEGIN, reference::markdown(groups), reference::END);
    let have = &src[start..end + reference::END.len()];
    if have == want {
        return;
    }
    if std::env::var_os("DIRMAP_UPDATE_DOCS").is_some() {
        let new = format!("{}{}{}", &src[..start], want, &src[end + reference::END.len()..]);
        std::fs::write(&path, new).unwrap();
        eprintln!("updated {page}");
    } else {
        panic!("{page} is out of date; run `DIRMAP_UPDATE_DOCS=1 cargo test help` to regenerate it");
    }
}

#[test]
fn keyboard_reference_is_current() {
    check_generated(topics::KEYBOARD, KEYBOARD);
}

#[test]
fn mouse_reference_is_current() {
    check_generated(topics::MOUSE, MOUSE);
}

#[test]
fn filter_syntax_examples_behave_as_documented() {
    let d = docs();
    let mut checked = 0;
    let mut bad = Vec::new();
    for (path, b) in all_blocks(&d) {
        let Block::Table { head, rows } = b else { continue };
        let heads: Vec<String> = head.iter().map(|c| cell_text(c)).collect();
        let first = heads.first().map(String::as_str).unwrap_or("");
        if !first.starts_with("Pattern") {
            continue;
        }
        let opts = Options {
            match_all: first.contains("match all"),
            case_sensitive: first.contains("case-sensitive"),
        };
        for row in rows {
            let pats = code_spans(&row[0]);
            assert_eq!(pats.len(), 1, "{path}: pattern cell must be one `code` span: {:?}", cell_text(&row[0]));
            let pat = &pats[0];
            let compiled = Filter::compile(pat, opts);
            match heads.get(1).map(String::as_str) {
                Some("Problem") => {
                    if compiled.is_ok() {
                        bad.push(format!("{path}: `{pat}` is listed as invalid but compiles"));
                    }
                }
                Some("Matches") => {
                    let f = match compiled {
                        Ok(f) => f,
                        Err(e) => {
                            bad.push(format!("{path}: `{pat}` doesn't compile: {e}"));
                            continue;
                        }
                    };
                    for name in code_spans(&row[1]) {
                        if !f.matches(&name) {
                            bad.push(format!("{path}: `{pat}` should match `{name}` but doesn't"));
                        }
                    }
                    for name in row.get(2).map(|c| code_spans(c)).unwrap_or_default() {
                        if f.matches(&name) {
                            bad.push(format!("{path}: `{pat}` shouldn't match `{name}` but does"));
                        }
                    }
                }
                _ => {
                    if let Err(e) = compiled {
                        bad.push(format!("{path}: `{pat}` doesn't compile: {e}"));
                    }
                }
            }
            checked += 1;
        }
    }
    assert!(bad.is_empty(), "filter examples are wrong:\n{}", bad.join("\n"));
    assert!(checked > 40, "only {checked} pattern examples found");
}

#[test]
fn filter_code_blocks_compile() {
    let d = docs();
    let mut n = 0;
    for (path, b) in all_blocks(&d) {
        let Block::Code { lang, text } = b else { continue };
        if lang != "filter" {
            continue;
        }
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if let Err(e) = Filter::compile(line, Options::default()) {
                panic!("{path}: example `{line}` doesn't compile: {e}");
            }
            n += 1;
        }
    }
    assert!(n >= 15, "only {n} filter examples found");
}

#[test]
fn categories_table_matches_the_code() {
    let d = docs();
    let page = d.page(topics::CATEGORIES).unwrap();
    let Some(Block::Table { rows, .. }) = page.blocks.iter().find(|b| matches!(b, Block::Table { head, .. } if cell_text(&head[0]) == "Category"))
    else {
        panic!("no Category table on {}", topics::CATEGORIES)
    };
    let mut documented = HashSet::new();
    for row in rows {
        let name = cell_text(&row[0]);
        let cat = Category::ALL
            .into_iter()
            .find(|c| {
                let label = c.label();
                let label = label.split_once(' ').map_or(label, |(_, l)| l);
                name.ends_with(label) || label.eq_ignore_ascii_case(name.trim_start_matches(|c: char| !c.is_ascii_alphabetic()))
            })
            .unwrap_or_else(|| panic!("categories table: no category called {name:?}"));
        if cat == Category::Other {
            continue;
        }
        // Spans before ", and" are plain extensions; the rest describe special names.
        for s in &row[1] {
            if !s.code {
                if s.text.contains(", and") {
                    break;
                }
                continue;
            }
            let ext = &s.text;
            assert_eq!(categories::of(&format!("x.{ext}")), cat, "`.{ext}` is listed under {name}");
            documented.insert(ext.clone());
        }
    }
    // And the other way round: every extension the code knows is in the table.
    let src = read("src/categories.rs");
    let body = &src[src.find("match ext {").unwrap()..];
    let body = &body[..body.find("_ =>").unwrap()];
    let mut missing = Vec::new();
    for lit in body.split('"').skip(1).step_by(2) {
        if !documented.contains(lit) {
            missing.push(lit);
        }
    }
    assert!(missing.is_empty(), "extensions missing from {}: {missing:?}", topics::CATEGORIES);
}

#[test]
fn search_finds_the_obvious_pages() {
    let d = docs();
    let first = |q: &str| d.search(q).first().map(|h| h.path.clone()).unwrap_or_default();
    assert_eq!(first("duplicates"), topics::DUPLICATES);
    assert_eq!(first("sunburst"), topics::SUNBURST);
    assert_eq!(first("keyboard shortcuts"), topics::KEYBOARD);
    assert!(d.search("frame padding").iter().any(|h| h.path == topics::SYNTAX));
    assert!(d.search("zzqqxx-not-a-word").is_empty());
    assert!(d.search("   ").is_empty());
}

#[test]
fn neighbours_follow_the_contents() {
    let d = docs();
    let (prev, next) = d.neighbours(&d.order[1]);
    assert_eq!(prev, Some(d.order[0].as_str()));
    assert_eq!(next, Some(d.order[2].as_str()));
    assert_eq!(d.neighbours(&d.order[0]).0, None);
}

#[test]
fn slugs_follow_mdbook() {
    assert_eq!(doc::slug("Match all terms (AND)"), "match-all-terms-and");
    assert_eq!(doc::slug("Units: binary and decimal"), "units-binary-and-decimal");
    assert_eq!(doc::slug("What's New"), "whats-new");
}

#[test]
fn links_resolve_relative_to_the_page() {
    assert_eq!(doc::resolve("views/treemap.md", "../filtering/syntax.md#x"), Some(("filtering/syntax.md".into(), Some("x".into()))));
    assert_eq!(doc::resolve("views/treemap.md", "#y"), Some(("views/treemap.md".into(), Some("y".into()))));
    assert_eq!(doc::resolve("index.md", "https://example.com"), None);
}

#[test]
fn table_columns_fit() {
    assert_eq!(render::fit(&[100.0, 50.0], 400.0), vec![100.0, 50.0]);
    let w = render::fit(&[600.0, 50.0, 400.0], 500.0);
    assert!((w.iter().sum::<f32>() - 500.0).abs() < 1.0, "{w:?}");
    assert_eq!(w[1], 50.0, "narrow columns keep their width");
    assert!(w.iter().all(|&c| c >= 40.0));
}

#[test]
fn mdbook_config_points_at_the_pages() {
    let toml = read("docs/book.toml");
    assert!(toml.contains("src = \"src\""));
}
