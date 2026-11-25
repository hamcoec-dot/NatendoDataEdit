//! Open time of the import readers (xlsx, an xlsx whose every cell is a 16-19 digit whole number, JSON,
//! ODS, SQLite), for the speed gate against main (A1a). It uses only `import::import` and
//! `sqlite::read_table`, which both sides have, and writes its fixtures to a shared folder once, so the two
//! builds read the very same files.
//!
//! ```text
//! cargo run --release -p natendo-engine --example import_bench -- [rows] [repeats] [folder] [which]
//! ```
//! `which` is `all` (default), `xlsx`, `worst`, `json`, `ods` or `sqlite`. Prints the best and every time.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Instant;

use natendo_engine::import;
use natendo_engine::zipwrite::ZipWriter;
use natendo_engine::Progress;

const COLS: usize = 20;

fn col_name(c: usize) -> String {
    let mut s = Vec::new();
    let mut c = c;
    loop {
        s.push(b'A' + (c % 26) as u8);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// An xlsx with `rows` rows of 20 cells. `worst`: every cell a whole number of 16 to 19 digits (the worst
/// case for the digit count); else a mix of whole numbers, decimals and short text.
fn write_xlsx(path: &Path, rows: u64, worst: bool) {
    let mut zip = ZipWriter::new(BufWriter::with_capacity(
        1 << 20,
        File::create(path).unwrap(),
    ));
    let put = |zip: &mut ZipWriter<_>, name: &str, body: &str| {
        let mut p = zip.part(name).unwrap();
        p.write_all(body.as_bytes()).unwrap();
        p.finish().unwrap();
    };
    put(
        &mut zip,
        "_rels/.rels",
        r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
    );
    put(
        &mut zip,
        "xl/workbook.xml",
        r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
    );
    put(
        &mut zip,
        "xl/_rels/workbook.xml.rels",
        r#"<Relationships><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
    );
    put(
        &mut zip,
        "xl/styles.xml",
        r#"<styleSheet><cellXfs><xf numFmtId="0"/></cellXfs></styleSheet>"#,
    );
    {
        let mut p = zip.part("xl/worksheets/sheet1.xml").unwrap();
        p.write_all(b"<worksheet><sheetData>").unwrap();
        let mut w = BufWriter::with_capacity(1 << 16, &mut p);
        let names: Vec<String> = (0..COLS).map(col_name).collect();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for r in 0..rows {
            write!(w, "<row r=\"{}\">", r + 1).unwrap();
            for (c, name) in names.iter().enumerate() {
                let x = rng.next();
                if worst {
                    // 16 to 19 digits
                    let digits = 16 + (x % 4) as u32;
                    let v = 10u64.pow(digits - 1) + x % (9 * 10u64.pow(digits - 1));
                    write!(w, "<c r=\"{}{}\"><v>{v}</v></c>", name, r + 1).unwrap();
                } else if c % 3 == 0 {
                    write!(w, "<c r=\"{}{}\"><v>{}</v></c>", name, r + 1, x % 1_000_000).unwrap();
                } else if c % 3 == 1 {
                    write!(
                        w,
                        "<c r=\"{}{}\"><v>{}.{:02}</v></c>",
                        name,
                        r + 1,
                        x % 100_000,
                        x % 100
                    )
                    .unwrap();
                } else {
                    write!(
                        w,
                        "<c r=\"{}{}\" t=\"inlineStr\"><is><t>name {}</t></is></c>",
                        name,
                        r + 1,
                        x % 9973
                    )
                    .unwrap();
                }
            }
            w.write_all(b"</row>").unwrap();
        }
        w.flush().unwrap();
        drop(w);
        p.write_all(b"</sheetData></worksheet>").unwrap();
        p.finish().unwrap();
    }
    zip.finish().unwrap().flush().unwrap();
}

fn write_ods(path: &Path, rows: u64) {
    let mut zip = ZipWriter::new(BufWriter::with_capacity(
        1 << 20,
        File::create(path).unwrap(),
    ));
    let mut p = zip.part("mimetype").unwrap();
    p.write_all(b"application/vnd.oasis.opendocument.spreadsheet")
        .unwrap();
    p.finish().unwrap();
    let mut p = zip.part("content.xml").unwrap();
    p.write_all(br#"<office:document-content xmlns:office="o" xmlns:table="t" xmlns:text="x"><office:body><office:spreadsheet><table:table table:name="Data">"#).unwrap();
    {
        let mut w = BufWriter::with_capacity(1 << 16, &mut p);
        let mut rng = Rng(0x1234_5678_9ABC_DEF1);
        for _ in 0..rows {
            w.write_all(b"<table:table-row>").unwrap();
            for c in 0..COLS {
                let x = rng.next();
                if c % 2 == 0 {
                    write!(w, "<table:table-cell office:value-type=\"float\" office:value=\"{}.{:02}\"><text:p>{}</text:p></table:table-cell>", x % 100_000, x % 100, x % 100_000).unwrap();
                } else {
                    write!(w, "<table:table-cell office:value-type=\"string\"><text:p>name {}</text:p></table:table-cell>", x % 9973).unwrap();
                }
            }
            w.write_all(b"</table:table-row>").unwrap();
        }
        w.flush().unwrap();
    }
    p.write_all(b"</table:table></office:spreadsheet></office:body></office:document-content>")
        .unwrap();
    p.finish().unwrap();
    zip.finish().unwrap().flush().unwrap();
}

fn write_json(path: &Path, rows: u64) {
    let mut w = BufWriter::with_capacity(1 << 20, File::create(path).unwrap());
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    w.write_all(b"[").unwrap();
    for r in 0..rows {
        if r > 0 {
            w.write_all(b",").unwrap();
        }
        let x = rng.next();
        write!(
            w,
            "{{\"id\":{r},\"price\":{}.{:02},\"name\":\"name {}\",\"city\":\"City {}\",\"ok\":{},\"big\":{},\"note\":null",
            x % 100_000,
            x % 100,
            x % 9973,
            x % 311,
            x % 2 == 0,
            x % 1_000_000_007
        )
        .unwrap();
        for c in 0..12 {
            write!(w, ",\"v{c}\":{}", (x >> (c * 2)) % 1_000_000).unwrap();
        }
        w.write_all(b"}").unwrap();
    }
    w.write_all(b"]").unwrap();
    w.flush().unwrap();
}

fn write_sqlite(path: &Path, rows: u64) {
    let _ = std::fs::remove_file(path);
    let c = rusqlite::Connection::open(path).unwrap();
    c.execute_batch(
        "CREATE TABLE t(id INTEGER, price DECIMAL(10,2), name TEXT, city TEXT, r REAL, a TEXT, b TEXT, c TEXT, d TEXT, e TEXT);",
    )
    .unwrap();
    c.execute_batch("BEGIN").unwrap();
    {
        let mut st = c
            .prepare("INSERT INTO t VALUES (?,?,?,?,?,?,?,?,?,?)")
            .unwrap();
        let mut rng = Rng(0x0F0F_1234_5678_9ABC);
        for i in 0..rows {
            let x = rng.next();
            st.execute(rusqlite::params![
                i as i64,
                (x % 100_000) as f64 + (x % 100) as f64 / 100.0,
                format!("name {}", x % 9973),
                format!("City {}", x % 311),
                (x % 1_000_000) as f64 / 7.0,
                format!("v{}", x % 1000),
                format!("v{}", (x >> 8) % 1000),
                format!("v{}", (x >> 16) % 1000),
                format!("v{}", (x >> 24) % 1000),
                format!("v{}", (x >> 32) % 1000)
            ])
            .unwrap();
        }
    }
    c.execute_batch("COMMIT").unwrap();
}

fn best(name: &str, repeats: usize, mut f: impl FnMut() -> u64) {
    let mut times = Vec::new();
    let mut n = 0;
    for _ in 0..repeats {
        let t = Instant::now();
        n = f();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let b = times.iter().cloned().fold(f64::MAX, f64::min);
    let all: Vec<String> = times.iter().map(|t| format!("{t:.0}")).collect();
    println!("{name}: best {b:.0} ms ({n} rows) [{}]", all.join(", "));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rows: u64 = args
        .first()
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);
    let repeats: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(3);
    let dir = args.get(2).map_or_else(
        || std::env::temp_dir().join("nde-import-bench"),
        std::path::PathBuf::from,
    );
    let which = args.get(3).map_or("all", |s| s.as_str());
    std::fs::create_dir_all(&dir).unwrap();
    let want = |k: &str| which == "all" || which == k;
    let p = Progress::new();
    let imp = |path: &Path, ext: &str| -> u64 {
        let data = std::fs::read(path).unwrap();
        import::import(&data, Some(ext), &Default::default(), None, &p)
            .unwrap()
            .rows
    };
    if want("xlsx") {
        let f = dir.join(format!("mixed-{rows}.xlsx"));
        if !f.exists() {
            write_xlsx(&f, rows, false);
        }
        best("xlsx mixed", repeats, || imp(&f, "xlsx"));
    }
    if want("worst") {
        let n = rows / 4;
        let f = dir.join(format!("worst-{n}.xlsx"));
        if !f.exists() {
            write_xlsx(&f, n, true);
        }
        best("xlsx all 16-19 digit", repeats, || imp(&f, "xlsx"));
    }
    if want("json") {
        let f = dir.join(format!("rows-{rows}.json"));
        if !f.exists() {
            write_json(&f, rows);
        }
        best("json", repeats, || imp(&f, "json"));
    }
    if want("ods") {
        let n = rows / 4;
        let f = dir.join(format!("mixed-{n}.ods"));
        if !f.exists() {
            write_ods(&f, n);
        }
        best("ods", repeats, || imp(&f, "ods"));
    }
    if want("sqlite") {
        let f = dir.join(format!("table-{rows}.sqlite"));
        if !f.exists() {
            write_sqlite(&f, rows);
        }
        best("sqlite", repeats, || {
            natendo_engine::sqlite::read_table(&f, "t", &p)
                .unwrap()
                .row_count()
        });
    }
}
