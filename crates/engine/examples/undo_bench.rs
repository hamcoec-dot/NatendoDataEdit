//! Plain edit/undo/redo cost, to compare this build with main side by side (B4 adds an id per undo step):
//!
//! ```text
//! cargo run --release -p natendo-engine --example undo_bench -- [rows] [repeats]
//! ```
//! 20,000 cell edits on a 1M-row file, then every one undone and redone, each the best of `repeats`.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use natendo_engine::{Document, OpenOptions};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rows: u64 = args
        .first()
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);
    let repeats: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(7);
    let dir = std::env::temp_dir().join("nde-undo-bench");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("u-{rows}.csv"));
    if !path.exists() {
        let mut w = BufWriter::with_capacity(1 << 20, File::create(&path).unwrap());
        writeln!(w, "id,name,amount,city\r").unwrap();
        for i in 0..rows {
            write!(w, "{i},name {},{},City {}\r\n", i % 9973, i % 1000, i % 311).unwrap();
        }
        w.flush().unwrap();
    }
    let mut d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
    let mut best = [f64::MAX; 3];
    for _ in 0..repeats {
        let mut x: u64 = 777;
        let t = Instant::now();
        for i in 0..20_000u64 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            d.set_cell(1 + x % (rows - 1), (i % 4) as u32, format!("e{i}"))
                .unwrap();
        }
        let a = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        while d.undo() {}
        let b = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        while d.redo() {}
        let c = t.elapsed().as_secs_f64() * 1000.0;
        while d.undo() {}
        best[0] = best[0].min(a);
        best[1] = best[1].min(b);
        best[2] = best[2].min(c);
    }
    println!(
        "20,000 edits: best {:.1} ms; undo all: best {:.1} ms; redo all: best {:.1} ms",
        best[0], best[1], best[2]
    );
}
