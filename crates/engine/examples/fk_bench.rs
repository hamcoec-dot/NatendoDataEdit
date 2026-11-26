//! Times "value must exist in another table" rules on a file whose first column holds
//! distinct ids: `cargo run --release --example fk_bench -- <file> [runs]`
//! Lines: a one-column unique rule on the ids (the reference), reading the ids into a
//! key set, a run with a fresh set (read + check), a run with a set read before (the
//! app keeps it while the other table is unchanged), and the same older rules with and
//! without the rule, and an allowed-values rule (100,000 values) on the same column in
//! place of the kept-set rule (the Reviewer's reference for a kept set: at most 1.1x).

use std::sync::Arc;
use std::time::Instant;

use natendo_engine::keyset::{KeyBinding, KeySet};
use natendo_engine::validate::{Check, CheckSpec};
use natendo_engine::{Document, OpenOptions, Progress};

fn spec(json: &str, keys: Vec<KeyBinding>) -> CheckSpec {
    let checks: Vec<Check> = serde_json::from_str(json).expect("rules");
    CheckSpec {
        checks,
        keep_first: true,
        keys,
        ..Default::default()
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let runs: usize = args.get(1).map_or(5, |s| s.parse().expect("runs"));
    let d = Document::open_indexed(std::path::Path::new(&args[0]), OpenOptions::default(), None)
        .expect("open");
    let p = Progress::new();
    let fk = r#"{"col":0,"kind":{"type":"foreign_key","table":"t","column":"id"}}"#;
    let older = r#"{"col":1,"kind":{"type":"required"}},{"col":4,"kind":{"type":"number","min":0,"max":500}}"#;
    let unique = spec(r#"[{"col":0,"kind":{"type":"unique"}}]"#, vec![]);
    let values: Vec<String> = (1..=100_000).map(|i| i.to_string()).collect();
    let list = spec(
        &format!(
            r#"[{{"col":0,"kind":{{"type":"list","values":{}}}}}]"#,
            serde_json::to_string(&values).expect("values")
        ),
        vec![],
    );
    let (mut uq, mut build, mut fresh, mut kept, mut without, mut with, mut listed) =
        (vec![], vec![], vec![], vec![], vec![], vec![], vec![]);
    let read = || {
        Arc::new(KeySet::build(&d, 0, true, false, true, u64::MAX, &Progress::new()).expect("set"))
    };
    let set = read();
    for _ in 0..runs {
        let t = Instant::now();
        d.run_checks(&unique, &p).expect("unique");
        uq.push(ms(t));
        let t = Instant::now();
        let s = read();
        build.push(ms(t));
        let t = Instant::now();
        let s2 = read();
        let r = d
            .run_checks(&spec(&format!("[{fk}]"), vec![KeyBinding::Ready(s2)]), &p)
            .expect("fresh");
        fresh.push(ms(t));
        assert_eq!(r.problems, 0);
        let t = Instant::now();
        d.run_checks(
            &spec(&format!("[{fk}]"), vec![KeyBinding::Ready(set.clone())]),
            &p,
        )
        .expect("kept");
        kept.push(ms(t));
        let t = Instant::now();
        d.run_checks(&spec(&format!("[{older}]"), vec![]), &p)
            .expect("without");
        without.push(ms(t));
        let t = Instant::now();
        d.run_checks(
            &spec(
                &format!("[{older},{fk}]"),
                vec![
                    KeyBinding::None,
                    KeyBinding::None,
                    KeyBinding::Ready(set.clone()),
                ],
            ),
            &p,
        )
        .expect("with");
        with.push(ms(t));
        let t = Instant::now();
        d.run_checks(&list, &p).expect("list");
        listed.push(ms(t));
        drop(s);
    }
    let listed = median(listed);
    let (uq, build, fresh, kept, without, with) = (
        median(uq),
        median(build),
        median(fresh),
        median(kept),
        median(without),
        median(with),
    );
    println!(
        "{} rows, {} distinct keys, medians of {runs}: unique {uq:.0} ms; read the key set {build:.0} ms; value-must-exist with a fresh set {fresh:.0} ms ({:.2}x unique); with a kept set {kept:.0} ms; older rules {without:.0} ms, older rules + value-must-exist (kept set) {with:.0} ms ({:.3}x); allowed values (100k) {listed:.0} ms, kept set / allowed values {:.2}x",
        d.row_count() - 1,
        set.len(),
        fresh / uq,
        with / without,
        kept / listed
    );
}
