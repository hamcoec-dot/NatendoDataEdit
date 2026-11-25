//! Open time of a big file, to compare this build with main side by side (B2 touches the open path's options only):
//!
//! ```text
//! cargo run --release -p natendo-engine --example open_bench -- <file> [repeats]
//! ```
//! `Document::open_indexed` on the file, best of `repeats`.

use std::time::Instant;

use natendo_engine::{Document, OpenOptions};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = std::path::PathBuf::from(args.first().expect("a file to open"));
    let repeats: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(5);
    let mut best = f64::MAX;
    let mut rows = 0;
    for _ in 0..repeats {
        let t = Instant::now();
        let d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        rows = d.row_count();
    }
    println!("open_indexed: best {best:.0} ms ({rows} rows)");
}
