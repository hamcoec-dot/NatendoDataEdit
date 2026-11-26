//! Time of the Excel export of every row in parts (split_files), for the speed gate:
//!
//! ```text
//! cargo run --release -p natendo-engine --example xlsx_bench -- [rows] [repeats] [shuffled|plain|session] [folder] [part_rows]
//! ```
//! Prints the best and every time in milliseconds (default 1,000,000 rows, 3 repeats). With `shuffled`
//! the rows are shuffled first, as in the perf run (rows read out of file order).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use natendo_engine::export::{ExportFormat, ExportOptions};
use natendo_engine::expr::ComputeSpec;
use natendo_engine::find::{FindSpec, Pattern, PatternKind};
use natendo_engine::sort::{SortKey, SortKind, SortSpec};
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
    let repeats: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(3);
    // Where the fixture and the output go (default: the temp folder; perf uses target/perf).
    let dir = args.get(3).map_or_else(
        || std::env::temp_dir().join("nde-find-bench"),
        std::path::PathBuf::from,
    );
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("fixture-{rows}.csv"));
    if !path.exists() {
        generate(&path, rows).unwrap();
    }
    let mut d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
    let p = Progress::new();
    if args.get(2).is_some_and(|a| a == "session") {
        // A working session: sort, filter by a formula, Find all, clear the filter, undo,
        // sort again, edit a few cells; then export.
        let key = |col, kind| SortSpec {
            scope: Default::default(),
            keys: vec![SortKey {
                col,
                descending: false,
                kind,
                match_case: false,
            }],
            keep_first: true,
            read: Default::default(),
            lists: Vec::new(),
        };
        let plan = d.plan_sort(&key(2, SortKind::Number), &p).unwrap();
        d.apply_sort(plan).unwrap();
        let (plan, _) = d
            .plan_formula_filter(
                &ComputeSpec {
                    expr: "AND(VALUE([col2]) > 500, LEN([col4]) = 8)".into(),
                    keep_first: true,
                    ..Default::default()
                },
                &p,
            )
            .unwrap();
        d.apply_rows(plan).unwrap();
        let find = FindSpec {
            pattern: Pattern {
                text: "City 7".into(),
                kind: PatternKind::Plain,
                ..Default::default()
            },
            ..Default::default()
        };
        d.find_all(&find, 1_000, &p).unwrap();
        d.clear_filter();
        d.undo();
        let plan = d.plan_sort(&key(1, SortKind::Auto), &p).unwrap();
        d.apply_sort(plan).unwrap();
        d.clear_filter();
        let shown = d.row_count();
        for i in 0..200u64 {
            d.set_cell(
                1 + (i * (rows / 250).max(1)) % (shown - 1),
                3,
                format!("edited {i}"),
            )
            .unwrap();
        }
        println!(
            "(session done: {} rows shown, {} undo steps)",
            d.row_count(),
            d.can_undo() as u8
        );
    } else if args.get(2).is_some_and(|a| a == "shuffled") {
        let plan = d.plan_shuffle(1, rows - 1, 7, &p).unwrap();
        d.apply_shuffle(plan).unwrap();
    }
    // Data rows per part (0: as many as a sheet holds, so 1,048,575 and files only past that).
    let part_rows: u64 = args.get(4).and_then(|a| a.parse().ok()).unwrap_or(0);
    let out = dir.join("xlsx-bench-parts");
    let mut times = Vec::new();
    for _ in 0..repeats {
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();
        let t = Instant::now();
        let r = d
            .export_file(
                &out.join("big.xlsx"),
                &ExportOptions {
                    format: ExportFormat::Xlsx,
                    split_files: true,
                    part_rows,
                    ..Default::default()
                },
                &p,
            )
            .unwrap();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        println!(
            "{rows} rows: {ms:.0} ms ({} rows, {} files, {:.0} MB)",
            r.rows,
            r.files.len(),
            r.bytes as f64 / 1e6
        );
        times.push(ms);
    }
    let _ = std::fs::remove_dir_all(&out);
    println!(
        "best {:.0} ms",
        times.iter().cloned().fold(f64::MAX, f64::min)
    );
}
