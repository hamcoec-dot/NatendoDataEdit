//! Times data checks on a file: a one-column unique rule, the same rule over several
//! columns together, and the Show problems path (run, then the row plan) for older kinds:
//! `cargo run --release --example checks_bench -- <file> [runs]`
//! Rules are read from JSON so the same file builds on older trees (where `also` is
//! not known and the composite line is then a one-column rule: compare it on this tree only).

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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let runs: usize = args.get(1).map_or(5, |s| s.parse().expect("runs"));
    let d = Document::open_indexed(std::path::Path::new(&args[0]), OpenOptions::default(), None)
        .expect("open");
    let one = spec(r#"[{"col":1,"kind":{"type":"unique"}}]"#);
    let key = spec(r#"[{"col":1,"kind":{"type":"unique","also":[2,3]}}]"#);
    let old = spec(
        r#"[{"col":0,"kind":{"type":"required"}},{"col":4,"kind":{"type":"number","min":0,"max":500}},{"col":0,"kind":{"type":"unique"}}]"#,
    );
    let p = Progress::new();
    let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
    let mut found = (0, 0, 0);
    for _ in 0..runs {
        let t = Instant::now();
        found.0 = d.run_checks(&one, &p).expect("one").problems;
        a.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        found.1 = d.run_checks(&key, &p).expect("key").problems;
        b.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        let r = d.run_checks(&old, &p).expect("old");
        let plan = d.plan_show_problems(&r, true, false).expect("plan");
        found.2 = plan.rows;
        c.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let (a, b, c) = (median(a), median(b), median(c));
    println!(
        "{} rows, medians of {runs}: unique one column {a:.0} ms ({} problems), unique 3 columns together {b:.0} ms ({} problems, ratio {:.3}), show problems with older kinds {c:.0} ms ({} rows)",
        d.row_count() - 1,
        found.0,
        found.1,
        b / a,
        found.2
    );
}
