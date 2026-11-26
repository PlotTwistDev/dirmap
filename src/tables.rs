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

pub fn read_sqlite(path: &Path, rows: usize, cols: usize) -> Result<Book, String> {
    use rusqlite::{Connection, OpenFlags};
    if !is_sqlite(path) {
        return Err("This isn't a SQLite database.".into());
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut db = Connection::open_with_flags(uri(path), flags).map_err(|e| format!("Couldn't open the database: {e}"))?;
    // Nothing in the file is trusted to run: no functions from views or triggers, and the
    // defensive switches that stop a query from changing the file.
    let _ = db.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false);
    let _ = db.set_db_config(rusqlite::config::DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true);
    // Every query is stopped after a short while (counting rows in a huge table, say).
    let deadline = Instant::now() + Duration::from_secs(4);
    let _ = db.progress_handler(10_000, Some(move || Instant::now() > deadline));
    let fail = |e: rusqlite::Error| match e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::OperationInterrupted => "Reading the database took too long.".to_string(),
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::NotADatabase => "This database is encrypted or damaged.".to_string(),
        e => format!("Couldn't read the database: {e}"),
    };
    let mut q = db.prepare("SELECT type, name FROM sqlite_schema WHERE name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY type, name").map_err(fail)?;
    let all: Vec<(String, String)> = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(fail)?.filter_map(|r| r.ok()).collect();
    drop(q);
    let count = |t: &str| all.iter().filter(|(k, _)| k == t).count();
    let tables: Vec<&String> = all.iter().filter(|(k, _)| k == "table").map(|(_, n)| n).collect();
    let mut sheets = Vec::new();
    for name in tables.iter().take(TABLES) {
        let mut q = match db.prepare(&format!("SELECT * FROM {} LIMIT {}", quoted(name), rows)) {
            Ok(q) => q,
            Err(_) => continue, // a virtual table whose module isn't here, say
        };
        let heads: Vec<String> = q.column_names().take(cols).map(|s| s.to_string()).collect();
        let width = q.column_count();
        let n = heads.len();
        let got: Vec<Vec<String>> = match q.query_map([], |r| Ok((0..n).map(|i| r.get_ref(i).map(value).unwrap_or_default()).collect::<Vec<String>>())) {
            Ok(it) => it.map_while(|r| r.ok()).collect(),
            Err(_) => continue,
        };
        drop(q);
        let total = db.query_row(&format!("SELECT count(*) FROM {}", quoted(name)), [], |r| r.get::<_, i64>(0)).ok();
        sheets.push(Sheet { name: name.to_string(), size: (total.map_or(got.len(), |t| t as usize), width), rows: got, heads, rows_known: total.is_some() });
    }
    // The facts first, so an empty database still says what's in it.
    let mut facts = Vec::new();
    let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    facts.push(("Tables", plural(tables.len(), "table", "tables")));
    for (t, one, many, label) in [("view", "view", "views", "Views"), ("index", "index", "indexes", "Indexes"), ("trigger", "trigger", "triggers", "Triggers")] {
        let n = count(t);
        if n > 0 {
            facts.push((label, plural(n, one, many)));
        }
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
    Ok(Book { sheets: vec![Sheet { name, rows: all, size: (total, width), heads, rows_known: scan_all }], total: 1, noun: "sheets", facts: Vec::new() })
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
        let book = read_sqlite(&p, 2, 40).unwrap();
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
        std::fs::write(&p, b"not a database at all, just some words").unwrap();
        assert!(read_sqlite(&p, 5, 5).is_err());
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
