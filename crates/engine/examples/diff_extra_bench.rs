//! Branch-only timings of the key-aware diff: the whole `diff_files` (index, pair, values, result table,
//! marks index), a window of marks on each table (limit 10 ms for 60 x 20), and a next-difference jump.
//!
//! ```text
//! cargo run --release -p natendo-engine --example diff_extra_bench -- [rows] [repeats]
//! ```
//! (The fixture is the one `diff_bench` writes.)

use natendo_engine::diff::{DiffSpec, Side};
use natendo_engine::document::Progress;
use natendo_engine::{Document, OpenOptions};

use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rows: u64 = args
        .first()
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);
    let repeats: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(5);
    let dir = std::env::temp_dir().join("nde-diff-bench");
    let (pa, pb) = (
        dir.join(format!("a-{rows}.csv")),
        dir.join(format!("b-{rows}.csv")),
    );
    assert!(
        pa.exists() && pb.exists(),
        "run diff_bench first to write the fixture"
    );
    let a = Document::open_indexed(&pa, OpenOptions::default(), None).unwrap();
    let b = Document::open_indexed(&pb, OpenOptions::default(), None).unwrap();
    let best = |f: &mut dyn FnMut()| {
        f();
        let mut best = f64::MAX;
        for _ in 0..repeats {
            let t = Instant::now();
            f();
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        }
        best
    };
    let p = Progress::new();
    // `once`: only the diff itself, one time (for measuring peak memory from outside).
    let once = args.get(2).map(String::as_str) == Some("once");
    let spec = DiffSpec {
        keys: vec![0],
        keep_first: true,
        match_case: false,
        trim: true,
    };
    if once {
        let o = a.diff_files(&b, &spec, &p).unwrap();
        println!(
            "diff_files once: {} changed, {} listed",
            o.report.changed, o.report.listed
        );
        return;
    }
    let mut outcome = None;
    let ms = best(&mut || {
        outcome = Some(a.diff_files(&b, &spec, &p).unwrap());
    });
    let o = outcome.unwrap();
    println!(
        "diff_files, {rows} rows ({} changed, {} removed, {} added, {} listed)\t{ms:.1}",
        o.report.changed, o.report.removed, o.report.added, o.report.listed
    );
    for (name, doc, side, row) in [
        ("first table", &a, Side::A, 500_000u64),
        ("second table", &b, Side::B, 500_000),
    ] {
        let ms = best(&mut || {
            doc.diff_marks(&o.index, side, row, 60, 0, 20).unwrap();
        });
        println!("diff_marks, {name}, 60 rows x 20 columns in the middle\t{ms:.3}");
    }
    let ms = best(&mut || {
        a.diff_next(&o.index, Side::A, 500_001, false, &p).unwrap();
    });
    println!("diff_next from the middle (the next change is within 10 rows)\t{ms:.3}");
}
