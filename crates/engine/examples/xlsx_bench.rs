//! Time of the Excel export of every row in parts (split_files), for the speed gate:
//!
//! ```text
//! cargo run --release -p natendo-engine --example xlsx_bench -- [rows] [repeats] [shuffled]
//! ```
//! Prints the best and every time in milliseconds (default 1,000,000 rows, 3 repeats). With `shuffled`
//! the rows are shuffled first, as in the perf run (rows read out of file order).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use natendo_engine::export::{ExportFormat, ExportOptions};
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
    let dir = std::env::temp_dir().join("nde-find-bench");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("fixture-{rows}.csv"));
    if !path.exists() {
        generate(&path, rows).unwrap();
    }
    let mut d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
    let p = Progress::new();
    if args.get(2).is_some_and(|a| a == "shuffled") {
        let plan = d.plan_shuffle(1, rows - 1, 7, &p).unwrap();
        d.apply_shuffle(plan).unwrap();
    }
    let out = dir.join("xlsx-parts");
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
