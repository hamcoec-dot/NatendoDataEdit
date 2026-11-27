//! Filter chips (C8) speed: on 1,000,000 rows of a made-up voter list, a
//! filter of 5 rules (as main does: planned, then applied), and (not
//! `plain`) switching a chip off and on, Match any, Invert all and a tool
//! chip beside the rules, with the bytes the rule results keep.
//! `cargo run --release --example chips_bench -- <file.csv> [plain]`
//! The file is made (made-up data) when it does not exist.
use natendo_engine::document::Progress;
use natendo_engine::filter::FilterSpec;
use natendo_engine::{Document, OpenOptions};
use std::io::Write;
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn make(path: &str) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    writeln!(f, "id,party,town,age,balance,status").unwrap();
    let parties = ["DEM", "REP", "IND", "GRN"];
    let status = ["active", "inactive", "moved"];
    for i in 0..1_000_000u32 {
        writeln!(
            f,
            "{i},{},Town {},{},{}.{:02},{}",
            parties[(i % 7 % 4) as usize],
            i % 20,
            18 + i % 82,
            i % 5000,
            i % 100,
            status[(i % 11 % 3) as usize]
        )
        .unwrap();
    }
}

/// The 5 rules, as JSON (the same on main and on chips).
fn spec() -> FilterSpec {
    serde_json::from_str(
        r#"{"keep_first":true,"columns":[
        {"col":1,"rule":{"type":"values","values":["DEM","REP"],"blanks":false}},
        {"col":2,"rule":{"type":"values","values":["Town 1","Town 2","Town 3","Town 4","Town 5","Town 6","Town 7","Town 8","Town 9","Town 10"],"blanks":false}},
        {"col":3,"rule":{"type":"conditions","first":{"type":"number","op":"gt","value":30}}},
        {"col":4,"rule":{"type":"conditions","first":{"type":"number_between","low":100,"high":4000}}},
        {"col":5,"rule":{"type":"conditions","first":{"type":"text","op":"not_equals","text":"inactive"}}}
        ]}"#,
    )
    .unwrap()
}

fn main() {
    let path = std::env::args().nth(1).expect("a CSV file");
    let plain = std::env::args().nth(2).is_some_and(|a| a == "plain");
    if !std::path::Path::new(&path).exists() {
        make(&path);
    }
    let mut d = Document::open_indexed(path.as_ref(), OpenOptions::default(), None).unwrap();
    let s = spec();
    let mut runs = Vec::new();
    let mut plans = Vec::new();
    let mut shown = 0;
    for k in 0..7 {
        d.clear_filter();
        // An edit first, so no result from before is used (a first build).
        d.set_cell(1, 0, if k % 2 == 0 { "0x" } else { "0" }.into())
            .unwrap();
        let t = Instant::now();
        let plan = d.plan_filter(&s, &Progress::new()).unwrap();
        plans.push(ms(t));
        shown = plan.shown;
        d.apply_filter(plan).unwrap();
        runs.push(ms(t));
    }
    println!("filter of 5 rules, plan and apply: median {:.1} ms ({shown} rows)", median(runs));
    println!("plan alone: median {:.1} ms", median(plans));
    if plain {
        return;
    }
    run_chips(&mut d);
}

fn run_chips(d: &mut Document) {
    use natendo_engine::chips::ChipChange;
    use natendo_engine::filter::Combine;
    let time = |d: &mut Document, name: &str, changes: &[ChipChange]| {
        let mut runs = Vec::new();
        for _ in 0..5 {
            for c in changes {
                let t = Instant::now();
                d.change_filter(c).unwrap();
                runs.push(ms(t));
            }
        }
        println!(
            "{name}: median {:.1} ms, slowest {:.1} ms",
            median(runs.clone()),
            runs.iter().cloned().fold(0.0, f64::max)
        );
    };
    time(
        d,
        "switch a chip off and on",
        &[ChipChange::Toggle { index: 2 }, ChipChange::Toggle { index: 2 }],
    );
    time(
        d,
        "Match any and back",
        &[
            ChipChange::Combine {
                combine: Combine::Or,
            },
            ChipChange::Combine {
                combine: Combine::And,
            },
        ],
    );
    time(
        d,
        "Invert all and back",
        &[ChipChange::Invert { on: true }, ChipChange::Invert { on: false }],
    );
    time(
        d,
        "invert one chip and back",
        &[ChipChange::Negate { index: 0 }, ChipChange::Negate { index: 0 }],
    );
    let ids: Vec<u32> = (1..1_000_000u32).step_by(3).collect();
    let t = Instant::now();
    let plan = d.plan_show_ids(&ids, true);
    d.apply_rows(plan).unwrap();
    println!("a tool chip beside the rules: {:.1} ms", ms(t));
    time(
        d,
        "switch the tool chip off and on",
        &[ChipChange::Toggle { index: 5 }, ChipChange::Toggle { index: 5 }],
    );
    let c = d.filter_chips();
    println!("chips: {} shown of {}", c.shown, c.total);
}
