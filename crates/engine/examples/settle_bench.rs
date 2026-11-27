//! Times a "settled by" rule on a file (columns id, precinct, ward, n): the rule (ward
//! settled by precinct), one Unique rule on precinct for scale, the scan's own exact count
//! for the same pair (Find relationships > Show the exceptions), and the older rules
//! (Required, Number, Unique) that must not get slower.
//! `cargo run --release --example settle_bench -- <file> [runs] [old]`; with `old`, only
//! the older rules (it builds on trees without the new kind: rules are read from JSON).

use natendo_engine::validate::{Check, CheckSpec};
use natendo_engine::{Document, OpenOptions, Progress};
use std::time::Instant;

fn spec(json: &str) -> CheckSpec {
    let checks: Vec<Check> = serde_json::from_str(json).expect("rules");
    CheckSpec {
        checks,
        keep_first: true,
        ..Default::default()
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn time(runs: usize, mut f: impl FnMut()) -> f64 {
    median(
        (0..runs)
            .map(|_| {
                let s = Instant::now();
                f();
                s.elapsed().as_secs_f64() * 1000.0
            })
            .collect(),
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let runs: usize = args.get(1).map_or(5, |s| s.parse().expect("runs"));
    let only_old = args.get(2).is_some_and(|a| a == "old");
    let d = Document::open_indexed(std::path::Path::new(&args[0]), OpenOptions::default(), None)
        .expect("open");
    let p = Progress::new();
    let old = spec(
        r#"[{"col":0,"kind":{"type":"required"}},{"col":3,"kind":{"type":"number","min":0,"max":500}},{"col":0,"kind":{"type":"unique"}}]"#,
    );
    let t_old = time(runs, || {
        d.run_checks(&old, &p).unwrap();
    });
    println!("older rules (required, number, unique)\t{t_old:.1}");
    if only_old {
        return;
    }
    let unique = spec(r#"[{"col":1,"kind":{"type":"unique"}}]"#);
    let settled = spec(r#"[{"col":2,"kind":{"type":"settled_by","by":1}}]"#);
    let t_unique = time(runs, || {
        d.run_checks(&unique, &p).unwrap();
    });
    let mut problems = 0;
    let t_settled = time(runs, || {
        problems = d.run_checks(&settled, &p).unwrap().problems;
    });
    let scan = d
        .find_relations(
            &natendo_engine::relations::RelationsSpec {
                cols: vec![1, 2],
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    let mut shown = 0;
    let t_scan = time(runs, || {
        shown = d
            .plan_relation_exceptions(
                &scan,
                natendo_engine::relations::RelationKind::Depends,
                1,
                2,
                &p,
            )
            .unwrap()
            .rows;
    });
    println!("one unique rule on precinct\t{t_unique:.1}");
    println!("ward settled by precinct\t{t_settled:.1}\t{problems} problems");
    println!("the scan's exact count (exceptions)\t{t_scan:.1}\t{shown} rows");
    println!("settled vs unique\t{:.2}x", t_settled / t_unique);
}
