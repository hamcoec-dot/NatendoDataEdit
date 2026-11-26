//! Fill blank cells (C5) speed on a grouped 1,000,000-row report: today's fill
//! (no stops), each stop, all stops, and the dialog's capped count.
//! `cargo run --release --example fill_bench -- <file.csv> [plain]`
//! (`plain`: only what main can do too, for a side-by-side run.) The file is
//! made (made-up data) when it does not exist.
use natendo_engine::document::Progress;
use natendo_engine::tools::FillBlanks;
use natendo_engine::{Document, OpenOptions};
use std::io::Write;
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn make(path: &str) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    writeln!(f, "region,id,amount,city,note,code,day,flag").unwrap();
    let regions = ["North", "South", "East", "West"];
    for i in 0..1_000_000u32 {
        let g = i / 25;
        let k = i % 25;
        if k == 24 {
            // A subtotal row, then (every other group) a spacer.
            writeln!(f, "Subtotal,,{},,,,,", 100 + g % 50).unwrap();
            continue;
        }
        if k == 23 && g % 2 == 0 {
            writeln!(f, ",,,,,,,").unwrap();
            continue;
        }
        let region = if k == 0 {
            regions[(g % 4) as usize]
        } else {
            ""
        };
        let id = if k % 11 == 7 {
            String::new()
        } else {
            format!("{}", 100_000 + i)
        };
        writeln!(
            f,
            "{region},{id},{}.{:02},Dayton,ok,C{},2024-01-{:02},y",
            i % 97,
            i % 100,
            i % 13,
            1 + i % 28
        )
        .unwrap();
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let path = std::env::args().nth(1).expect("a CSV file");
    let plain = std::env::args().nth(2).is_some_and(|a| a == "plain");
    if !std::path::Path::new(&path).exists() {
        make(&path);
    }
    let d = Document::open_indexed(path.as_ref(), OpenOptions::default(), None).unwrap();
    let spec = |extra: &str| -> FillBlanks {
        serde_json::from_str(&format!(
            r#"{{"cols":[0],"from":{{"kind":"above"}},"spaces":true,"keep_first":true{extra}}}"#
        ))
        .unwrap()
    };
    let mut cases = vec![("no stops (today's fill)", spec(""))];
    if !plain {
        cases.push(("guard on id", spec(r#","guard":1"#)));
        // A guard that never stops: the same cells filled as today, the cost of reading the guard.
        cases.push(("guard on a full column", spec(r#","guard":3"#)));
        cases.push(("blank rows", spec(r#","stop_blank_rows":true"#)));
        cases.push(("total rows", spec(r#","stop_total_rows":true"#)));
        cases.push((
            "all three",
            spec(r#","guard":1,"stop_blank_rows":true,"stop_total_rows":true"#),
        ));
    }
    for (name, s) in &cases {
        let mut runs = Vec::new();
        let mut cells = 0;
        for _ in 0..7 {
            let t = Instant::now();
            let p = d.plan_fill_blanks(s, &Progress::new()).unwrap();
            runs.push(ms(t));
            cells = p.groups;
        }
        println!("{name}: median {:.1} ms ({cells} cells)", median(runs));
    }
    if !plain {
        let s = spec(r#","guard":1,"stop_blank_rows":true,"stop_total_rows":true"#);
        let mut runs = Vec::new();
        for _ in 0..7 {
            let t = Instant::now();
            let c = d.count_fill_blanks(&s, &Progress::new()).unwrap();
            runs.push(ms(t));
            assert!(c.counted <= 200_000);
        }
        println!(
            "count line (all three, capped): median {:.1} ms",
            median(runs)
        );
    }
}
