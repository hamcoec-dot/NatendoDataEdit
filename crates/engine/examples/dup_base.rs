//! The Duplicate tab speed gate's baseline, with APIs main has too, so the same file builds on main and on
//! the branch: `cargo run --release --example dup_base -- <dir> <gen|open> [rows]`
//! - gen: write `big.csv` (made-up, 10 columns) of `rows` rows (1,000,000 by default);
//! - open: open and index it (median of 5), what S1 compares a Duplicate with.

use natendo_engine::{Document, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = Path::new(&args[1]);
    let what = args.get(2).map(String::as_str).unwrap_or("open");
    let file = dir.join("big.csv");
    if what == "gen" {
        let rows: u32 = args
            .get(3)
            .and_then(|s| s.parse().ok())
            .unwrap_or(1_000_000);
        let mut f = std::io::BufWriter::new(std::fs::File::create(&file).unwrap());
        writeln!(f, "id,first,city,zip,phone,party,amount,joined,status,town").unwrap();
        for i in 0..rows {
            writeln!(
                f,
                "{i},N{},City {},{:05},555-{:04},P{},{}.{:02},2026-{:02}-{:02},s{},T{}",
                i % 97,
                i % 300,
                i % 99_999,
                i % 10_000,
                i % 5,
                i % 5000,
                i % 100,
                1 + i % 12,
                1 + i % 28,
                i % 3,
                i % 40
            )
            .unwrap();
        }
        return;
    }
    let mut times = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        let d = Document::open_indexed(&file, OpenOptions::default(), None).unwrap();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        assert!(d.row_count() > 0);
    }
    let mut s = times.clone();
    s.sort_by(f64::total_cmp);
    println!("open {:.1} ms (median of 5; {:.1?})", s[2], times);
}
