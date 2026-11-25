//! Branch-only timings of the ghost diff: the window of cells on screen, long cells, and the count.
//!
//! ```text
//! cargo run --release -p natendo-engine --example ghost_bench -- [rows] [repeats]
//! ```
//! Prints `name<TAB>best milliseconds` per measurement, the best of `repeats` runs after a warm-up
//! (default 1,000,000 rows, 7 repeats).

use natendo_engine::document::Progress;
use natendo_engine::find::{FindSpec, Pattern, PatternKind};
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rows: u64 = args
        .first()
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);
    let repeats: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(7);
    let dir = std::env::temp_dir().join("nde-find-bench");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("fixture-{rows}.csv"));
    if !path.exists() {
        generate(&path, rows).unwrap();
    }
    let d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
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
    let spec = |text: &str, kind: PatternKind| FindSpec {
        pattern: Pattern {
            text: text.into(),
            kind,
            ..Default::default()
        },
        ..Default::default()
    };
    let p = Progress::new();
    let plain = spec("name", PatternKind::Plain);
    let rx = spec("(v)([0-9])", PatternKind::Regex);
    for (label, s, rep) in [("plain", &plain, "Nom"), ("regex with groups", &rx, "$2$1")] {
        let ms = best(&mut || {
            d.replace_ghosts(s, rep, 500_000, 60, 0, 20).unwrap();
        });
        println!(
            "replace_ghosts, {label}, 60 rows x 20 columns in the middle of {rows} rows	{ms:.3}"
        );
        let ms = best(&mut || {
            d.replace_count(s, rep, &p).unwrap();
        });
        println!("replace_count, {label}, {rows} rows	{ms:.2}");
    }
    // The worst window for a long-cell file: 1,200 cells of about 100 KB each (60 rows x 20 columns).
    let long = "ab ".repeat(33_000);
    let mut csv = String::new();
    for _ in 0..60 {
        csv.push_str(&vec![long.as_str(); 20].join(","));
        csv.push_str(
            "
",
        );
    }
    let l = Document::from_bytes(csv.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
    let evil = spec("(a)b", PatternKind::Regex);
    let mut partial = false;
    let ms = best(&mut || {
        partial = l
            .replace_ghosts(&evil, "$1-", 0, 60, 0, 20)
            .unwrap()
            .partial;
    });
    println!("replace_ghosts, 1,200 cells of 100 KB with a regex (stops at its time cap: partial = {partial})	{ms:.2}");
}
