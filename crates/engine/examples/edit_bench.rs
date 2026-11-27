//! C8 review M1 probe (bench only): an edit with a 5-rule filter on, then what the chip bar asks
//! (`filter_chips`; main has no bar). `cargo run --release --example edit_bench -- <file.csv>`
use natendo_engine::document::Progress;
use natendo_engine::filter::FilterSpec;
use natendo_engine::{Document, OpenOptions};
use std::io::Write;
use std::time::Instant;

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
    if !std::path::Path::new(&path).exists() {
        make(&path);
    }
    let mut d = Document::open_indexed(path.as_ref(), OpenOptions::default(), None).unwrap();
    let _ = Progress::new();
    d.set_filter(&spec()).unwrap();
    let mut runs = Vec::new();
    for k in 0..41u32 {
        let t = Instant::now();
        d.set_cell(1 + u64::from(k % 50), 0, format!("{k}x"))
            .unwrap();
        std::hint::black_box(d.filter_chips());
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        if k > 0 {
            runs.push(ms);
        }
    }
    runs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "edit (+ bar) median {:.3} ms, p90 {:.3} ms",
        runs[runs.len() / 2],
        runs[runs.len() * 9 / 10]
    );
}
