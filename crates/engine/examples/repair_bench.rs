//! Times what the repair bar does on a large file:
//! `cargo run --release --example repair_bench -- <ragged-1m.csv>` (the file `ragged_bench` writes).
use natendo_engine::document::repair::{Between, Repair};
use natendo_engine::{Document, OpenOptions, Progress};
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let path = std::env::args().nth(1).expect("a ragged CSV file");
    let mut d = Document::open_indexed(path.as_ref(), OpenOptions::default(), None).unwrap();
    let found = d.ragged_rows(true, &Progress::new()).unwrap();
    let (ids, _) = d.ragged_ids(&found, 100_000);
    for round in 0..3 {
        // Rows far apart, the last included.
        let picks = [ids[0], ids[ids.len() / 2], ids[ids.len() - 2]];
        let mut worst = 0f64;
        for &id in &picks {
            let t = Instant::now();
            d.row_text(id, true).unwrap();
            worst = worst.max(ms(t));
        }
        let t = Instant::now();
        d.row_width(500_000).unwrap();
        println!(
            "round {round}: row text worst of 3 {worst:.3} ms; one row's width {:.3} ms",
            ms(t)
        );
    }
    // After a descending sort on the first column (rows far from their own numbers).
    let spec = natendo_engine::sort::SortSpec {
        keys: vec![natendo_engine::sort::SortKey {
            col: 0,
            descending: true,
            kind: Default::default(),
            match_case: false,
        }],
        keep_first: true,
        ..Default::default()
    };
    d.sort(&spec).unwrap();
    let mut worst = 0f64;
    for &id in &[ids[0], ids[ids.len() / 2], ids[ids.len() - 2]] {
        let t = Instant::now();
        d.row_text(id, true).unwrap();
        worst = worst.max(ms(t));
    }
    println!("row text after a descending sort, worst of 3: {worst:.3} ms");
    assert!(d.undo());
    // With rows removed before them (every later row has moved up).
    d.delete_rows(10, 5).unwrap();
    let t = Instant::now();
    let seen = d.row_text(ids[ids.len() / 2], true).unwrap();
    println!("row text after rows were removed: {:.3} ms", ms(t));
    let t = Instant::now();
    d.repair_row(seen.id, seen.revision, &Repair::Join { groups: vec![2] })
        .unwrap();
    println!("join two fields: {:.3} ms", ms(t));
    let revision = d.revision();
    let t = Instant::now();
    d.repair_row(
        ids[ids.len() / 2 + 2],
        revision,
        &Repair::Next {
            between: Between::Space,
        },
    )
    .unwrap();
    println!("join with the next row: {:.3} ms", ms(t));
}
