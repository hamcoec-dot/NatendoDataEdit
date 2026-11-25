//! Times opening a large file and finding its ragged rows:
//! `cargo run --release --example ragged_bench -- <work folder>`. Writes
//! 1,000,000 rows of 14 columns, one row in 100 short or long.
use natendo_engine::{Document, OpenOptions, Progress};
use std::io::Write;
use std::time::Instant;

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("a work folder"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ragged-1m.csv");
    if !path.exists() {
        let mut w = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let head: Vec<String> = (0..14).map(|c| format!("c{c}")).collect();
        writeln!(w, "{}", head.join(",")).unwrap();
        for r in 0..1_000_000u32 {
            let n = match r % 200 {
                7 => 13,
                107 => 15,
                _ => 14,
            };
            let row: Vec<String> = (0..n).map(|c| format!("v{}", (r + c) % 9973)).collect();
            writeln!(w, "{}", row.join(",")).unwrap();
        }
    }
    for round in 0..5 {
        let t = Instant::now();
        let d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        let open = t.elapsed();
        let t = Instant::now();
        let counted = d.ragged_at_open(false).unwrap();
        let count = t.elapsed();
        let t = Instant::now();
        let r = d.ragged_rows(true, &Progress::new()).unwrap();
        let job = t.elapsed();
        println!(
            "round {round}: open and index {:.1} ms; count at open {:.3} ms ({} rows, usual {}); job {:.1} ms ({} short, {} long)",
            open.as_secs_f64() * 1000.0,
            count.as_secs_f64() * 1000.0,
            counted.1,
            counted.0,
            job.as_secs_f64() * 1000.0,
            r.short,
            r.long
        );
    }
}
