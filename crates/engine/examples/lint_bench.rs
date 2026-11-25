//! Time of the data checks on a generated file, for comparing two builds (a branch against main):
//! the same source runs on both because it uses only `run_checks` and `check_marks` and builds the
//! rules from JSON.
//!
//! ```text
//! cargo run --release -p natendo-engine --example lint_bench -- [rows] [repeats]
//! ```
//! Prints `name<TAB>best milliseconds` per measurement, the best of `repeats` runs after a warm-up
//! (default 1,000,000 rows, 7 repeats).

use natendo_engine::document::Progress;
use natendo_engine::validate::CheckSpec;
use natendo_engine::{Document, OpenOptions};

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

const COLS: usize = 20;

fn generate(path: &std::path::Path, rows: u64) -> std::io::Result<()> {
    let mut w = BufWriter::with_capacity(1 << 20, File::create(path)?);
    let header: Vec<String> = (0..COLS).map(|c| format!("col{c}")).collect();
    writeln!(w, "{}\r", header.join(","))?;
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in 0..rows {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        write!(
            w,
            "{i},name {},{}.{:02},\"note, {}\",City {},2026-{:02}-{:02},{}",
            x % 9973,
            x % 100_000,
            x % 100,
            x % 1000,
            x % 311,
            x % 12 + 1,
            x % 28 + 1,
            x % 2 == 0
        )?;
        for c in 8..COLS {
            write!(w, ",v{}", (x >> (c * 2)) % 1_000_000)?;
        }
        w.write_all(b"\r\n")?;
    }
    w.flush()
}

const RULES: &str = r#"{"keep_first":true,"checks":[
 {"col":1,"kind":{"type":"required"}},
 {"col":2,"kind":{"type":"number","min":0,"max":90000,"whole":false}},
 {"col":4,"kind":{"type":"pattern","pattern":{"text":"City [0-9]{1,2}","kind":"regex"}}},
 {"col":5,"kind":{"type":"length","min":10,"max":10}},
 {"col":0,"kind":{"type":"unique","match_case":false}},
 {"kind":{"type":"expression","expr":"LEN([col1]) > 8"}}]}"#;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rows: u64 = args.first().and_then(|a| a.parse().ok()).unwrap_or(1_000_000);
    let repeats: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(7);
    let dir = std::env::temp_dir().join("nde-find-bench");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("fixture-{rows}.csv"));
    if !path.exists() {
        generate(&path, rows).unwrap();
    }
    let d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
    let spec: CheckSpec = serde_json::from_str(RULES).unwrap();
    let best = |f: &mut dyn FnMut()| {
        f();
        let mut best = f64::MAX;
        for _ in 0..repeats {
            let t = Instant::now();
            f();
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        best
    };
    let p = Progress::new();
    let mut rep = None;
    let ms = best(&mut || {
        rep = Some(d.run_checks(&spec, &p).unwrap());
    });
    let rep = rep.unwrap();
    println!("run_checks, 6 rules, {rows} rows ({} problems)	{ms:.2}", rep.problems);
    // The screens a person scrolls through: 60 rows x 20 columns at 200 places down the table.
    let ms = best(&mut || {
        for k in 0..200u64 {
            let at = 1 + k * (rows.saturating_sub(100) / 200);
            d.check_marks(at, 60, 0, COLS as u32, &spec, &rep.repeats).unwrap();
        }
    });
    println!("check_marks, 200 screens of 60 x 20	{ms:.2}");
}
