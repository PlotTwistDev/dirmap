//! Tables that aren't spreadsheets: SQLite databases (rusqlite with SQLite built in; MIT, and
//! SQLite itself is public domain) and CSV/TSV files (the csv crate; MIT or Unlicense). Both are
//! read into the same shape as a workbook, so they show in the same table view.
//!
//! Databases are opened read-only and as immutable (nothing is written, not even a lock or a
//! journal), with the schema untrusted and a time limit on every query, so a crafted file can
//! neither change anything nor hang the preview.

use crate::sheets::{Book, Sheet};
use std::path::Path;
use std::time::{Duration, Instant};

/// How many tables are read.
const TABLES: usize = 24;
/// Longest cell text kept.
const CELL: usize = 200;
/// CSV files larger than this are read only as far as the rows shown.
const CSV_SCAN: u64 = 64 << 20;

/// SQLite's first 16 bytes.
pub const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

pub fn is_csv(path: &Path) -> bool {
    path.extension().is_some_and(|e| matches!(e.to_ascii_lowercase().to_str(), Some("csv" | "tsv" | "tab")))
}

fn header(path: &Path) -> Option<[u8; 20]> {
    use std::io::Read;
    let mut head = [0u8; 20];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).ok()?;
    head.starts_with(SQLITE_MAGIC).then_some(head)
}

pub fn is_sqlite(path: &Path) -> bool {
    header(path).is_some()
}

fn cut(mut s: String) -> String {
    if s.len() > CELL {
        let mut at = CELL;
        while !s.is_char_boundary(at) {
            at -= 1;
        }
        s.truncate(at);
        s.push('…');
    }
    s
}

fn value(v: rusqlite::types::ValueRef) -> String {
    use rusqlite::types::ValueRef as V;
    match v {
        V::Null => String::new(),
        V::Integer(i) => i.to_string(),
        V::Real(f) => {
            let s = format!("{f:.6}");
            let s = s.trim_end_matches('0').trim_end_matches('.');
            if s.is_empty() || s == "-" { "0".into() } else { s.to_string() }
        }
        V::Text(t) => cut(String::from_utf8_lossy(t).into_owned()),
        V::Blob(b) => format!("‹{} of data›", crate::scan::human(b.len() as u64)),
    }
}

