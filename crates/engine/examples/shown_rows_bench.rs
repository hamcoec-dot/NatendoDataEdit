//! Times `Document::shown_rows_at` for 100,000 custom-height rows on a
//! 1,000,000-row table with a filter showing about half the rows, and
//! `doc_rows` for 100,000 shown rows (gap #4: row heights while filtered).
//! `cargo run --release --example shown_rows_bench -- [runs]`

use natendo_engine::filter::FilterSpec;
use natendo_engine::{Document, OpenOptions};
use std::time::Instant;

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn main() {
    let runs: usize = std::env::args()
        .nth(1)
        .map_or(15, |s| s.parse().expect("runs"));
    let mut s = String::from("n,k\n");
    for r in 0..1_000_000u32 {
        // Made up: "a" on rows whose number mixes to even.
        let k = if r.wrapping_mul(2_654_435_761) >> 31 == 0 {
            "a"
        } else {
            "b"
        };
        s.push_str(&format!("{r},{k}\n"));
    }
    let mut d = Document::from_bytes(s.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
    let spec: FilterSpec = serde_json::from_str(
        r#"{"columns":[{"col":1,"rule":{"type":"values","values":["a"],"blanks":false}}],"keep_first":true}"#,
    )
    .unwrap();
    d.set_filter(&spec).unwrap();
    println!("shown rows: {} of {}", d.row_count(), d.total_rows());
    // 100,000 custom rows spread over the file, sorted (as the heights keep them).
    let places: Vec<u64> = (0..100_000u64).map(|i| i * 10 + 3).collect();
    let time = |f: &mut dyn FnMut()| {
        median(
            (0..runs)
                .map(|_| {
                    let t = Instant::now();
                    f();
                    t.elapsed().as_secs_f64() * 1000.0
                })
                .collect(),
        )
    };
    let mut shown = 0;
    let a = time(&mut || shown = d.shown_rows_at(&places).iter().flatten().count());
    println!("shown_rows_at, 100,000 places\t{a:.2} ms\t({shown} shown)");
    let mut n = 0;
    let b = time(&mut || n = d.doc_rows(1, 100_000).len());
    println!("doc_rows, 100,000 shown rows\t{b:.2} ms\t({n})");
}
