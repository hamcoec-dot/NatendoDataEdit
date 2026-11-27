//! Times of the engine's number-reading hot paths (A1b), for the speed gate against main. It uses only APIs both
//! sides have, so the same file builds on main and on the branch:
//!
//! ```text
//! cargo run --release -p natendo-engine --example hot_bench -- [rows] [repeats] [folder] [typed]
//! ```
//! Sort by a number column (Auto), by a text column, a number filter, the column summary and the quick totals of
//! 1M cells, each the best of `repeats` (every run on a fresh open, so a sorted or filtered table does not
//! change the next run). With `typed` (branch only: the file `typed_bench.rs` does that part) nothing differs.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use natendo_engine::filter::{CmpOp, ColumnFilter, Condition, FilterSpec, Rule};
use natendo_engine::sort::{SortKey, SortKind, SortSpec};
use natendo_engine::{Document, OpenOptions, Progress};

fn generate(path: &std::path::Path, rows: u64) -> std::io::Result<()> {
    let mut w = BufWriter::with_capacity(1 << 20, File::create(path)?);
    writeln!(w, "id,name,amount,city,joined,flag\r")?;
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in 0..rows {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        write!(
            w,
            "{i},name {},{}.{:02},City {},2026-{:02}-{:02},{}\r\n",
            x % 9973,
            x % 100_000,
            x % 100,
            x % 311,
            x % 12 + 1,
            x % 28 + 1,
            x % 2 == 0
        )?;
    }
    w.flush()
}

fn by(col: u32, kind: SortKind) -> SortSpec {
    SortSpec {
        keys: vec![SortKey {
            col,
            descending: false,
            kind,
            match_case: false,
        }],
        keep_first: true,
        ..Default::default()
    }
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
    println!("{name}: best {b:.0} ms (result {n}) [{}]", all.join(", "));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rows: u64 = args
        .first()
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);
    let repeats: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(3);
    let dir = args.get(2).map_or_else(
        || std::env::temp_dir().join("nde-hot-bench"),
        std::path::PathBuf::from,
    );
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("hot-{rows}.csv"));
    if !path.exists() {
        generate(&path, rows).unwrap();
    }
    let open = || Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
    let p = Progress::new();
    best("sort by number (Auto)", repeats, || {
        let mut d = open();
        d.sort(&by(2, SortKind::Auto)).unwrap();
        d.row_count()
    });
    best("sort by text (Auto)", repeats, || {
        let mut d = open();
        d.sort(&by(1, SortKind::Auto)).unwrap();
        d.row_count()
    });
    best("filter amount > 50000", repeats, || {
        let mut d = open();
        d.set_filter(&FilterSpec {
            combine: Default::default(),
            invert: false,
            columns: vec![ColumnFilter {
                enabled: true,
                negate: false,
                name: None,
                col: 2,
                rule: Rule::Conditions {
                    first: Condition::Number {
                        op: CmpOp::Gt,
                        value: 50_000.0,
                    },
                    second: None,
                    or: false,
                },
            }],
            keep_first: true,
            read: Default::default(),
        })
        .unwrap()
    });
    let d = open();
    best("column summary (amount)", repeats, || {
        d.summary(2, true, Default::default(), &p).unwrap().rows
    });
    best("quick totals of 1M cells", repeats, || {
        d.selection_totals(
            1,
            1_000_000.min(rows.saturating_sub(1)),
            2,
            1,
            Default::default(),
            &p,
        )
        .map(|t| t.numbers)
        .unwrap_or(0)
    });
}
