//! Speed of the clean matrix beside today's ways of doing the same work, on
//! made-up rows, alternating, best of 7:
//! `cargo run --release -p natendo-engine --example matrix_bench`

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use natendo_engine::clean::{CaseTo, CleanMatrix, CleanSpec, Cleaner, MatrixEntry, Spaces, Step};
use natendo_engine::find::Scope;
use natendo_engine::{Document, OpenOptions, Progress};

const ROWS: u64 = 1_000_000;
const COLS: u32 = 8;
const ROUNDS: usize = 7;

fn table() -> Document {
    let mut s = String::new();
    for c in 0..COLS + 2 {
        s.push_str(&format!("c{c},"));
    }
    s.push_str("end\r\n");
    for r in 0..ROWS {
        s.push_str(&format!("{r},"));
        for c in 1..COLS + 1 {
            // Most cells need work: spaces around and inside, small letters.
            s.push_str(&format!("  ann{}  smith {c} ,", r % 977));
        }
        s.push_str("x,y\r\n");
    }
    Document::from_bytes(s.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
}

fn steps() -> Vec<Step> {
    vec![
        Step::Spaces { how: Spaces::Trim },
        Step::Spaces {
            how: Spaces::Collapse,
        },
        Step::Case { to: CaseTo::Upper },
    ]
}

fn matrix(cols: u32) -> CleanMatrix {
    CleanMatrix {
        entries: (1..=cols)
            .map(|col| MatrixEntry {
                col,
                title: String::new(),
                steps: steps(),
                from_file: String::new(),
            })
            .collect(),
        skip_first: true,
        read: Default::default(),
    }
}

fn spec(cols: Vec<u32>, steps: Vec<Step>) -> CleanSpec {
    CleanSpec {
        cleaner: Cleaner {
            version: 1,
            name: String::new(),
            steps,
        },
        scope: Scope::Columns { cols },
        skip_first: true,
        read: Default::default(),
    }
}

fn time(f: impl FnOnce()) -> Duration {
    let t = Instant::now();
    f();
    t.elapsed()
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn main() {
    let d = table();
    let p = Progress::new;
    let one_pass = |cols: u32| {
        d.plan_clean_matrix(&matrix(cols), 20_000.0, 1, &p())
            .unwrap();
    };
    let plan = |s: &CleanSpec| {
        d.plan_clean(s, 20_000.0, 1, 0, &p()).unwrap();
    };
    let all = spec((1..=COLS).collect(), steps());
    let single = spec(vec![1], steps());
    let mut best = [Duration::MAX; 7];
    for _ in 0..ROUNDS {
        let runs: [Duration; 7] = [
            time(|| one_pass(COLS)),
            time(|| plan(&all)),
            time(|| {
                for c in 1..=COLS {
                    plan(&spec(vec![c], steps()));
                }
            }),
            time(|| {
                for c in 1..=COLS {
                    for s in steps() {
                        plan(&spec(vec![c], vec![s]));
                    }
                }
            }),
            time(|| one_pass(1)),
            time(|| plan(&single)),
            time(|| {
                d.preview_clean_matrix(&matrix(COLS), 20_000.0, 1, &p())
                    .unwrap();
            }),
        ];
        for (b, r) in best.iter_mut().zip(runs) {
            *b = (*b).min(r);
        }
    }
    let [m, one, per_col, per_cell, m1, p1, look] = best.map(ms);
    println!("{ROWS} rows, {COLS} columns x 3 cleaners");
    println!("matrix {m:.0} ms; preview {look:.0} ms");
    println!(
        "one plan_clean over the 8 columns {one:.0} ms: ratio {:.3} (gate 1.05)",
        m / one
    );
    println!(
        "8 per-column runs {per_col:.0} ms: ratio {:.3} (gate 0.50)",
        m / per_col
    );
    println!(
        "24 separate runs {per_cell:.0} ms: ratio {:.3} (gate 0.50)",
        m / per_cell
    );
    println!("one column: matrix {m1:.0} ms, plan_clean {p1:.0} ms: ratio {:.3} (gate 1.05); each further column {:.1} ms", m1 / p1, (m - m1) / f64::from(COLS - 1));
    // Cancel at points through the run: the longest wait to stop.
    let mut worst = Duration::ZERO;
    for part in 1..10 {
        let progress = Progress::new();
        std::thread::scope(|s| {
            let at = Duration::from_secs_f64(m / 1e3 * f64::from(part) / 10.0);
            let progress = &progress;
            let h = s.spawn(move || {
                std::thread::sleep(at);
                progress.cancel.store(true, Ordering::Relaxed);
                Instant::now()
            });
            let _ = d.plan_clean_matrix(&matrix(COLS), 20_000.0, 1, progress);
            let ended = Instant::now();
            worst = worst.max(ended.saturating_duration_since(h.join().unwrap()));
        });
    }
    println!("slowest stop after Cancel {:.1} ms", ms(worst));
}
