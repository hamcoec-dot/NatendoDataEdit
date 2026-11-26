//! Report notes (C4) speed: open, write and save a large file as before, and
//! set rows aside, undo and save with notes.
//! `cargo run --release --example notes_bench -- <file.csv> [plain]`
//! (`plain`: only what main can do too, for a side-by-side run.)
use natendo_engine::save::SaveOptions;
use natendo_engine::{Document, OpenOptions};
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let path = std::env::args().nth(1).expect("a CSV file");
    let plain = std::env::args().nth(2).is_some_and(|a| a == "plain");
    let opts = OpenOptions {
        skip_lines: Some(0),
        ..Default::default()
    };
    let mut best = [f64::MAX; 3];
    for _ in 0..3 {
        let t = Instant::now();
        let d = Document::open_indexed(path.as_ref(), opts, None).unwrap();
        best[0] = best[0].min(ms(t));
        let t = Instant::now();
        d.write_to(&mut std::io::sink(), d.format(), false).unwrap();
        best[1] = best[1].min(ms(t));
    }
    // A save over a copy of the file after one edit (rewrite, verify, reopen).
    let dir = std::env::temp_dir().join(format!("nde-notes-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let copy = dir.join("copy.csv");
    for _ in 0..3 {
        std::fs::copy(&path, &copy).unwrap();
        let mut d = Document::open_indexed(&copy, opts, None).unwrap();
        d.set_cell(5, 1, "edited".into()).unwrap();
        let t = Instant::now();
        d.save(SaveOptions::default()).unwrap();
        best[2] = best[2].min(ms(t));
    }
    println!(
        "open {:.0} ms, write {:.0} ms, save {:.0} ms ({} rows)",
        best[0],
        best[1],
        best[2],
        Document::open_indexed(path.as_ref(), opts, None)
            .unwrap()
            .total_rows()
    );
    if plain {
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    use natendo_engine::document::report::Place;
    let mut aside = [f64::MAX; 5];
    for _ in 0..3 {
        std::fs::copy(&path, &copy).unwrap();
        let mut d = Document::open_indexed(&copy, opts, None).unwrap();
        let n = d.total_rows();
        let t = Instant::now();
        d.set_aside_rows(0, 4, Place::Head).unwrap();
        aside[0] = aside[0].min(ms(t));
        let t = Instant::now();
        assert!(d.undo());
        aside[1] = aside[1].min(ms(t));
        assert!(d.redo());
        let t = Instant::now();
        d.set_aside_rows(n - 5, 1, Place::Foot).unwrap();
        aside[2] = aside[2].min(ms(t));
        let t = Instant::now();
        d.write_to(&mut std::io::sink(), d.format(), false).unwrap();
        aside[3] = aside[3].min(ms(t));
        let t = Instant::now();
        d.save(SaveOptions::default()).unwrap();
        aside[4] = aside[4].min(ms(t));
        assert_eq!(std::fs::read(&copy).unwrap(), std::fs::read(&path).unwrap());
    }
    // The total-row check (piece b) against the column totals over the same rows.
    let d = Document::open_indexed(path.as_ref(), opts, None).unwrap();
    let read = natendo_engine::value::ReadOptions::default();
    let p = natendo_engine::Progress::new();
    let cols: Vec<u32> = (0..d.col_count()).collect();
    let mut tb = [f64::MAX; 2];
    for _ in 0..5 {
        let t = Instant::now();
        let found = d.find_total_row(true, read, &p).unwrap();
        tb[0] = tb[0].min(ms(t));
        assert!(found.is_some_and(|f| f.adds_up));
        let t = Instant::now();
        d.column_totals(&cols, true, read, &p).unwrap();
        tb[1] = tb[1].min(ms(t));
    }
    println!(
        "total row check {:.1} ms, column totals of every column {:.1} ms ({:.2}x)",
        tb[0],
        tb[1],
        tb[0] / tb[1]
    );
    println!(
        "set aside 4 head {:.1} ms, undo {:.1} ms, set aside 1 foot {:.1} ms, write with notes {:.0} ms, save with notes {:.0} ms (bytes equal the original)",
        aside[0], aside[1], aside[2], aside[3], aside[4]
    );
    let _ = std::fs::remove_dir_all(&dir);
}
