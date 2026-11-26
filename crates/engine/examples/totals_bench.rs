//! Times the totals row against the selection totals of the same cells:
//! `cargo run --release --example totals_bench -- <file> <columns>`

use natendo_engine::value::ReadOptions;
use natendo_engine::{Document, OpenOptions, Progress};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n: u32 = args[1].parse().expect("columns");
    let d = Document::open_indexed(std::path::Path::new(&args[0]), OpenOptions::default(), None)
        .expect("open");
    let cols: Vec<u32> = (0..n.min(d.col_count())).collect();
    let rows = d.row_count() - 1;
    let (mut selection, mut row) = (Vec::new(), Vec::new());
    for _ in 0..9 {
        let t = Instant::now();
        let a = d
            .selection_totals(1, rows, 0, n, ReadOptions::default(), &Progress::new())
            .expect("selection");
        selection.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        let b = d
            .column_totals(&cols, true, ReadOptions::default(), &Progress::new())
            .expect("totals");
        row.push(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(a.numbers, b.iter().map(|c| c.totals.numbers).sum::<u64>());
    }
    selection.sort_by(f64::total_cmp);
    row.sort_by(f64::total_cmp);
    println!(
        "{rows} rows x {} columns, medians of 9: selection totals {:.0} ms, totals row {:.0} ms, ratio {:.3}",
        cols.len(),
        selection[4],
        row[4],
        row[4] / selection[4]
    );
}
