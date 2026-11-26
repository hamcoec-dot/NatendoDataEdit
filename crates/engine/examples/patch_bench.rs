//! Speed of patch files on 1,000,000 made-up rows with 100,000 differences,
//! beside Compare files by key and Take changes, alternating, best of 5:
//! `cargo run --release -p natendo-engine --example patch_bench`

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use natendo_engine::diff::DiffSpec;
use natendo_engine::patch::{Patch, PatchSpec};
use natendo_engine::updates::{Choice, NewerIndex, NewerValues, UpdateChoice, UpdateSpec};
use natendo_engine::{Document, OpenOptions, Progress};

const ROWS: u64 = 1_000_000;
const ROUNDS: usize = 5;

/// 80,000 changed cells, 10,000 rows removed and 10,000 added when `newer`.
fn table(newer: bool) -> Document {
    let mut s = String::from("Voter,Name,Town,Ward,Phone,Note\r\n");
    for r in 0..ROWS {
        if newer && r % 100 == 7 {
            continue;
        }
        let town = if newer && r % 25 < 2 {
            "Brackwen"
        } else {
            "Yarrowby"
        };
        s.push_str(&format!(
            "V-{r:09},Name {r},{town},{},555-{:07},kept\r\n",
            r % 40,
            r % 9_999_999
        ));
    }
    if newer {
        for r in ROWS..ROWS + 10_000 {
            s.push_str(&format!("V-{r:09},Name {r},Hollin,1,555-0000000,new\r\n"));
        }
    }
    Document::from_bytes(s.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
}

fn time(f: impl FnOnce()) -> Duration {
    let t = Instant::now();
    f();
    t.elapsed()
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// How long after Cancel the work stops, cancelled a third of the way in.
fn cancel(full: Duration, f: impl Fn(&Progress) -> bool) -> Duration {
    let p = Progress::new();
    std::thread::scope(|s| {
        let asked = s.spawn(|| {
            std::thread::sleep(full / 3);
            p.cancel.store(true, Ordering::Relaxed);
            Instant::now()
        });
        assert!(f(&p), "it finished before Cancel");
        asked.join().unwrap().elapsed()
    })
}

fn main() {
    let (old, new) = (table(false), table(true));
    let mut target = table(false);
    let spec = PatchSpec {
        keys: vec![0],
        trim: true,
        ..Default::default()
    };
    let dspec = DiffSpec {
        keys: vec![0],
        keep_first: true,
        match_case: false,
        trim: true,
    };
    let uspec = UpdateSpec {
        keys: vec![0],
        keep_first: true,
        match_case: false,
        trim: true,
    };
    let all = || Choice {
        all: true,
        picked: Vec::new(),
    };
    let choice = UpdateChoice {
        cells: all(),
        added: all(),
        removed: all(),
        cells_skip_hidden: false,
        ..Default::default()
    };
    let p = Progress::new();
    let mut best = [Duration::MAX; 7];
    let mut keep = |i: usize, d: Duration| best[i] = best[i].min(d);
    let mut patch = None;
    let mut bytes = Vec::new();
    for _ in 0..ROUNDS {
        keep(
            0,
            time(|| patch = Some(old.make_patch(&new, &spec, &p).unwrap().patch)),
        );
        keep(
            1,
            time(|| {
                let titles = old.diff_titles(&dspec);
                let index = Document::diff_index(&new, &titles, &dspec, &p).unwrap();
                let first = old.diff_first(&index, &dspec, &p).unwrap();
                new.diff_second(index, first, &dspec, &p).unwrap();
            }),
        );
        let made = patch.as_ref().unwrap();
        keep(2, time(|| bytes = made.to_bytes(None).unwrap()));
        let mut loaded = None;
        keep(
            3,
            time(|| loaded = Some(Patch::from_bytes(&bytes, None).unwrap())),
        );
        // What the app does: the dry run's plan is what Apply applies.
        let loaded = loaded.unwrap();
        keep(
            4,
            time(|| {
                let (plan, report) = target.plan_patch(&loaded, &p).unwrap();
                assert_eq!(report.unapplied(), 0);
                target.apply_rows(plan).unwrap();
            }),
        );
        assert!(target.undo());
        // Take changes: find them, then apply them all.
        let mut apply_only = Duration::ZERO;
        keep(
            5,
            time(|| {
                let cols: Vec<u32> = (0..target.col_count()).collect();
                let titles = target.titles(&cols, true);
                let index = NewerIndex::build(&new, &titles, &uspec, &p).unwrap();
                let pairing = target.pair_newer(&index, &uspec, &p).unwrap();
                let values = NewerValues::fetch(&new, &index, &pairing, &p).unwrap();
                let (ups, _) = target.diff_newer(&index, &pairing, values, &p).unwrap();
                apply_only = time(|| {
                    let plan = target.plan_updates(&ups, &choice).unwrap();
                    target.apply_rows(plan).unwrap();
                });
            }),
        );
        keep(6, apply_only);
        assert!(target.undo());
    }
    let made = patch.unwrap();
    println!(
        "{} changed rows, {} added, {} removed; patch file {:.1} MB",
        made.changes.len(),
        made.adds.len(),
        made.removes.len(),
        bytes.len() as f64 / 1e6
    );
    let names = [
        "make patch",
        "compare by key",
        "seal and write bytes",
        "load, check seal and entries",
        "dry run + apply (patch)",
        "take changes: find + apply",
        "take changes: apply only",
    ];
    for (n, d) in names.iter().zip(best) {
        println!("{n:<32} {:>9.1} ms", ms(d));
    }
    println!(
        "create / compare by key = {:.2} (gate 1.2)",
        ms(best[0]) / ms(best[1])
    );
    println!(
        "dry run + apply / take changes = {:.2} (gate 1.2)",
        ms(best[4]) / ms(best[5])
    );
    println!(
        "cancel: make {:.0} ms, dry run {:.0} ms (gate 1000)",
        ms(cancel(best[0], |p| old.make_patch(&new, &spec, p).is_err())),
        ms(cancel(best[4] / 4, |p| target
            .plan_patch(&made, p)
            .is_err()))
    );
}
