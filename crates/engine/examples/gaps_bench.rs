//! Speed of the gap audit beside a plain value count of the same column, on
//! made-up rows. `cargo run --release -p natendo-engine --example gaps_bench`

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use natendo_engine::counts::CountsSpec;
use natendo_engine::gaps::{DateStep, GapsSpec};
use natendo_engine::{Document, OpenOptions, Progress};

fn table(rows: u64, scattered: bool) -> Document {
    let mut s = String::from("Invoice,Ward\r\n");
    for i in 0..rows {
        // All different; every thousandth number left out.
        let n = if scattered { (i * 7_919) % rows } else { i };
        let n = n + n / 1000;
        s.push_str(&format!("INV-{n:08},{}\r\n", i % 40));
    }
    Document::from_bytes(s.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
}

/// A different suffix on every row: far more shapes than are told apart.
fn shapes(rows: u64) -> Document {
    let mut s = String::from("Invoice,Ward\r\n");
    for i in 0..rows {
        s.push_str(&format!(
            "INV-{i:06}-x{}a{}b3,{}\r\n",
            i % 9973,
            i / 7,
            i % 40
        ));
    }
    Document::from_bytes(s.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
}

/// A date on every row, in no order, one day in 500 left out.
fn dates(rows: u64) -> Document {
    let mut s = String::from("Invoice,Ward\r\n");
    for i in 0..rows {
        let n = (i * 7_919) % 100_000;
        let day = n + n / 500;
        let (y, m, d) = natendo_engine::fill::civil_from_days(day as i64);
        s.push_str(&format!("{y:04}-{m:02}-{d:02},{}\r\n", i % 40));
    }
    Document::from_bytes(s.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
}

fn time(f: impl FnOnce()) -> Duration {
    let t = Instant::now();
    f();
    t.elapsed()
}

fn date_run(rows: u64) {
    let d = dates(rows);
    let gaps = GapsSpec {
        col: 0,
        keep_first: true,
        dates: Some(DateStep::Day),
        ..Default::default()
    };
    let counts = CountsSpec {
        cols: vec![0],
        keep_first: true,
    };
    let (mut a, mut b, mut c) = (Duration::MAX, Duration::MAX, Duration::MAX);
    let mut ranges = 0;
    for _ in 0..7 {
        a = a.min(time(|| {
            d.value_counts(&counts, &Progress::new()).unwrap();
        }));
        c = c.min(time(|| {
            d.date_scan(0, true, None, &Progress::new()).unwrap();
        }));
        b = b.min(time(|| {
            ranges = d.gap_audit(&gaps, &Progress::new()).unwrap().range_total;
        }));
    }
    let base = a + c;
    println!(
        "dates: value_counts {:.1} ms + date_scan {:.1} ms, date audit (its scan included) {:.1} ms, ratio {:.2} (gate 1.30), {ranges} ranges",
        a.as_secs_f64() * 1e3,
        c.as_secs_f64() * 1e3,
        b.as_secs_f64() * 1e3,
        b.as_secs_f64() / base.as_secs_f64()
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
            let _ = d.gap_audit(&gaps, p);
            let ended = Instant::now();
            worst = worst.max(ended.saturating_duration_since(h.join().unwrap()));
        });
    }
    println!(
        "dates: slowest stop after Cancel {:.1} ms",
        worst.as_secs_f64() * 1e3
    );
}

fn main() {
    let rows = 1_000_000;
    date_run(rows);
    for (name, scattered, series) in [
        ("in order", false, false),
        ("scattered", true, false),
        ("scattered, by ward", true, true),
        ("a shape on every row", false, false),
    ] {
        let d = if name == "a shape on every row" {
            shapes(rows)
        } else {
            table(rows, scattered)
        };
        let gaps = GapsSpec {
            col: 0,
            series: if series { vec![1] } else { vec![] },
            keep_first: true,
            ..Default::default()
        };
        let counts = CountsSpec {
            cols: vec![0],
            keep_first: true,
        };
        let (mut a, mut b) = (Duration::MAX, Duration::MAX);
        let mut ranges = 0;
        for _ in 0..7 {
            a = a.min(time(|| {
                d.value_counts(&counts, &Progress::new()).unwrap();
            }));
            b = b.min(time(|| {
                ranges = d.gap_audit(&gaps, &Progress::new()).unwrap().range_total;
            }));
        }
        println!(
            "{name}: value_counts {:.1} ms, gap audit {:.1} ms, ratio {:.2} (gate 1.30), {ranges} ranges",
            a.as_secs_f64() * 1e3,
            b.as_secs_f64() * 1e3,
            b.as_secs_f64() / a.as_secs_f64()
        );
        // Cancel at points through the run: the longest wait to stop.
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
                let _ = d.gap_audit(&gaps, p);
                let ended = Instant::now();
                let asked = h.join().unwrap();
                worst = worst.max(ended.saturating_duration_since(asked));
            });
        }
        println!(
            "{name}: slowest stop after Cancel {:.1} ms",
            worst.as_secs_f64() * 1e3
        );
    }
}
