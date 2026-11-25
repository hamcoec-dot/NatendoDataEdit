//! Times for Checkpoints (B4), for the speed gate:
//!
//! ```text
//! cargo run --release -p natendo-engine --example checkpoint_bench -- [rows] [repeats] [folder]
//! ```
//! Making a mark (a checkpoint) is O(1); going back across 1,000 steps on a 1M-row file (target under 500 ms) and
//! forward again; and what the extra bookkeeping costs a plain cell edit and a sort-free bulk step, each the best of
//! `repeats`.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use natendo_engine::{Document, OpenOptions};

fn generate(path: &std::path::Path, rows: u64) -> std::io::Result<()> {
    let mut w = BufWriter::with_capacity(1 << 20, File::create(path)?);
    writeln!(w, "id,name,amount,city\r")?;
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in 0..rows {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        write!(
            w,
            "{i},name {},{}.{:02},City {}\r\n",
            x % 9973,
            x % 100_000,
            x % 100,
            x % 311
        )?;
    }
    w.flush()
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
    let all: Vec<String> = times.iter().map(|t| format!("{t:.2}")).collect();
    println!("{name}: best {b:.2} ms (result {n}) [{}]", all.join(", "));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rows: u64 = args
        .first()
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);
    let repeats: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(5);
    let dir = args.get(2).map_or_else(
        || std::env::temp_dir().join("nde-checkpoint-bench"),
        std::path::PathBuf::from,
    );
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("cp-{rows}.csv"));
    if !path.exists() {
        generate(&path, rows).unwrap();
    }
    let mut d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
    // Making a mark: 1,000,000 of them.
    best("1,000,000 marks (step_mark)", repeats, || {
        let mut s = 0u64;
        for _ in 0..1_000_000 {
            s = s.wrapping_add(d.step_mark());
        }
        s & 1
    });
    let start = d.step_mark();
    let mut x: u64 = 12345;
    let t = Instant::now();
    for i in 0..1_000u64 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        d.set_cell(1 + x % (rows - 1), (i % 4) as u32, format!("e{i}"))
            .unwrap();
    }
    println!(
        "1,000 cell edits: {:.1} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );
    let end = d.step_mark();
    best("go back across 1,000 steps (go_to_mark)", repeats, || {
        assert!(d.go_to_mark(start));
        u64::from(d.go_to_mark(end))
    });
    best("reach query (step_reach)", repeats, || {
        let mut n = 0;
        for _ in 0..100_000 {
            n += d.step_reach(start).is_some() as u64;
        }
        n
    });
}
