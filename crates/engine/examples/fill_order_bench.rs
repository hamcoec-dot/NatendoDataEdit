//! Times fills planned on a 1,000,001-row table (made-up values): a 2-cell
//! number drag, a 1M-row number fill, a 1M-row month-name fill, and (where
//! the tree has custom orders) a 1M-row `Low, Medium, High` fill along a
//! custom order. Fills are read from JSON, so the same file builds on trees
//! without custom orders (there the order fill copies, and is not compared).
//! `cargo run --release --example fill_order_bench -- [runs]`

use natendo_engine::ops::FillSpec;
use natendo_engine::{Document, OpenOptions, Progress};
use std::time::Instant;

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

const ROWS: u64 = 1_000_000;

fn doc(first: &[&str]) -> Document {
    let mut t = String::from("v\n");
    for f in first {
        t.push_str(f);
        t.push('\n');
    }
    for _ in first.len() as u64..ROWS {
        t.push_str("x\n");
    }
    Document::from_bytes(t.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
}

fn spec(rows: u64, count: u64, extra: &str) -> FillSpec {
    serde_json::from_str(&format!(
        r#"{{"row":1,"col":0,"rows":{rows},"cols":1,"dir":"down","count":{count},"mode":{{"kind":"auto"}}{extra}}}"#
    ))
    .unwrap()
}

fn main() {
    let runs: usize = std::env::args().nth(1).map_or(7, |s| s.parse().expect("runs"));
    let p = Progress::new();
    let nums = doc(&["1", "2"]);
    let months = doc(&["Jan"]);
    let low = doc(&["Low"]);
    let order = r#","orders":[{"name":"Priority","items":["Low","Medium","High"],"one_cell":true}]"#;
    let cases: [(&str, &Document, FillSpec); 4] = [
        ("2-cell number drag (3 rows)", &nums, spec(2, 3, "")),
        ("1M-row number fill", &nums, spec(2, ROWS - 2, "")),
        ("1M-row month fill", &months, spec(1, ROWS - 1, "")),
        ("1M-row Low/Medium/High order fill", &low, spec(1, ROWS - 1, order)),
    ];
    for (name, d, s) in cases {
        let mut cells = 0;
        let ms = time(runs, || cells = d.plan_fill(&s, &p).unwrap().cells);
        println!("{name}\t{ms:.2} ms\t({cells} cells)");
    }
}
