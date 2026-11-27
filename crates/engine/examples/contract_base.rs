//! The comparison points of the file contract (C9) speed gates, with APIs that main has too, so the
//! same file builds on main and on the C9 branch and the two can be timed side by side:
//! `cargo run --release --example contract_base -- <file.csv> <gen|checks|same|tally> [rows]`
//! - gen: write a made-up monthly file (10 columns) of `rows` rows (1,000,000 by default);
//! - checks: Data checks with the contract's two rules plus a "required" rule on each of the 10
//!   columns (the blank count), as main runs them;
//! - same: those plus the rules the contract implies (amount a number, joined a date, party and status
//!   from their lists): the same checks as the drift pass, as main runs them;
//! - tally: one Edit values tally of each of the 10 columns.
//!
//! Each prints the median of 5 runs in ms.

use natendo_engine::editing::TallySpec;
use natendo_engine::validate::{Check, CheckKind, CheckSpec};
use natendo_engine::{Document, OpenOptions, Progress};
use std::io::Write;
use std::time::Instant;

pub fn gen(path: &str, rows: u32) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    writeln!(
        f,
        "id,first,last,party,precinct,phone,joined,amount,status,town"
    )
    .unwrap();
    let parties = ["DEM", "REP", "IND", "GRN", "LIB"];
    let firsts = ["Ann", "Bo", "Cy", "Di", "Ed", "Flo", "Gus", "Hal"];
    let lasts = ["Moss", "Lane", "Hart", "Fox", "Ray", "Oak", "Elm", "Ash"];
    for i in 0..rows {
        let phone = if i % 6 == 0 {
            String::new()
        } else {
            format!("555-{:04}", i % 10_000)
        };
        writeln!(
            f,
            "{i},{},{},{},P{:03},{phone},2026-{:02}-{:02},{}.{:02},{},Town {}",
            firsts[(i % 8) as usize],
            lasts[(i / 8 % 8) as usize],
            parties[(i % 13 % 5) as usize],
            i % 300,
            1 + i % 12,
            1 + i % 28,
            i % 5000,
            i % 100,
            if i % 9 == 0 { "moved" } else { "active" },
            i % 40
        )
        .unwrap();
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// The contract's own rules: phone required, amount a number.
pub fn rules() -> Vec<Check> {
    vec![
        Check::new(Some(5), CheckKind::Required),
        Check::new(
            Some(7),
            serde_json::from_str(r#"{"type":"number"}"#).expect("a number rule"),
        ),
    ]
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = &args[0];
    let rows: u32 = args.get(2).map_or(1_000_000, |s| s.parse().unwrap());
    match args[1].as_str() {
        "gen" => gen(path, rows),
        mode => {
            let d = Document::open_indexed(path.as_ref(), OpenOptions::default(), None).unwrap();
            let mut times = Vec::new();
            for _ in 0..5 {
                let t = Instant::now();
                match mode {
                    "checks" | "same" => {
                        let mut checks = rules();
                        checks.extend((0..10).map(|c| Check::new(Some(c), CheckKind::Required)));
                        if mode == "same" {
                            // What the contract also asks: amount a number, joined a date, party and
                            // status from their lists.
                            for (c, k) in [
                                (7, r#"{"type":"number"}"#),
                                (6, r#"{"type":"date"}"#),
                                (3, r#"{"type":"list","values":["DEM","GRN","IND","LIB","REP"],"match_case":true}"#),
                                (8, r#"{"type":"list","values":["active","moved"],"match_case":true}"#),
                            ] {
                                checks.push(Check::new(Some(c), serde_json::from_str(k).expect("a rule")));
                            }
                        }
                        let spec = CheckSpec {
                            checks,
                            keep_first: true,
                            ..Default::default()
                        };
                        let r = d.run_checks(&spec, &Progress::new()).unwrap();
                        std::hint::black_box(r.problems);
                    }
                    "tally" => {
                        for col in 0..10 {
                            let r = d
                                .value_tally(
                                    &TallySpec {
                                        col,
                                        keep_first: true,
                                        limit: None,
                                    },
                                    &Progress::new(),
                                )
                                .unwrap();
                            std::hint::black_box(r);
                        }
                    }
                    _ => panic!("gen, checks, same or tally"),
                }
                times.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            let m = median(times.clone());
            println!("{mode}: median {m:.1} ms ({times:.0?})");
        }
    }
}
