//! Which parts of a document to give the model when all of it doesn't fit in its context: the
//! start, then the passages most related to the question ("Can I use this commercially?" finds
//! the clauses about commercial use, sale and warranties), or, for a question that names nothing
//! in particular ("Summarise this"), passages spread through the whole document. The model is
//! told what was left out, and so is the user.

/// Lines chosen from a document.
#[derive(Debug, PartialEq)]
pub struct Picked {
    /// The chosen lines, with `[…]` where lines were left out, numbered with their real line
    /// numbers when asked.
    pub text: String,
    /// The lines included, from 1, first and last of each run.
    pub ranges: Vec<(usize, usize)>,
    /// Lines in the document.
    pub total: usize,
    /// The last line included was cut short: one line longer than all the room.
    pub cut: bool,
}

impl Picked {
    /// Every line went in.
    pub fn whole(&self) -> bool {
        !self.cut && (self.ranges == [(1, self.total)] || self.total == 0)
    }

    /// Only the opening lines went in.
    pub fn start_only(&self) -> bool {
        !self.whole() && self.ranges.len() == 1 && self.ranges[0].0 == 1
    }

    /// "lines 1–40, 120–160 and 300 of 900".
    pub fn describe(&self) -> String {
        let runs: Vec<String> = self.ranges.iter().map(|&(a, b)| if a == b { a.to_string() } else { format!("{a}–{b}") }).collect();
        let list = match runs.as_slice() {
            [] => String::new(),
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        };
        format!("{} {list} of {}", if runs.len() == 1 && !list.contains('–') { "line" } else { "lines" }, self.total)
    }
}

/// How much of the room a character takes: one for ASCII, four for anything else (see
/// `askabout::fit`).
fn cost(s: &str) -> usize {
    s.chars().map(|c| if c.is_ascii() { 1 } else { 4 }).sum()
}

/// What `text` that fits in `room` best answers `question`. `number` puts each line's number in
/// front of it, so the model can cite it.
pub fn pick(text: &str, room: usize, question: &str, number: bool) -> Picked {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let line = |i: usize| if number { format!("{}: {}", i + 1, lines[i]) } else { lines[i].to_string() };
    let all: usize = (0..total).map(|i| cost(&line(i)) + 1).sum();
    if all <= room {
        let text = (0..total).map(line).collect::<Vec<_>>().join("\n");
        return Picked { text, ranges: if total == 0 { Vec::new() } else { vec![(1, total)] }, total, cut: false };
    }
    let chunks = chunks(&lines);
    let want = terms(question);
    let scores: Vec<usize> = chunks.iter().map(|&(a, b)| score(&lines[a..=b], &want)).collect();
    let spread = spread_order(chunks.len());
    let rank: Vec<usize> = {
        let mut r = vec![0; chunks.len()];
        for (k, &i) in spread.iter().enumerate() {
            r[i] = k;
        }
        r
    };
    // The start always goes first: titles and definitions are there. Then the best matches, and
    // where nothing matches (or among equal matches), passages spread through the document.
    let mut order: Vec<usize> = (1..chunks.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(scores[i]), rank[i]));
    order.insert(0, 0);
    let gap = cost("[…]") + 1;
    let mut left = room;
    let mut chosen = vec![false; chunks.len()];
    for i in order {
        let (a, b) = chunks[i];
        let c: usize = (a..=b).map(|k| cost(&line(k)) + 1).sum::<usize>() + gap;
        if c <= left {
            chosen[i] = true;
            left -= c;
        } else if i == 0 {
            // A huge opening paragraph: as many of its lines as fit.
            let mut end = a;
            let mut used = gap;
            while end <= b && used + cost(&line(end)) < left {
                used += cost(&line(end)) + 1;
                end += 1;
            }
            if end > a {
                return finish(&[(a, end - 1)], total, &line);
            }
            // Its first line alone is longer than the room: as much of that line as fits.
            let mut text = String::new();
            let mut used = gap;
            for ch in line(a).chars() {
                used += cost(&ch.to_string());
                if used > left {
                    break;
                }
                text.push(ch);
            }
            return Picked { text, ranges: vec![(1, 1)], total, cut: true };
        }
    }
    let runs: Vec<(usize, usize)> = chunks.iter().zip(&chosen).filter(|(_, c)| **c).map(|(r, _)| *r).collect();
    finish(&runs, total, &line)
}

