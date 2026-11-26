//! Times reading a 100,000-row `.ods` sheet with number styles:
//! `cargo run --release --example ods_bench -- <file.ods>` (the best of 7).
use natendo_engine::{import, Progress};
use std::time::Instant;

fn main() {
    let p = std::env::args().nth(1).expect("an .ods file");
    let data = std::fs::read(p).unwrap();
    let opts = import::ImportOptions {
        pad_zeros: Some(true),
        ..Default::default()
    };
    let mut best = f64::MAX;
    let mut rows = 0;
    for _ in 0..7 {
        let t = Instant::now();
        let table = import::import(&data, Some("ods"), &opts, None, &Progress::new()).unwrap();
        best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        rows = table.rows;
    }
    println!("ods import best of 7: {best:.1} ms, {rows} rows");
}
