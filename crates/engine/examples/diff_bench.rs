//! Time (and, run alone, peak memory) of comparing two generated tables by key, for comparing two builds
//! (a branch against main): the same source runs on both because it uses only what both have, the
//! newer-file engine (`NewerIndex`, `pair_newer`, `NewerValues::fetch`, `diff_newer`) and today's Compare
//! (`CompareIndex`, `compare_rows`, `compare_finish`).
//!
//! ```text
//! cargo run --release -p natendo-engine --example diff_bench -- [rows] [repeats] [newer|compare|both]
//! ```
//! Prints `name<TAB>best milliseconds`; with one mode only, the process is a clean run for measuring its
//! peak memory from outside.

use natendo_engine::document::Progress;
use natendo_engine::tools::{CompareIndex, CompareSpec};
use natendo_engine::updates::{NewerIndex, NewerValues, UpdateSpec};
use natendo_engine::{Document, OpenOptions};

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

fn generate(a: &std::path::Path, b: &std::path::Path, rows: u64) -> std::io::Result<()> {
    let mut wa = BufWriter::with_capacity(1 << 20, File::create(a)?);
    let mut wb = BufWriter::with_capacity(1 << 20, File::create(b)?);
    writeln!(wa, "id,name,city,amount,note\r")?;
    // The second table: another column order and one more column, every 10th row changed, every 997th
    // row missing and a few rows only here.
    writeln!(wb, "note,city,id,amount,name,zip\r")?;
    for i in 0..rows {
        writeln!(
            wa,
            "{i},name {},City {},{}.{:02},note {}\r",
            i % 9973,
            i % 311,
            i % 100_000,
            i % 100,
            i % 1000
        )?;
        if i % 997 == 0 {
            continue;
        }
        let changed = i % 10 == 0;
        writeln!(
            wb,
            "note {},City {},{i},{}.{:02},name {},{}\r",
            i % 1000,
            if changed { (i + 1) % 311 } else { i % 311 },
            i % 100_000,
            i % 100,
            i % 9973,
            10_000 + i % 90_000
        )?;
    }
    for i in 0..(rows / 5000) {
        writeln!(wb, "note x,City x,{},1.00,name x,1\r", rows + i)?;
    }
    wa.flush()?;
    wb.flush()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rows: u64 = args
        .first()
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);
    let repeats: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(5);
    let mode = args.get(2).map(String::as_str).unwrap_or("both");
    let dir = std::env::temp_dir().join("nde-diff-bench");
    std::fs::create_dir_all(&dir).unwrap();
    let (pa, pb) = (
        dir.join(format!("a-{rows}.csv")),
        dir.join(format!("b-{rows}.csv")),
    );
    if !pa.exists() || !pb.exists() {
        generate(&pa, &pb, rows).unwrap();
    }
    let a = Document::open_indexed(&pa, OpenOptions::default(), None).unwrap();
    let b = Document::open_indexed(&pb, OpenOptions::default(), None).unwrap();
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
    if mode == "newer" || mode == "both" {
        let all: Vec<u32> = (0..a.col_count()).collect();
        let titles = a.titles(&all, true);
        let us = UpdateSpec {
            keys: vec![0],
            keep_first: true,
            match_case: false,
            trim: true,
        };
        let mut cells = 0;
        let ms = best(&mut || {
            let index = NewerIndex::build(&b, &titles, &us, &p).unwrap();
            let pairing = a.pair_newer(&index, &us, &p).unwrap();
            let values = NewerValues::fetch(&b, &index, &pairing, &p).unwrap();
            let (_, list) = a.diff_newer(&index, &pairing, values, &p).unwrap();
            cells = list.cells_total;
        });
        println!("newer-file diff by key (index, pair, values, list), {rows} rows ({cells} changed cells)\t{ms:.1}");
    }
    if mode == "compare" || mode == "both" {
        let spec = CompareSpec {
            keys: vec![0],
            match_case: false,
            trim: true,
            keep_first: true,
            show_same: false,
        };
        // Today's Compare matches columns by position: the second table is the same file here.
        let ms = best(&mut || {
            let index = CompareIndex::build(&a, &spec, &p).unwrap();
            let plan = a.compare_rows(&index, &spec, &p).unwrap();
            let _ = a.compare_finish(&index, plan, &spec, &p).unwrap();
        });
        println!("today's Compare by key (table with itself), {rows} rows\t{ms:.1}");
    }
}