fn finish(runs: &[(usize, usize)], total: usize, line: &dyn Fn(usize) -> String) -> Picked {
    // Neighbouring chunks join into one run.
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for &(a, b) in runs {
        match ranges.last_mut() {
            Some(last) if last.1 + 1 == a => last.1 = b,
            _ => ranges.push((a, b)),
        }
    }
    let mut out = Vec::new();
    for (k, &(a, b)) in ranges.iter().enumerate() {
        if k > 0 || a > 0 {
            out.push("[…]".to_string());
        }
        out.extend((a..=b).map(line));
    }
    if ranges.last().is_some_and(|r| r.1 + 1 < total) {
        out.push("[…]".to_string());
    }
    Picked { text: out.join("\n"), ranges: ranges.iter().map(|&(a, b)| (a + 1, b + 1)).collect(), total, cut: false }
}

/// Passages: paragraphs (lines between blank lines), at most 16 lines each. Indexes are 0-based
/// and inclusive.
fn chunks(lines: &[&str]) -> Vec<(usize, usize)> {
    const MOST: usize = 16;
    let mut out = Vec::new();
    let mut start = 0;
    for i in 0..lines.len() {
        let blank = lines[i].trim().is_empty();
        let end_of_para = blank || i + 1 == lines.len() || lines[i + 1].trim().is_empty();
        if end_of_para || i + 1 - start >= MOST {
            out.push((start, i));
            start = i + 1;
        }
    }
    // Blank lines alone aren't worth a place of their own: fold them into the passage before.
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (a, b) in out {
        let empty = lines[a..=b].iter().all(|l| l.trim().is_empty());
        match merged.last_mut() {
            Some(last) if empty => last.1 = b,
            _ => merged.push((a, b)),
        }
    }
    merged
}

/// A word's stem for matching: lower case, its first six letters.
fn stem(w: &str) -> String {
    w.chars().take(6).collect()
}

fn words(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 3).map(|w| stem(&w.to_lowercase()))
}

const STOP: &[&str] = &[
    "the", "and", "for", "are", "can", "use", "this", "that", "what", "which", "with", "from", "does", "how", "who", "why", "when", "where", "about", "file", "files",
    "explai", "summar", "plain", "englis", "someon", "isn", "lawyer", "should", "would", "could", "there", "these", "those", "into", "any", "all", "you", "your", "its", "it",
    "they", "them", "their", "have", "has", "had", "not", "but", "was", "were", "will", "say", "says", "tell", "give", "list", "show", "mean", "means", "docume", "licenc",
    "licens", "is", "do", "of", "in", "to", "a", "an", "or", "me", "my", "i", "be", "on", "at", "by", "as", "if", "so", "no", "yes",
];

/// What to look for in the document for `question`: its own words, and for questions about
/// licences and agreements the words their clauses use.
fn terms(question: &str) -> Vec<String> {
    const RELATED: &[(&[&str], &[&str])] = &[
        (&["commer", "sell", "busine", "profit", "money", "paid", "monet"], &["commer", "sell", "sale", "profit", "fee", "charge"]),
        (&["restri", "limit", "condit", "allowe", "allow", "permit", "permis", "prohib"], &["restri", "condit", "limit", "permit", "permis", "prohib", "must", "shall", "except", "grant"]),
        (&["warran", "liabil", "liable", "guaran", "damage", "risk"], &["warran", "liabil", "liable", "disclai", "damage"]),
        (&["credit", "attrib", "notice", "mentio"], &["attrib", "credit", "notice", "copyri"]),
        (&["modify", "change", "adapt", "deriva", "fork"], &["modify", "modifi", "adapt", "deriva", "change"]),
        (&["share", "distri", "redist", "publis", "copy"], &["distri", "redist", "copy", "copies", "publis", "share"]),
        (&["patent"], &["patent"]),
        (&["tradem", "brand", "logo"], &["tradem"]),
        (&["error", "fail", "crash", "wrong", "proble", "warn"], &["error", "fail", "except", "warn", "fatal", "panic", "crash"]),
    ];
    let mut out: Vec<String> = Vec::new();
    for w in words(question) {
        if STOP.contains(&w.as_str()) {
            continue;
        }
        for (when, add) in RELATED {
            if when.iter().any(|k| w.starts_with(k) || k.starts_with(w.as_str()) && w.len() >= 4) {
                out.extend(add.iter().map(|s| s.to_string()));
            }
        }
        out.push(w);
    }
    out.sort();
    out.dedup();
    out
}

/// How well a passage matches: distinct terms found count most, then how often.
fn score(lines: &[&str], want: &[String]) -> usize {
    if want.is_empty() {
        return 0;
    }
    let mut found = vec![0usize; want.len()];
    for l in lines {
        for w in words(l) {
            if let Ok(i) = want.binary_search(&w) {
                found[i] += 1;
            }
        }
    }
    found.iter().filter(|&&n| n > 0).count() * 10 + found.iter().sum::<usize>().min(9)
}