/// A name in double quotes, for SQL.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// The database as a URI that opens it read-only and immutable.
fn uri(path: &Path) -> String {
    let mut p = path.to_string_lossy().replace('\\', "/");
    if !p.starts_with('/') {
        p.insert(0, '/'); // C:/… → /C:/…
    }
    let mut out = String::from("file:");
    for c in p.chars() {
        match c {
            '%' | '?' | '#' => out.push_str(&format!("%{:02X}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push_str("?mode=ro&immutable=1");
    out
}

/// Read a database: its tables' first rows, with every table's row count and, when `sizes` is
/// set, the space each one takes in the file (its own pages and its indexes'), largest first.
/// Sizes read every page of the file, so tile snippets leave them out.
pub fn read_sqlite(path: &Path, rows: usize, cols: usize, sizes: bool) -> Result<Book, String> {
    use rusqlite::{Connection, OpenFlags};
    use std::sync::{Arc, Mutex};
    if !is_sqlite(path) {
        return Err("This isn't a SQLite database.".into());
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut db = Connection::open_with_flags(uri(path), flags).map_err(|e| format!("Couldn't open the database: {e}"))?;
    // Nothing in the file is trusted to run: no functions from views or triggers, and the
    // defensive switches that stop a query from changing the file.
    let _ = db.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false);
    let _ = db.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true);
    // Every query is stopped after a short while (counting rows in a huge table, say). Each step
    // gets its own allowance, so one slow table doesn't cost the others theirs.
    let deadline = Arc::new(Mutex::new(Instant::now() + Duration::from_secs(4)));
    let d = deadline.clone();
    let _ = db.progress_handler(10_000, Some(move || d.lock().is_ok_and(|d| Instant::now() > *d)));
    let allow = |ms: u64| {
        if let Ok(mut d) = deadline.lock() {
            *d = Instant::now() + Duration::from_millis(ms);
        }
    };
    let fail = |e: rusqlite::Error| match e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::OperationInterrupted => "Reading the database took too long.".to_string(),
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::NotADatabase => "This database is encrypted or damaged.".to_string(),
        e => format!("Couldn't read the database: {e}"),
    };
    let mut q = db.prepare("SELECT type, name, tbl_name FROM sqlite_schema WHERE name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY type, name").map_err(fail)?;
    let all: Vec<(String, String, String)> = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<String>>(2)?.unwrap_or_default()))).map_err(fail)?.filter_map(|r| r.ok()).collect();
    drop(q);
    let count = |t: &str| all.iter().filter(|(k, ..)| k == t).count();
    let mut tables: Vec<&String> = all.iter().filter(|(k, ..)| k == "table").map(|(_, n, _)| n).collect();
    let listed = tables.len();
    // A full-text search index (or an R-tree) keeps its data in tables of its own, named after
    // it: messages_fts_data, messages_fts_idx and so on. They're folded into the index they
    // belong to, which is where their rows show and their space is counted.
    let shadow: Vec<String> = db
        .prepare("SELECT name FROM pragma_table_list WHERE schema = 'main' AND type = 'shadow'")
        .and_then(|mut q| q.query_map([], |r| r.get::<_, String>(0))?.collect())
        .unwrap_or_default();
    let virt: Vec<&String> = all.iter().filter(|(k, _, _)| k == "table").map(|(_, n, _)| n).filter(|n| !shadow.contains(n)).collect();
    let owner_of = |name: &str| -> Option<&String> {
        shadow.iter().any(|s| s == name).then(|| virt.iter().filter(|v| name.len() > v.len() + 1 && name.starts_with(v.as_str()) && name.as_bytes()[v.len()] == b'_').max_by_key(|v| v.len()).copied()).flatten()
    };
    tables.retain(|t| owner_of(t).is_none());
    let folded = listed - tables.len();
    let page = db.query_row("PRAGMA page_size", [], |r| r.get::<_, i64>(0)).unwrap_or(4096).max(512) as u64;
    // The space each table takes: dbstat walks every b-tree once, summing its pages. An index
    // counts towards its table; the schema's own pages are left out.
    let space: Option<std::collections::HashMap<&str, u64>> = sizes
        .then(|| {
            allow(8_000);
            // Page by page rather than dbstat's own totals, so the time limit can stop it.
            let mut q = db.prepare("SELECT name, sum(pgsize) FROM dbstat('main') GROUP BY name").ok()?;
            let got: Vec<(String, i64)> = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).ok()?.collect::<Result<_, _>>().ok()?;
            let mut by = std::collections::HashMap::new();
            for (name, bytes) in got {
                let owner = all.iter().find(|(_, n, _)| *n == name).map(|(_, _, t)| t.as_str()).filter(|t| !t.is_empty()).unwrap_or(name.as_str());
                let owner = owner_of(owner).map_or(owner, |v| v.as_str());
                if let Some(t) = tables.iter().find(|t| t.as_str() == owner) {
                    *by.entry(t.as_str()).or_insert(0) += bytes.max(0) as u64;
                }
            }
            Some(by)
        })
        .flatten();
    let bytes_of = |name: &str| space.as_ref().map(|m| m.get(name).copied().unwrap_or(0));
    // Largest first, so the preview opens where the space is.
    if space.is_some() {
        tables.sort_by_key(|t| std::cmp::Reverse(bytes_of(t)));
    }
    let mut sheets = Vec::new();
    // Counting rows reads a table's smallest index from end to end: a second for each table at
    // most, and twelve in all, after which the rest show as "at least".
    let mut counting = Duration::from_secs(12);
    for name in tables.iter().take(TABLES) {
        allow(2_000);
        let mut q = match db.prepare(&format!("SELECT * FROM {} LIMIT {}", quoted(name), rows)) {
            Ok(q) => q,
            Err(_) => continue, // a virtual table whose module isn't here, say
        };
        let heads: Vec<String> = q.column_names().take(cols).map(|s| s.to_string()).collect();
        let width = q.column_count();
        let n = heads.len();
        let mut whole = true;
        let got: Vec<Vec<String>> = match q.query_map([], |r| Ok((0..n).map(|i| r.get_ref(i).map(value).unwrap_or_default()).collect::<Vec<String>>())) {
            Ok(it) => it.map_while(|r| r.inspect_err(|_| whole = false).ok()).collect(),
            Err(_) => continue,
        };
        drop(q);
        // A table that fits in what was read needs no count.
        let total = if whole && got.len() < rows {
            Some(got.len() as i64)
        } else if counting.is_zero() {
            None
        } else {
            let t0 = Instant::now();
            allow(counting.min(Duration::from_secs(1)).as_millis() as u64);
            let t = db.query_row(&format!("SELECT count(*) FROM {}", quoted(name)), [], |r| r.get::<_, i64>(0)).ok();
            counting = counting.saturating_sub(t0.elapsed());
            t
        };
        sheets.push(Sheet { name: name.to_string(), size: (total.map_or(got.len(), |t| t as usize), width), rows: got, heads, rows_known: total.is_some(), bytes: bytes_of(name) });
    }
    // Without sizes, the table with the most rows comes first instead (a tile's snippet, say).
    if space.is_none() {
        sheets.sort_by_key(|s| std::cmp::Reverse(s.size.0));
    }
    // The facts first, so an empty database still says what's in it.
    let mut facts = Vec::new();
    let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut t = plural(tables.len(), "table", "tables");
    if folded > 0 {
        t.push_str(&format!(" (and {folded} that hold search indexes, counted with them)"));
    }
    facts.push(("Tables", t));
    for (t, one, many, label) in [("view", "view", "views", "Views"), ("index", "index", "indexes", "Indexes"), ("trigger", "trigger", "triggers", "Triggers")] {
        let n = count(t);
        if n > 0 {
            facts.push((label, plural(n, one, many)));
        }
    }
    if sizes && space.is_none() {
        facts.push(("Sizes", "Not measured: reading every page of this file took too long".into()));
    }
    if space.is_some() {
        let big: Vec<String> = tables.iter().take(3).filter_map(|t| bytes_of(t).filter(|b| *b > 0).map(|b| format!("{t} {}", crate::scan::human(b)))).collect();
        if !big.is_empty() {
            facts.push(("Largest", big.join(" · ")));
        }
    }
    // Pages the database has let go of but the file still holds, until a VACUUM.
    allow(1_000);
    if let Ok(free) = db.query_row("PRAGMA freelist_count", [], |r| r.get::<_, i64>(0))
        && free > 0
    {
        facts.push(("Unused", format!("{} of free pages inside the file (VACUUM gives it back)", crate::scan::human(free as u64 * page))));
    }
    // Bytes 18 and 19 of the header are 2 in write-ahead-log mode.
    if header(path).is_some_and(|h| h[18] == 2 && h[19] == 2) {
        facts.push(("Journal", "Write-ahead log (recent changes may be in the -wal file beside it)".into()));
    }
    if sheets.is_empty() {
        return Err(if tables.is_empty() { "This database has no tables.".into() } else { "None of this database's tables could be read.".into() });
    }
    Ok(Book { sheets, total: tables.len(), noun: "tables", facts })
}

