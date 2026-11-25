//! Time of the pattern pill's filter over a whole column of a generated file (branch only: main has
//! no `plan_shape_filter`), against two whole-column scans as yardsticks (Find count and matching rows).
//!
//! ```text
//! cargo run --release -p natendo-engine --example shape_bench -- [rows] [repeats]
//! ```
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use natendo_engine::find::{FindSpec, Pattern, PatternKind, Scope};
use natendo_engine::profile::ShapeFilter;
use natendo_engine::{Document, OpenOptions, Progress};

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
    let p = Progress::new();
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
    // Column 5 holds dates (2026-MM-DD): the shape 9999-99-99; column 1 holds "name N".
    for (col, shape) in [(5u32, "9999-99-99"), (1, "AAAA 9999")] {
        for negate in [true, false] {
            let spec = ShapeFilter {
                col,
                keep_first: true,
                shape: shape.into(),
                negate,
                ..Default::default()
            };
            let ms = best(&mut || {
                d.plan_shape_filter(&spec, &p).unwrap();
            });
            println!(
                "shape filter, col {col}, {}, {rows} rows	{ms:.2}",
                if negate {
                    "rows that differ"
                } else {
                    "rows that match"
                }
            );
        }
    }
    let find = FindSpec {
        pattern: Pattern {
            text: "name 1".into(),
            kind: PatternKind::Plain,
            ..Default::default()
        },
        scope: Scope::Columns { cols: vec![1] },
        ..Default::default()
    };
    println!(
        "yardstick: Find count, col 1	{:.2}",
        best(&mut || {
            d.find_count(&find, None, &p).unwrap();
        })
    );
    println!(
        "yardstick: matching rows, col 1	{:.2}",
        best(&mut || {
            d.matching_rows(&find, &p).unwrap();
        })
    );
}
