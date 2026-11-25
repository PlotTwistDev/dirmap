//! Spreadsheets (Excel .xlsx/.xlsm/.xlsb/.xls, OpenDocument .ods): the cells' saved values, read
//! with calamine (pure Rust). Formulas are never worked out and macros never run: what's shown
//! is what the program that saved the file last calculated.

use calamine::{Data, Reader};
use std::path::Path;

/// Larger files aren't read (a sheet is read whole before its first rows can be taken).
pub const MAX_FILE: u64 = 48 << 20;
/// How much of each sheet is kept.
pub const ROWS: usize = 500;
pub const COLS: usize = 40;
/// How many sheets are read.
const SHEETS: usize = 12;

#[derive(Debug, Clone)]
pub struct Sheet {
    pub name: String,
    /// The cells, as text, from the top left of the used area.
    pub rows: Vec<Vec<String>>,
    /// The sheet's full size (rows, columns).
    pub size: (usize, usize),
}

#[derive(Debug, Clone)]
pub struct Book {
    pub sheets: Vec<Sheet>,
    /// Sheets in the file (some may not have been read).
    pub total: usize,
}

/// A cell's value in words.
fn text(d: &Data) -> String {
    match d {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", *f as i64),
        Data::Float(f) => {
            let s = format!("{f:.6}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        Data::DateTime(dt) => match dt.as_datetime() {
            Some(t) if dt.is_duration() => format!("{}", t.format("%H:%M:%S")),
            Some(t) if t.time() == chrono::NaiveTime::MIN => t.format("%Y-%m-%d").to_string(),
            Some(t) => t.format("%Y-%m-%d %H:%M").to_string(),
            None => dt.as_f64().to_string(),
        },
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#{e:?}").to_uppercase(),
    }
}

/// Read a workbook. Runs on worker threads.
pub fn read(path: &Path, rows: usize, cols: usize) -> Result<Book, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("Couldn't open it: {e}"))?.len();
    if size > MAX_FILE {
        return Err(format!("Too big to preview ({}).", crate::scan::human(size)));
    }
    let mut wb = calamine::open_workbook_auto(path).map_err(|e| format!("Couldn't read the workbook: {e}"))?;
    let names = wb.sheet_names();
    let mut sheets = Vec::new();
    for name in names.iter().take(SHEETS) {
        let Ok(range) = wb.worksheet_range(name) else { continue };
        let (h, w) = range.get_size();
        let rows: Vec<Vec<String>> = range.rows().take(rows).map(|r| r.iter().take(cols).map(text).collect()).collect();
        sheets.push(Sheet { name: name.clone(), rows, size: (h, w) });
    }
    if sheets.is_empty() {
        return Err("This workbook has no sheets Filescape can read.".into());
    }
    Ok(Book { sheets, total: names.len() })
}

/// A spreadsheet column's letters: 0 → A, 26 → AA.
pub fn column(mut i: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// The first rows as text for a tile: cells joined by two spaces.
pub fn snippet(book: &Book) -> String {
    let s = &book.sheets[0];
    s.rows.iter().take(40).map(|r| r.iter().map(|c| c.replace(['\n', '\t'], " ")).collect::<Vec<_>>().join("  ").trim_end().to_string()).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_and_values() {
        assert_eq!(column(0), "A");
        assert_eq!(column(25), "Z");
        assert_eq!(column(26), "AA");
        assert_eq!(column(701), "ZZ");
        assert_eq!(column(702), "AAA");
        assert_eq!(text(&Data::Float(3.0)), "3");
        assert_eq!(text(&Data::Float(2.50)), "2.5");
        assert_eq!(text(&Data::Bool(true)), "TRUE");
        assert!(read(Path::new("/no/such.xlsx"), 5, 5).is_err());
    }

    /// A real workbook: an .ods (a zip of XML), built here.
    #[test]
    fn reads_an_ods() {
        use std::io::Write;
        let p = std::env::temp_dir().join(format!("fs-sheet-{}.ods", std::process::id()));
        let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("mimetype", opt).unwrap();
        z.write_all(b"application/vnd.oasis.opendocument.spreadsheet").unwrap();
        z.start_file("META-INF/manifest.xml", opt).unwrap();
        z.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#).unwrap();
        z.start_file("content.xml", opt).unwrap();
        z.write_all(br#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2">
<office:body><office:spreadsheet><table:table table:name="Budget">
<table:table-row><table:table-cell office:value-type="string"><text:p>Item</text:p></table:table-cell><table:table-cell office:value-type="string"><text:p>Cost</text:p></table:table-cell></table:table-row>
<table:table-row><table:table-cell office:value-type="string"><text:p>Paint</text:p></table:table-cell><table:table-cell office:value-type="float" office:value="12.5"><text:p>12.5</text:p></table:table-cell></table:table-row>
</table:table></office:spreadsheet></office:body></office:document-content>"#).unwrap();
        z.finish().unwrap();
        let b = read(&p, 10, 10);
        let _ = std::fs::remove_file(&p);
        let b = b.unwrap();
        assert_eq!(b.sheets[0].name, "Budget");
        assert_eq!(b.sheets[0].rows, vec![vec!["Item", "Cost"], vec!["Paint", "12.5"]]);
        assert!(snippet(&b).starts_with("Item  Cost\nPaint  12.5"));
    }
}
