//! Speed of golden records on 1,000,000 made-up rows holding 50,000 groups of
//! 2 to 4 look-alikes, beside the Match people pass they follow:
//! `cargo run --release -p natendo-engine --example golden_bench`
//! (`once`: one pass of match and golden only, for measuring peak memory).

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use natendo_engine::fuzzy::{find_matches, FieldKind, MatchField, MatchSide, MatchSpec};
use natendo_engine::golden::{ColumnRule, GoldenSpec, Rule};
use natendo_engine::{Document, OpenOptions, Progress};

const ROWS: u64 = 1_000_000;
const GROUPS: u64 = 50_000;

/// Letters only, different for every number.
fn word(mut n: u64) -> String {
    const SYL: [&str; 20] = [
        "ba", "ce", "di", "fo", "gu", "ka", "le", "mi", "no", "pu", "ra", "se", "ti", "vo", "zu",
        "han", "wel", "dor", "mun", "yar",
    ];
    let mut s = String::new();
    for _ in 0..5 {
        s.push_str(SYL[(n % 20) as usize]);
        n /= 20;
    }
    s
}

fn table() -> Document {
    let mut s = String::from("Id,Name,Born,Town,Paid,Seen\r\n");
    let mut id = 0u64;
    let mut person = 0u64;
    while id < ROWS {
        // The first groups of people are written 2, 3 or 4 times.
        let times = if person < GROUPS { 2 + person % 3 } else { 1 };
        for k in 0..times.min(ROWS - id) {
            s.push_str(&format!(
                "{id},{} {},19{:02}-{:02}-{:02},Town {},{}.{:02},2025-{:02}-{:02}\r\n",
                word(person.wrapping_mul(7_919) % 3_200_000),
                word(person % 3_200_000),
                40 + person % 60,
                1 + person % 12,
                1 + person % 28,
                k,
                id % 1000,
                id % 100,
                1 + id % 12,
                1 + id % 28,
            ));
            id += 1;
        }
        person += 1;
    }
    Document::from_bytes(s.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
}

fn time<T>(f: impl FnOnce() -> T) -> (Duration, T) {
    let t = Instant::now();
    let r = f();
    (t.elapsed(), r)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn main() {
    let once = std::env::args().any(|a| a == "once");
    let d = table();
    let p = Progress::new();
    let spec = MatchSpec {
        fields: [(1, FieldKind::Name), (2, FieldKind::Exact)]
            .into_iter()
            .map(|(col, kind)| MatchField {
                col,
                other_col: col,
                kind,
                weight: 1.0,
            })
            .collect(),
        keep_first: true,
        other_header: true,
        min_score: 95.0,
        ..Default::default()
    };
    let golden = GoldenSpec {
        rules: [
            (3, Rule::MostCommon),
            (4, Rule::Sum),
            (
                5,
                Rule::Newest {
                    blank_counts: false,
                },
            ),
        ]
        .into_iter()
        .map(|(col, rule)| ColumnRule { col, rule })
        .collect(),
        id_col: Some(0),
        date_col: Some(5),
        ..Default::default()
    };
    let rounds = if once { 1 } else { 5 };
    let mut best = [Duration::MAX; 4];
    let mut made = None;
    for _ in 0..rounds {
        let (matching, report) = time(|| {
            let kinds = [FieldKind::Name, FieldKind::Exact];
            let side = MatchSide::build(&d, &[1, 2], &kinds, &[], true, spec.read, &p).unwrap();
            find_matches(&side, None, &spec, &p).unwrap()
        });
        let accepted: Vec<(u32, u64, f32)> = report
            .listed
            .iter()
            .map(|c| (c.id, c.other_row, c.score))
            .collect();
        let (planning, plan) = time(|| {
            let pairs = d.golden_pairs(&accepted).unwrap();
            d.plan_golden(&pairs, &golden, &p).unwrap()
        });
        let (making, out) = time(|| {
            let (table, away) = d.golden_tables(&plan, true, &p).unwrap();
            (
                table.into_document().unwrap(),
                away.map(|t| t.into_document().unwrap()),
            )
        });
        // The same rows copied to a new tab the ordinary way.
        let (copying, _) = time(|| {
            let mut bytes = Vec::new();
            for r in 0..d.row_count() {
                bytes.extend_from_slice(d.row_values(r).unwrap().join(",").as_bytes());
                bytes.extend_from_slice(b"\r\n");
            }
            Document::from_bytes(bytes, OpenOptions::default(), Some("csv")).unwrap()
        });
        for (b, t) in best.iter_mut().zip([matching, planning, making, copying]) {
            *b = (*b).min(t);
        }
        made = Some((plan, out.0.row_count(), accepted));
    }
    let (plan, rows, accepted) = made.unwrap();
    println!(
        "{} pairs, {} clusters, {} rows folded, {} for review, {} rows in the result",
        accepted.len(),
        plan.counts.clusters,
        plan.counts.folded,
        plan.counts.review,
        rows - 1
    );
    for (n, t) in [
        "match people pass",
        "golden plan",
        "result tabs",
        "plain copy of every row to a tab",
    ]
    .iter()
    .zip(best)
    {
        println!("{n:<34} {:>9.1} ms", ms(t));
    }
    let gate = 1.2 * ms(best[0]) + 500.0 * plan.counts.clusters as f64 / 100_000.0;
    println!(
        "plan {:.0} ms, gate {gate:.0} ms; result / plain copy = {:.2}",
        ms(best[1]),
        ms(best[2]) / ms(best[3])
    );
    if once {
        return;
    }
    let pairs = d.golden_pairs(&accepted).unwrap();
    let stop = Progress::new();
    let late = std::thread::scope(|s| {
        let asked = s.spawn(|| {
            std::thread::sleep(best[1] / 3);
            stop.cancel.store(true, Ordering::Relaxed);
            Instant::now()
        });
        assert!(
            d.plan_golden(&pairs, &golden, &stop).is_err(),
            "it finished before Cancel"
        );
        asked.join().unwrap().elapsed()
    });
    println!("cancel: plan {:.0} ms (gate 1000)", ms(late));
}
