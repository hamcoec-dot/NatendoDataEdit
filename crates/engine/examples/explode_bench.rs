//! Explode cells (C7) speed: on 1,000,000 rows of made-up CRM data (a JSON
//! column of 10 keys, a pipe list of 1-5 tags), Split column and Split into
//! rows on the tags (as main does), and (not `plain`) the scan, explode to
//! columns (JSON keys, fan out, flags), unnest to rows, and how fast Cancel
//! stops a scan. Plans are timed, then applied once and undone.
//! `cargo run --release --example explode_bench -- <file.csv> [plain]`
//! The file is made (made-up data) when it does not exist.
use natendo_engine::document::Progress;
use natendo_engine::ops::SplitColumn;
use natendo_engine::reshape::SplitRows;
use natendo_engine::{Document, OpenOptions};
use std::io::Write;
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

const TAGS: [&str; 7] = ["vip", "beta", "annual", "trial", "partner", "east", "west"];

fn make(path: &str) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    writeln!(f, "id,details,tags").unwrap();
    for i in 0..1_000_000u32 {
        let json = format!(
            "{{\"\"plan\"\":\"\"{}\"\",\"\"seats\"\":{},\"\"price\"\":{}.50,\"\"region\"\":\"\"R{}\"\",\"\"active\"\":{},\"\"since\"\":\"\"2026-0{}-1{}\"\",\"\"owner\"\":\"\"user{}\"\",\"\"score\"\":{},\"\"source\"\":\"\"web\"\",\"\"note\"\":null}}",
            ["gold", "free", "pro"][(i % 3) as usize],
            i % 50,
            i % 300,
            i % 9,
            i % 2 == 0,
            1 + i % 9,
            i % 10,
            i % 500,
            i % 100,
        );
        let n = 1 + (i % 5) as usize;
        let tags: Vec<&str> = (0..n).map(|k| TAGS[(i as usize + k * 3) % 7]).collect();
        writeln!(f, "{i},\"{json}\",{}", tags.join("|")).unwrap();
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("a CSV file");
    let plain = std::env::args().nth(2).is_some_and(|a| a == "plain");
    if !std::path::Path::new(&path).exists() {
        make(&path);
    }
    let mut d = Document::open_indexed(path.as_ref(), OpenOptions::default(), None).unwrap();
    let time = |name: &str, f: &dyn Fn() -> String| {
        let mut runs = Vec::new();
        let mut what = String::new();
        for _ in 0..5 {
            let t = Instant::now();
            what = f();
            runs.push(ms(t));
        }
        println!("{name}: median {:.1} ms ({what})", median(runs));
    };
    let split = SplitColumn {
        col: 2,
        separator: "|".into(),
        keep_original: true,
        keep_first: true,
        ..Default::default()
    };
    time("split column (tags)", &|| {
        let p = d.plan_split_column(&split, &Progress::new()).unwrap();
        format!("{} cells", p.cells)
    });
    let rows = SplitRows {
        col: 2,
        separator: "|".into(),
        keep_first: true,
        ..Default::default()
    };
    time("split into rows (tags)", &|| {
        let p = d.plan_split_rows(&rows, &Progress::new()).unwrap();
        format!("{} rows", p.rows)
    });
    let t = Instant::now();
    let p = d.plan_split_column(&split, &Progress::new()).unwrap();
    d.apply_cols(p).unwrap();
    println!("split column plan+apply once: {:.1} ms", ms(t));
    d.undo();
    if plain {
        return;
    }
    use natendo_engine::explode::{Explode, ExplodeHow, ExplodeKind};
    let json = Explode {
        col: 1,
        kind: ExplodeKind::Object,
        how: ExplodeHow::Columns,
        ..Default::default()
    };
    time("scan the JSON column", &|| {
        let s = d.explode_scan(&json, &Progress::new()).unwrap();
        format!("{} keys, {} bad", s.distinct, s.bad)
    });
    time("explode JSON to columns (plan)", &|| {
        let (p, r) = d.plan_explode_columns(&json, &Progress::new()).unwrap();
        format!("{} cells, {} columns", p.cells, r.cols)
    });
    let fan = Explode {
        col: 2,
        kind: ExplodeKind::List,
        how: ExplodeHow::Columns,
        ..json.clone()
    };
    time(
        "fan out tags to columns (plan; same work as split column)",
        &|| {
            let (p, r) = d.plan_explode_columns(&fan, &Progress::new()).unwrap();
            format!("{} cells, {} columns", p.cells, r.cols)
        },
    );
    let flags = Explode {
        how: ExplodeHow::Flags,
        ..fan.clone()
    };
    time("tags to flag columns (plan)", &|| {
        let (p, r) = d.plan_explode_columns(&flags, &Progress::new()).unwrap();
        format!("{} cells, {} columns", p.cells, r.cols)
    });
    let unnest = Explode {
        how: ExplodeHow::Rows,
        ..fan.clone()
    };
    time(
        "unnest tags to rows (plan; same work as split into rows)",
        &|| {
            let (p, r) = d.plan_explode_rows(&unnest, &Progress::new()).unwrap();
            format!("{} rows, {} cells changed", r.rows, p.groups)
        },
    );
    let t = Instant::now();
    let (p, _) = d.plan_explode_columns(&json, &Progress::new()).unwrap();
    d.apply_cols(p).unwrap();
    println!("explode JSON plan+apply once: {:.1} ms", ms(t));
    d.undo();
    let t = Instant::now();
    let (p, _) = d
        .plan_explode_rows(
            &Explode {
                original_col: true,
                ..unnest.clone()
            },
            &Progress::new(),
        )
        .unwrap();
    d.apply_rows(p).unwrap();
    println!(
        "unnest with the original column plan+apply once: {:.1} ms",
        ms(t)
    );
    d.undo();
    // Cancel: pressed 30 ms into a scan, how long until it stops.
    let mut worst = 0f64;
    for _ in 0..5 {
        let progress = std::sync::Arc::new(Progress::new());
        let p2 = progress.clone();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(30));
            p2.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            Instant::now()
        });
        let r = d.explode_scan(&json, &progress);
        let done = Instant::now();
        let pressed = stopper.join().unwrap();
        let after = done.saturating_duration_since(pressed).as_secs_f64() * 1000.0;
        worst = worst.max(after);
        assert!(r.is_err(), "the scan was cancelled");
    }
    println!("cancel: the scan stopped at most {worst:.1} ms after Cancel");
}