/// Passage indexes in an order that covers the document evenly: the start, the end, the middle,
/// the quarters, and so on.
fn spread_order(n: usize) -> Vec<usize> {
    let mut out = Vec::with_capacity(n);
    let mut seen = vec![false; n];
    let mut push = |i: usize, out: &mut Vec<usize>| {
        if i < n && !seen[i] {
            seen[i] = true;
            out.push(i);
        }
    };
    if n == 0 {
        return out;
    }
    push(0, &mut out);
    push(n - 1, &mut out);
    let mut queue = std::collections::VecDeque::from([(0usize, n - 1)]);
    while let Some((a, b)) = queue.pop_front() {
        if b <= a + 1 {
            continue;
        }
        let m = (a + b) / 2;
        push(m, &mut out);
        queue.push_back((a, m));
        queue.push_back((m, b));
    }
    for i in 0..n {
        push(i, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn licence() -> String {
        let mut t = String::from("Creative Commons Legal Code\n\nCC0 1.0 Universal\n\n");
        for i in 0..40 {
            t.push_str(&format!("Paragraph {i} about the public domain and the affirmer's intent in general terms.\nIt goes on for a while.\n\n"));
        }
        t.push_str("4. Limitations and Disclaimers.\nNo trademark or patent rights held by Affirmer are waived.\nAffirmer offers the Work as-is and makes no warranties of any kind.\n\n");
        for i in 40..60 {
            t.push_str(&format!("Paragraph {i} with more general words.\n\n"));
        }
        t.push_str("The Work may be used for any purpose whatsoever, including commercial purposes.\n");
        t
    }

    #[test]
    fn small_documents_go_in_whole() {
        let p = pick("a\nb\nc", 100, "anything", true);
        assert_eq!(p.text, "1: a\n2: b\n3: c");
        assert!(p.whole());
    }

    #[test]
    fn finds_the_clauses_a_question_is_about() {
        let t = licence();
        let p = pick(&t, 900, "Can I use this commercially?", true);
        assert!(!p.whole());
        assert!(p.text.starts_with("1: Creative Commons Legal Code"), "{}", p.text);
        assert!(p.text.contains("including commercial purposes"), "{}", p.text);
        assert!(p.text.contains("[…]"));
        assert!(p.ranges.len() > 1);
        let w = pick(&t, 900, "Is there a warranty?", true);
        assert!(w.text.contains("makes no warranties"), "{}", w.text);
        // Line numbers are the real ones.
        let n = t.lines().position(|l| l.contains("commercial purposes")).unwrap() + 1;
        assert!(p.text.contains(&format!("{n}: The Work may be used")), "{}", p.text);
        assert!(p.ranges.iter().any(|&(a, b)| a <= n && n <= b));
    }

    #[test]
    fn general_questions_cover_the_whole_document() {
        let t = licence();
        let p = pick(&t, 1200, "Summarise this licence", false);
        assert!(!p.start_only(), "{:?}", p.ranges);
        let last = t.lines().count();
        assert!(p.ranges.iter().any(|&(_, b)| b == last), "{:?}", p.ranges);
        assert!(!p.text.contains("1: "));
    }

    #[test]
    fn describes_what_went_in() {
        let p = Picked { text: String::new(), ranges: vec![(1, 40), (120, 160), (300, 300)], total: 900, cut: false };
        assert_eq!(p.describe(), "lines 1–40, 120–160 and 300 of 900");
        assert!(!p.whole() && !p.start_only());
        let s = Picked { text: String::new(), ranges: vec![(1, 40)], total: 900, cut: false };
        assert!(s.start_only());
    }

    #[test]
    fn a_huge_first_paragraph_is_cut_to_fit() {
        let t = (0..100).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let p = pick(&t, 100, "x", false);
        assert!(p.start_only(), "{:?}", p.ranges);
        assert!(cost(&p.text) <= 100);
    }

    #[test]
    fn one_very_long_line_is_cut() {
        let t = "y".repeat(1000);
        let p = pick(&t, 100, "", true);
        assert!(p.start_only() && p.cut, "{p:?}");
        assert!(p.text.starts_with("1: yyy") && cost(&p.text) <= 100);
    }

    #[test]
    fn spread_starts_at_both_ends() {
        assert_eq!(&spread_order(9)[..3], &[0, 8, 4]);
        let mut all = spread_order(7);
        all.sort();
        assert_eq!(all, (0..7).collect::<Vec<_>>());
    }
}