/// A CSV or TSV file: its first rows. The first row is taken as the column names when it looks
/// like them (all text, and not repeated).
pub fn read_csv(path: &Path, rows: usize, cols: usize) -> Result<Book, String> {
    use std::io::Read;
    let tsv = path.extension().is_some_and(|e| !e.eq_ignore_ascii_case("csv"));
    let file = std::fs::File::open(path).map_err(|e| format!("Couldn't open it: {e}"))?;
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut head = Vec::new();
    file.take(64 << 10).read_to_end(&mut head).map_err(|e| format!("Couldn't read it: {e}"))?;
    // Comma, semicolon (European spreadsheets) or tab: whichever splits the first line most.
    let first = head.split(|&b| b == b'\n').next().unwrap_or(&[]);
    let delim = if tsv {
        b'\t'
    } else {
        *[b',', b';', b'\t', b'|'].iter().max_by_key(|d| first.iter().filter(|b| b == d).count()).unwrap_or(&b',')
    };
    let file = std::fs::File::open(path).map_err(|e| format!("Couldn't open it: {e}"))?;
    let mut rd = csv::ReaderBuilder::new().delimiter(delim).has_headers(false).flexible(true).from_reader(file);
    let mut all: Vec<Vec<String>> = Vec::new();
    let mut total = 0usize;
    let mut width = 0usize;
    let mut rec = csv::ByteRecord::new();
    let scan_all = size <= CSV_SCAN;
    while rd.read_byte_record(&mut rec).map_err(|e| format!("Couldn't read it as CSV: {e}"))? {
        total += 1;
        width = width.max(rec.len());
        if all.len() <= rows {
            all.push(rec.iter().take(cols).map(|f| cut(String::from_utf8_lossy(f).into_owned())).collect());
        } else if !scan_all {
            break;
        }
    }
    if all.is_empty() {
        return Err("This file is empty.".into());
    }
    let looks_named = all.len() > 1 && {
        let h = &all[0];
        let mut seen = std::collections::HashSet::new();
        h.iter().all(|c| !c.trim().is_empty() && c.parse::<f64>().is_err() && seen.insert(c.as_str()))
    };
    let heads = if looks_named { all.remove(0) } else { Vec::new() };
    if looks_named {
        total -= 1;
    } else {
        all.truncate(rows);
    }
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Book { sheets: vec![Sheet { name, rows: all, size: (total, width), heads, rows_known: scan_all, bytes: None }], total: 1, noun: "sheets", facts: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_database_without_changing_it() {
        let dir = std::env::temp_dir().join(format!("filescape-sql-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("shop #%.sqlite");
        {
            let db = rusqlite::Connection::open(&p).unwrap();
            db.execute_batch(
                "CREATE TABLE \"odd \"\"name\"\"\" (id INTEGER PRIMARY KEY, title TEXT, price REAL, pic BLOB);
                 INSERT INTO \"odd \"\"name\"\"\" VALUES (1, 'Kettle', 19.5, x'00112233'), (2, NULL, 3.0, NULL), (3, 'Mug', 4.25, NULL);
                 CREATE TABLE empty (a, b);
                 CREATE INDEX by_title ON \"odd \"\"name\"\"\"(title);
                 CREATE VIEW cheap AS SELECT * FROM \"odd \"\"name\"\"\" WHERE price < 5;",
            )
            .unwrap();
        }
        let before = std::fs::read(&p).unwrap();
        let book = read_sqlite(&p, 2, 40, true).unwrap();
        let facts = &book.facts;
        assert_eq!(std::fs::read(&p).unwrap(), before);
        assert!(!dir.join("shop #%.sqlite-journal").exists() && !dir.join("shop #%.sqlite-wal").exists());
        assert_eq!(book.noun, "tables");
        assert_eq!(book.sheets.len(), 2);
        let s = book.sheets.iter().find(|s| s.name == "odd \"name\"").unwrap();
        assert_eq!(s.heads, ["id", "title", "price", "pic"]);
        assert_eq!(s.size, (3, 4)); // all 3 rows counted, 2 read
        assert_eq!(s.rows, vec![vec!["1", "Kettle", "19.5", "‹4 B of data›"], vec!["2", "", "3", ""]]);
        assert!(facts.contains(&("Tables", "2 tables".into())));
        assert!(facts.contains(&("Views", "1 view".into())));
        assert!(facts.contains(&("Indexes", "1 index".into())));
        assert!(!facts.iter().any(|f| f.0 == "Journal"));
        // Sizes: the table with rows (and an index) comes first, and the preview opens on it.
        assert_eq!(book.sheets[0].name, "odd \"name\"");
        assert_eq!(s.bytes, Some(2 * 4096)); // its own page and its index's
        assert_eq!(book.sheets[1].bytes, Some(4096));
        assert!(facts.iter().any(|f| f.0 == "Largest" && f.1.starts_with("odd \"name\" ")));
        // Tiles leave sizes out.
        assert!(read_sqlite(&p, 2, 40, false).unwrap().sheets.iter().all(|s| s.bytes.is_none()));
        std::fs::write(&p, b"not a database at all, just some words").unwrap();
        assert!(read_sqlite(&p, 5, 5, true).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A big database whose first table is empty, like a chat app's: the preview opens on the
    /// table that holds the space, counts what it can, and still changes nothing.
    #[test]
    fn opens_on_the_largest_table() {
        let dir = std::env::temp_dir().join(format!("filescape-sqlbig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("state.db");
        {
            let db = rusqlite::Connection::open(&p).unwrap();
            db.execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE compression_locks (session_id, holder);
                 CREATE TABLE messages (id INTEGER PRIMARY KEY, body TEXT);
                 CREATE VIRTUAL TABLE messages_fts USING fts5(body, content='messages', content_rowid='id');
                 CREATE TABLE scratch (x);
                 WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 3000)
                   INSERT INTO messages (body) SELECT 'message number ' || i || ' ' || hex(randomblob(200)) FROM n;
                 INSERT INTO messages_fts (messages_fts) VALUES ('rebuild');
                 WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 500)
                   INSERT INTO scratch SELECT randomblob(2000) FROM n;
                 DELETE FROM scratch;
                 PRAGMA wal_checkpoint(TRUNCATE);",
            )
            .unwrap();
        }
        let before = std::fs::read(&p).unwrap();
        let book = read_sqlite(&p, 50, 40, true).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), before);
        // The search index's own tables are folded into it, so it's the biggest; messages next.
        assert_eq!(book.sheets[0].name, "messages_fts");
        assert!(!book.sheets.iter().any(|s| s.name.starts_with("messages_fts_")));
        assert_eq!(book.sheets[0].size.0, 3000);
        assert!(book.sheets[0].rows_known);
        assert_eq!(book.sheets[1].name, "messages");
        assert!(book.facts.iter().any(|f| f.0 == "Tables" && f.1.starts_with("4 tables (and 4 ")), "{:?}", book.facts);
        let sizes: Vec<u64> = book.sheets.iter().map(|s| s.bytes.unwrap()).collect();
        assert!(sizes.windows(2).all(|w| w[0] >= w[1]), "{sizes:?}");
        let empty = book.sheets.iter().find(|s| s.name == "compression_locks").unwrap();
        assert_eq!((empty.size.0, empty.rows_known), (0, true));
        assert!(book.facts.iter().any(|f| f.0 == "Unused"), "{:?}", book.facts);
        assert!(book.facts.iter().any(|f| f.0 == "Journal"));
        // Without sizes (a tile's snippet), a table with rows still comes before the empty one.
        let tile = read_sqlite(&p, 50, 40, false).unwrap();
        assert_eq!(tile.sheets[0].size.0, 3000);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reads_csv_with_and_without_names() {
        let dir = std::env::temp_dir().join(format!("filescape-csv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("people.csv");
        std::fs::write(&p, "name;age;city\nAnn;31;\"Paris; France\"\nBo;7\n").unwrap();
        let b = read_csv(&p, 10, 10).unwrap();
        let s = &b.sheets[0];
        assert_eq!(s.heads, ["name", "age", "city"]);
        assert_eq!(s.rows, vec![vec!["Ann", "31", "Paris; France"], vec!["Bo", "7"]]);
        assert_eq!(s.size, (2, 3));
        let p = dir.join("n.tsv");
        std::fs::write(&p, "1\t2\n3\t4\n5\t6\n").unwrap();
        let b = read_csv(&p, 2, 10).unwrap();
        assert!(b.sheets[0].heads.is_empty());
        assert_eq!(b.sheets[0].rows.len(), 2);
        assert_eq!(b.sheets[0].size, (3, 2));
        std::fs::remove_dir_all(&dir).ok();
    }
}
