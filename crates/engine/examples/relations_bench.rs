//! Speed of Find relationships beside the column insight strip over the same
//! columns and the same sample size, on made-up rows, alternating, best of 5:
//! `cargo run --release -p natendo-engine --example relations_bench`

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use natendo_engine::insight::InsightSpec;
use natendo_engine::relations::RelationsSpec;
use natendo_engine::{Document, OpenOptions, Progress};

const ROWS: u64 = 1_000_000;
const COLS: u64 = 30;

/// Columns of many kinds: keys of 50 to 5,000 values, columns they settle,
/// some independent, some with blanks.
fn table() -> Document {
    let mut s = String::new();
    for c in 0..COLS {
        s.push_str(&format!("c{c},"));
    }
    s.push_str("end\r\n");
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    for r in 0..ROWS {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        for c in 0..COLS {
            let key = (seed >> (c % 7)) % [50, 500, 5_000][(c % 3) as usize];
            let cell = match c % 5 {
                0 | 1 => format!("k{key}"),
                2 => format!(
                    "g{}",
                    ((seed >> ((c - 1) % 7)) % [50, 500, 5_000][((c - 1) % 3) as usize]) % 20
                ),
                3 if r % 9 == 0 => String::new(),
                3 => format!("p{}", r % 301),
                _ => format!("v{}", (seed >> c) % 12),
            };
            s.push_str(&cell);
            s.push(',');
        }
        s.push_str("x\r\n");
    }
    Document::from_bytes(s.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
}

fn time(f: impl FnOnce()) -> Duration {
    let t = Instant::now();
    f();
    t.elapsed()
}

fn main() {
    let d = table();
    let spec = RelationsSpec {
        cols: (0..COLS as u32).collect(),
        ..Default::default()
    };
    let r = d.find_relations(&spec, &Progress::new()).unwrap();
    let insights = InsightSpec {
        cols: spec.cols.clone(),
        keep_first: true,
        sample: r.sampled,
        ..Default::default()
    };
    let (mut a, mut b) = (Duration::MAX, Duration::MAX);
    let mut sampled = 0;
    for _ in 0..5 {
        a = a.min(time(|| {
            sampled = d.column_insights(&insights).unwrap().sampled;
        }));
        b = b.min(time(|| {
            d.find_relations(&spec, &Progress::new()).unwrap();
        }));
    }
    println!(
        "{ROWS} rows x {COLS} columns: {} findings, {} left out, {} pairs unchecked, sample {} rows",
        r.findings.len(),
        r.left_out.len(),
        r.unchecked,
        r.sampled
    );
    println!(
        "column_insights ({sampled} rows) {:.0} ms; find relationships {:.0} ms: ratio {:.2} (gate 3.00, and under 10 s)",
        a.as_secs_f64() * 1e3,
        b.as_secs_f64() * 1e3,
        b.as_secs_f64() / a.as_secs_f64()
    );
    let mut worst = Duration::ZERO;
    for part in 1..10 {
        let p = Progress::new();
        std::thread::scope(|s| {
            let at = b.mul_f64(f64::from(part) / 10.0);
            let p = &p;
            let h = s.spawn(move || {
                std::thread::sleep(at);
                p.cancel.store(true, Ordering::Relaxed);
                Instant::now()
            });
            let _ = d.find_relations(&spec, p);
            let ended = Instant::now();
            worst = worst.max(ended.saturating_duration_since(h.join().unwrap()));
        });
    }
    println!(
        "slowest stop after Cancel {:.0} ms",
        worst.as_secs_f64() * 1e3
    );
}
