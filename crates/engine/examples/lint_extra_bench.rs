//! Branch-only timings for A2: jump to next problem, and rule suggestions.
//!
//! ```text
//! cargo run --release -p natendo-engine --example lint_extra_bench -- [rows] [repeats]
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
 {"kind":{"type":"expression","expr":"[col0] <> 50000"}}]}"#;

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
    let full = best(&mut || {
        d.run_checks(&spec, &p).unwrap();
    });
    println!("run_checks, one sparse row rule (full run)	{full:.2}");
    let ms = best(&mut || {
        let r = d.next_problem(1, 0, false, &spec, &[], &[], &p).unwrap();
        assert_eq!(r.hit.map(|j| j.row), Some(50_001));
    });
    println!("next_problem, hit 50,000 rows away	{ms:.2}");
    let ms = best(&mut || {
        let r = d
            .next_problem(50_002, 0, false, &spec, &[], &[], &p)
            .unwrap();
        assert!(r.wrapped && r.hit.is_some());
    });
    println!("next_problem, miss to the end and wrap round 1M rows	{ms:.2}");
    let ms = best(&mut || {
        let r = d
            .next_problem(50_000, 0, true, &spec, &[], &[], &p)
            .unwrap();
        assert!(r.hit.is_some());
    });
    println!("next_problem backward, wraps	{ms:.2}");
    for (name, col) in [
        ("suggest_rules, 1 number column (col 0)", 0u32),
        ("suggest_rules, 1 name column (col 1)", 1),
        ("suggest_rules, 1 date column (col 5)", 5),
    ] {
        let ms = best(&mut || {
            d.suggest_rules(col, true, &p).unwrap();
        });
        println!("{name}, 200,000-row sample	{ms:.2}");
    }
}
