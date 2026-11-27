//! The rule card's (C10) own speed gates on the file `rulecard_base gen` writes:
//! `cargo run --release --example rulecard_bench -- <file.csv>`
//! - learn: the card's start (the rule and the example that breaks it), gate 2 s;
//! - preview: the first cells it would fill, gate 50 ms on 1,000,000 rows;
//! - outliers: the count of the cells it fills and the odd results, against the fill in the
//!   same run, gate 1.2x;
//! - after: the count after a fill (what the status line waits for), against the fill.
//!
//! Medians of 5 runs in ms, the fill and the counts alternating.

use natendo_engine::example::ExampleSpec;
use natendo_engine::{Document, OpenOptions, Progress};
use std::time::Instant;

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let path = std::env::args().nth(1).expect("a CSV file");
    let mut d = Document::open_indexed(path.as_ref(), OpenOptions::default(), None).unwrap();
    let spec = ExampleSpec {
        col: 4,
        skip_first: true,
        ..Default::default()
    };
    let (mut learn, mut preview, mut odd, mut fill) = (vec![], vec![], vec![], vec![]);
    let mut program = None;
    for _ in 0..5 {
        let t = Instant::now();
        let rep = d.example_learn(&spec).unwrap();
        learn.push(ms(t));
        let p = rep.program.expect("a rule");
        let t = Instant::now();
        let pv = d.example_preview(&spec, &p).unwrap();
        preview.push(ms(t));
        assert_eq!(pv.rows.len(), 20);
        let t = Instant::now();
        let o = d
            .example_outliers(&spec, &p, None, &Progress::new())
            .unwrap();
        odd.push(ms(t));
        let t = Instant::now();
        let (plan, r) = d.plan_fill_example(&spec, &Progress::new()).unwrap();
        fill.push(ms(t));
        assert_eq!(o.fills, r.filled);
        drop(plan);
        program = Some(p);
    }
    let (l, pv, o, f) = (median(learn), median(preview), median(odd), median(fill));
    println!("learn {l:.1} ms (gate 2000)");
    println!("preview {pv:.2} ms (gate 50)");
    println!(
        "outliers {o:.1} ms vs fill {f:.1} ms: {:.2}x (gate 1.2)",
        o / f
    );
    // After a fill: the status line's count.
    let p = program.unwrap();
    let (plan, rep) = d.plan_fill_example(&spec, &Progress::new()).unwrap();
    d.apply_cells(plan).unwrap();
    let mut after = vec![];
    for _ in 0..5 {
        let t = Instant::now();
        let o = d
            .example_outliers(&spec, &p, Some(&rep.example_ids), &Progress::new())
            .unwrap();
        after.push(ms(t));
        assert_eq!(o.fills, rep.filled);
    }
    let a = median(after);
    println!("after the fill {a:.1} ms: {:.2}x the fill", a / f);
}
