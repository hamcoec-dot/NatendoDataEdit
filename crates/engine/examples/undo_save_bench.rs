//! Undo after save, one case per process (so peak memory is the case's own):
//! `undo_save_bench <file> <case>` with case `none`, `e1k`, `e100k`, `e1m`, `sort100k` (100,000 edits + a sort)
//! or `cleans3` (three whole-column clean-ups). Copies `<file>` aside, makes the changes, saves over the copy and
//! prints `save_ms`, and (where the history survives) `undo_ms` for undoing everything. The process then waits a
//! moment so a watcher can read its peak memory.

use natendo_engine::clean::{CaseTo, CleanSpec, Cleaner, Step};
use natendo_engine::find::Scope;
use natendo_engine::sort::{SortKey, SortSpec};
use natendo_engine::{Document, OpenOptions, Progress, SaveOptions};
use std::time::Instant;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (src, case) = (&a[0], a[1].as_str());
    let work = std::path::PathBuf::from(format!("{src}.{case}.{}.csv", std::process::id()));
    std::fs::copy(src, &work).expect("copy");
    let mut d = Document::open_indexed(&work, OpenOptions::default(), None).expect("open");
    let rows = d.row_count();
    let edits = |d: &mut Document, n: u64| {
        let cells: Vec<(u64, u32, String)> = (0..n)
            .map(|i| (1 + (i * 7919) % (rows - 1), 1, format!("e{i}")))
            .collect();
        d.set_cells(cells).expect("edits");
    };
    match case {
        "none" => d
            .set_cells(vec![(1, 1, "x".into())])
            .map(|_| ())
            .expect("one edit"),
        "e1k" => edits(&mut d, 1_000),
        "e100k" => edits(&mut d, 100_000),
        "e1m" => edits(&mut d, (rows - 1).min(1_000_000)),
        "sort100k" => {
            edits(&mut d, 100_000);
            let spec = SortSpec {
                keys: vec![SortKey {
                    col: 1,
                    descending: true,
                    kind: Default::default(),
                    match_case: false,
                }],
                keep_first: true,
                ..Default::default()
            };
            d.sort(&spec).expect("sort");
        }
        "cleans3" => {
            for c in 0..3 {
                let spec = CleanSpec {
                    cleaner: Cleaner {
                        version: 1,
                        name: String::new(),
                        steps: vec![Step::Case { to: CaseTo::Upper }],
                    },
                    scope: Scope::Columns { cols: vec![c] },
                    skip_first: true,
                    read: Default::default(),
                };
                let p = d
                    .plan_clean(&spec, 20_000.0, 1, 0, &Progress::new())
                    .expect("plan");
                d.apply_clean(p).expect("clean");
            }
        }
        other => panic!("unknown case {other}"),
    }
    // "none": no history at all (the one edit is undone, then the file is rewritten).
    let rewrite = case == "none";
    if rewrite {
        d.undo();
    }
    // A watcher reads the memory in use while each mark line waits.
    let mark = |what: &str| {
        use std::io::Write;
        println!("{what}");
        let _ = std::io::stdout().flush();
        std::thread::sleep(std::time::Duration::from_millis(500));
    };
    mark("before");
    let t = Instant::now();
    d.save(SaveOptions {
        rewrite,
        ..Default::default()
    })
    .expect("save");
    let save_ms = t.elapsed().as_secs_f64() * 1000.0;
    mark("after");
    let t = Instant::now();
    let mut undone = 0;
    while d.undo() {
        undone += 1;
    }
    let undo_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!("save_ms {save_ms:.1} undo_ms {undo_ms:.1} undone {undone}");
    drop(d);
    let _ = std::fs::remove_file(&work);
    std::thread::sleep(std::time::Duration::from_millis(600));
}
