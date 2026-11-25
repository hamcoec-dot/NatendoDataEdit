//! Where the time of `diff_files` goes, step by step (branch only).
use natendo_engine::diff::DiffSpec;
use natendo_engine::document::Progress;
use natendo_engine::{Document, OpenOptions};
use std::time::Instant;

fn main() {
    let rows: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);
    let dir = std::env::temp_dir().join("nde-diff-bench");
    let a = Document::open_indexed(
        &dir.join(format!("a-{rows}.csv")),
        OpenOptions::default(),
        None,
    )
    .unwrap();
    let b = Document::open_indexed(
        &dir.join(format!("b-{rows}.csv")),
        OpenOptions::default(),
        None,
    )
    .unwrap();
    let spec = DiffSpec {
        keys: vec![0],
        keep_first: true,
        match_case: false,
        trim: true,
    };
    let p = Progress::new();
    for _ in 0..3 {
        let titles = a.diff_titles(&spec);
        let t = Instant::now();
        let index = Document::diff_index(&b, &titles, &spec, &p).unwrap();
        let t1 = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let first = a.diff_first(&index, &spec, &p).unwrap();
        let t2 = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let _ = b.diff_second(index, first, &spec, &p).unwrap();
        let t3 = t.elapsed().as_secs_f64() * 1000.0;
        println!("index {t1:.0} ms, first {t2:.0} ms, second {t3:.0} ms");
    }
}
