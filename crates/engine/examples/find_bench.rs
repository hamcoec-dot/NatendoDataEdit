//! Find timings on a generated file, for comparing two builds (a branch against main):
//! the same source runs on both because it uses only the Range, All and Columns scopes.
//!
//! ```text
//! cargo run --release -p natendo-engine --example find_bench -- [rows] [repeats]
//! ```
//! Prints one line per measurement, `name<TAB>best milliseconds`, the best of `repeats` runs
//! after one warm-up run (default 1,000,000 rows, 7 repeats).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use natendo_engine::find::{FindSpec, Pattern, PatternKind, Scope};
use natendo_engine::reshape::Duplicates;
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

fn spec(text: &str, scope: Scope) -> FindSpec {
    FindSpec {
        pattern: Pattern {
            text: text.into(),
            kind: PatternKind::Plain,
            ..Default::default()
        },
        scope,
        ..Default::default()
    }
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
    let n = rows - 1;
    let whole = spec("City 7", Scope::All);
    let nomatch = spec("zzzz", Scope::All);
    let block = spec(
        "City 7",
        Scope::Range {
            row: 1,
            rows: n,
            col: 4,
            cols: 3,
        },
    );
    let column = spec("City 7", Scope::Columns { cols: vec![4] });
    let line = |name: &str, ms: f64| println!("{name}\t{ms:.2}");
    line(
        "find next, whole file, first match",
        best(&mut || {
            d.find_next(&whole, None, false, &p).unwrap();
        }),
    );
    line(
        "find next, whole file, no match",
        best(&mut || {
            d.find_next(&nomatch, None, false, &p).unwrap();
        }),
    );
    line(
        "find count, whole file",
        best(&mut || {
            d.find_count(&whole, None, &p).unwrap();
        }),
    );
    line(
        "find all (1,000 listed), whole file",
        best(&mut || {
            d.find_all(&whole, 1_000, &p).unwrap();
        }),
    );
    line(
        "find count, one block (Range, 3 columns)",
        best(&mut || {
            d.find_count(&block, None, &p).unwrap();
        }),
    );
    line(
        "find all (1,000 listed), one block (Range)",
        best(&mut || {
            d.find_all(&block, 1_000, &p).unwrap();
        }),
    );
    line(
        "find all (1,000 listed), one column",
        best(&mut || {
            d.find_all(&column, 1_000, &p).unwrap();
        }),
    );
    line(
        "find next, one block (Range), no match",
        best(&mut || {
            let s = spec(
                "zzzz",
                Scope::Range {
                    row: 1,
                    rows: n,
                    col: 4,
                    cols: 3,
                },
            );
            d.find_next(&s, None, false, &p).unwrap();
        }),
    );
    line(
        "matching rows, whole file",
        best(&mut || {
            d.matching_rows(&whole, &p).unwrap();
        }),
    );
    let dups = Duplicates {
        cols: vec![1],
        match_case: false,
        trim: true,
        keep_first: true,
        ..Default::default()
    };
    line(
        "show duplicates, one column",
        best(&mut || {
            d.plan_show_duplicates(&dups, &p).unwrap();
        }),
    );
}
