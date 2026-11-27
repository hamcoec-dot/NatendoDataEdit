//! The Ctrl+E point of the rule card (C10) speed gates, with APIs main has too, so the same file
//! builds on main and on the C10 branch and the two can be timed side by side:
//! `cargo run --release --example rulecard_base -- <file.csv> <gen|fill> [rows]`
//! - gen: write a made-up file (id, first, last, city, short) of `rows` rows (1,000,000 by
//!   default), `short` typed in the first 3 rows ("A.Lee") and empty below;
//! - fill: plan Ctrl+E's fill of `short` (learn, then every empty cell), median of 5 runs in ms.

use natendo_engine::example::ExampleSpec;
use natendo_engine::{Document, OpenOptions, Progress};
use std::io::Write;
use std::time::Instant;

pub fn gen(path: &str, rows: u32) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    writeln!(f, "id,first,last,city,short").unwrap();
    let firsts = ["Ann", "Bo", "Cy", "Di", "Ed", "Flo", "Gus", "Hal"];
    let lasts = ["Moss", "Lane", "Hart", "Fox", "Ray", "Oakley", "Elm", "Ash"];
    for i in 0..rows {
        let (a, l) = (firsts[(i % 8) as usize], lasts[(i / 8 % 8) as usize]);
        let short = if i < 3 {
            format!("{}.{l}", &a[..1])
        } else {
            String::new()
        };
        writeln!(f, "{i},{a},{l},Town {},{short}", i % 40).unwrap();
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

pub fn spec() -> ExampleSpec {
    serde_json::from_str(r#"{"col":4,"skip_first":true}"#).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = &args[1];
    let what = args.get(2).map(String::as_str).unwrap_or("fill");
    if what == "gen" {
        let rows = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
        gen(path, rows);
        return;
    }
    let d = Document::open_indexed(path.as_ref(), OpenOptions::default(), None).unwrap();
    let mut times = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        let (plan, rep) = d.plan_fill_example(&spec(), &Progress::new()).unwrap();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        assert!(rep.filled > 0);
        drop(plan);
    }
    println!("fill {:.1} ms (median of 5; {times:.1?})", median(times.clone()));
}
