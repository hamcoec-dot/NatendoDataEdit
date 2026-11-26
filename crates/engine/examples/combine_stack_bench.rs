//! `combine` of one made-up file stacked N times (by title), median of 5:
//! `combine_stack_bench <file> [N]`. The same file builds on main, so the two can be
//! run side by side.

use std::path::Path;
use std::time::Instant;

use natendo_engine::tools::{combine, Combine};
use natendo_engine::{Document, OpenOptions, Progress};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(12);
    let d = Document::open_indexed(Path::new(&a[0]), OpenOptions::default(), None).unwrap();
    let tables: Vec<(String, &Document)> = (0..n).map(|i| (format!("f{i}.csv"), &d)).collect();
    let spec = Combine {
        header: true,
        by_title: true,
        source: true,
    };
    let mut ms = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        let table = combine(&tables, &spec, &Progress::new()).unwrap();
        ms.push(t.elapsed().as_millis());
        drop(table);
    }
    ms.sort();
    println!("{}", ms[2]);
}
