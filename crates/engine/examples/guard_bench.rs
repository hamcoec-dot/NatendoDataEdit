//! Times a share-safe copy and a split of a large file with the CSV formula
//! guard off and on: `cargo run --release --example guard_bench -- <file.csv>`.
use natendo_engine::privacy::{ColumnAction, SafeCopy};
use natendo_engine::tools::SplitParts;
use natendo_engine::{Document, OpenOptions, Progress};
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("a CSV file");
    let d = Document::open_indexed(path.as_ref(), OpenOptions::default(), None).unwrap();
    let p = Progress::new();
    for round in 0..3 {
        for guard in [false, true] {
            let spec = SafeCopy {
                columns: vec![ColumnAction::Keep; d.col_count() as usize],
                header: true,
                read: Default::default(),
                guard,
            };
            let t = Instant::now();
            let r = d
                .write_safe_copy(&mut std::io::sink(), &spec, 0.0, &p)
                .unwrap();
            println!(
                "round {round} safe copy guard={guard}: {} ms, {} rows, {} cells guarded",
                t.elapsed().as_millis(),
                r.rows,
                r.guarded
            );
            let split = SplitParts {
                col: None,
                every: 1_000_000,
                keep_first: true,
                guard,
            };
            let t = Instant::now();
            let mut guarded = 0;
            for part in d.split_parts(&split, &p).unwrap() {
                guarded += d
                    .write_part(&part.ids, true, guard, &mut std::io::sink())
                    .unwrap()
                    .1;
            }
            println!(
                "round {round} split guard={guard}: {} ms, {guarded} cells guarded",
                t.elapsed().as_millis()
            );
        }
    }
}
