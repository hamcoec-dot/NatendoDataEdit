//! Times moving rows on a 1,000,000-row table (gap #5): a plain move of one
//! row; with a filter showing about half the rows, a move of one shown row (one
//! run), the first move of 1,000 shown rows with hidden rows between them (a
//! new order), and Undo and Redo of that move. Builds where the filtered moves
//! are refused print "refused".
//! `cargo run --release --example move_filtered_bench -- [runs]`

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
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1000.0;
    // Plain: one row down one place and back (two moves per sample).
    let plain = median(
        (0..runs)
            .map(|i| {
                let at = 1000 + i as u64 * 7;
                let t = Instant::now();
                d.move_rows(at, 1, at + 1).unwrap();
                d.move_rows(at + 1, 1, at).unwrap();
                ms(t) / 2.0
            })
            .collect(),
    );
    println!("plain move, 1 row\t{plain:.3} ms");
    let spec: FilterSpec = serde_json::from_str(
        r#"{"columns":[{"col":1,"rule":{"type":"values","values":["a"],"blanks":false}}],"keep_first":true}"#,
    )
    .unwrap();
    d.set_filter(&spec).unwrap();
    println!("shown rows: {} of {}", d.row_count(), d.total_rows());
    if d.move_rows(10, 1, 11).is_err() {
        println!("filtered moves\trefused");
        return;
    }
    d.undo();
    let one = median(
        (0..runs)
            .map(|i| {
                let at = 1000 + i as u64 * 7;
                let t = Instant::now();
                d.move_rows(at, 1, at + 1).unwrap();
                d.move_rows(at + 1, 1, at).unwrap();
                ms(t) / 2.0
            })
            .collect(),
    );
    println!("filtered move, 1 shown row\t{one:.3} ms");
    let (mut first, mut undo, mut redo) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..runs {
        let at = 5000 + i as u64 * 3000;
        let t = Instant::now();
        d.move_rows(at, 1000, 400_000).unwrap();
        let _ = d.doc_rows(0, 1);
        first.push(ms(t));
        // Each with the next look at the shown rows (the list of shown rows
        // is made again after a new order).
        let t = Instant::now();
        d.undo();
        let _ = d.doc_rows(0, 1);
        undo.push(ms(t));
        let t = Instant::now();
        d.redo();
        let _ = d.doc_rows(0, 1);
        redo.push(ms(t));
        d.undo();
    }
    println!(
        "filtered first move, 1,000 shown rows over gaps (new order)\t{:.2} ms",
        median(first)
    );
    println!("  its Undo\t{:.2} ms", median(undo));
    println!("  its Redo\t{:.2} ms", median(redo));
}
