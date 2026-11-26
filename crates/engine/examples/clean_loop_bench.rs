//! `plan_clean` over eight columns and over one, to compare this build with
//! main side by side (the clean matrix shares its per-cell loop):
//! `cargo run --release -p natendo-engine --example clean_loop_bench`

use std::time::{Duration, Instant};

use natendo_engine::clean::{CaseTo, CleanSpec, Cleaner, Spaces, Step};
use natendo_engine::find::Scope;
use natendo_engine::{Document, OpenOptions, Progress};

const ROWS: u64 = 1_000_000;
const COLS: u32 = 8;
const ROUNDS: usize = 9;

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

fn main() {
    let d = table();
    for (name, cols) in [
        ("8 columns", (1..=COLS).collect::<Vec<_>>()),
        ("1 column", vec![1]),
    ] {
        let s = spec(cols, steps());
        let mut best = Duration::MAX;
        for _ in 0..ROUNDS {
            best = best.min(time(|| {
                d.plan_clean(&s, 20_000.0, 1, 0, &Progress::new()).unwrap();
            }));
        }
        println!(
            "plan_clean, {name}: best {:.1} ms",
            best.as_secs_f64() * 1e3
        );
    }
}
