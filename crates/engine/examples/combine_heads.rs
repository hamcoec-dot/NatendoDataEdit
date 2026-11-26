//! The column map's heads (first rows only) of N files, median of 5:
//! `combine_heads <file> [N]` prints milliseconds for all N.

use std::path::Path;
use std::time::Instant;

use natendo_engine::combine::{auto_plan, head_of, suggest};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(12);
    let mut ms = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        let heads: Vec<_> = (0..n)
            .map(|_| head_of(Path::new(&a[0]), true).unwrap())
            .collect();
        let rows: Vec<Vec<String>> = heads.iter().map(|h| h.titles.clone()).collect();
        let plan = auto_plan(&rows, true);
        let _ = suggest(&plan, &heads);
        ms.push(t.elapsed().as_millis());
    }
    ms.sort();
    println!("{}", ms[2]);
}
