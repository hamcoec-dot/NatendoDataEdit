//! Filter chips (C8) memory and speed on a big file (10,000,000 rows by
//! default, made-up voters): a filter of 50 rules, each switched off and on
//! (so each rule's result is read and kept), then 20 chips made by a tool.
//! Prints the process's memory (Windows: working set, read with PowerShell)
//! after each phase, and the time of each switch. The kept results are
//! capped at 64 MB, tool chips included, so memory must stay near that.
//! `cargo run --release --example chips_mem -- <file.csv> [rows]`
use natendo_engine::chips::ChipChange;
use natendo_engine::document::Progress;
use natendo_engine::filter::FilterSpec;
use natendo_engine::{Document, OpenOptions};
use std::io::Write;
use std::time::Instant;

fn mem() -> (f64, f64) {
    let pid = std::process::id();
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("$p = Get-Process -Id {pid}; \"$($p.WorkingSet64) $($p.PeakWorkingSet64)\""),
        ])
        .output()
        .expect("powershell");
    let s = String::from_utf8_lossy(&out.stdout);
    let mut it = s
        .split_whitespace()
        .map(|x| x.parse::<f64>().unwrap_or(0.0) / 1e6);
    (it.next().unwrap_or(0.0), it.next().unwrap_or(0.0))
}

fn make(path: &str, rows: u32) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    writeln!(f, "id,party,town,age,balance,status").unwrap();
    let parties = ["DEM", "REP", "IND", "GRN"];
    let status = ["active", "inactive", "moved"];
    for i in 0..rows {
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

fn main() {
    let path = std::env::args().nth(1).expect("a CSV file");
    let rows: u32 = std::env::args()
        .nth(2)
        .map_or(10_000_000, |s| s.parse().unwrap());
    if !std::path::Path::new(&path).exists() {
        make(&path, rows);
    }
    let mut d = Document::open_indexed(path.as_ref(), OpenOptions::default(), None).unwrap();
    let (w, p) = mem();
    println!(
        "opened {} rows: working set {w:.0} MB, peak {p:.0} MB",
        d.total_rows()
    );
    // 50 rules: age over 18..67 (each a chip of its own, all on).
    let cols: Vec<String> = (0..50)
        .map(|k| format!(r#"{{"col":3,"rule":{{"type":"conditions","first":{{"type":"number","op":"gt","value":{}}}}}}}"#, 18 + k))
        .collect();
    let s: FilterSpec = serde_json::from_str(&format!(
        r#"{{"keep_first":true,"columns":[{}]}}"#,
        cols.join(",")
    ))
    .unwrap();
    let t = Instant::now();
    let plan = d.plan_filter(&s, &Progress::new()).unwrap();
    let shown = plan.shown;
    d.apply_filter(plan).unwrap();
    let (w, p) = mem();
    println!(
        "filter of 50 rules: {:.0} ms, {shown} rows; working set {w:.0} MB, peak {p:.0} MB",
        t.elapsed().as_secs_f64() * 1000.0
    );
    let mut first = Vec::new();
    let mut again = Vec::new();
    for i in 0..50 {
        let t = Instant::now();
        d.change_filter(&ChipChange::Toggle { index: i }).unwrap();
        d.change_filter(&ChipChange::Toggle { index: i }).unwrap();
        first.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    for i in 0..50 {
        let t = Instant::now();
        d.change_filter(&ChipChange::Toggle { index: i }).unwrap();
        d.change_filter(&ChipChange::Toggle { index: i }).unwrap();
        again.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let stat = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        (v[v.len() / 2], v[v.len() - 1])
    };
    let (m1, x1) = stat(&mut first);
    let (m2, x2) = stat(&mut again);
    let (w, p) = mem();
    println!("each of 50 chips off and on, first time: median {m1:.0} ms, slowest {x1:.0} ms; again: median {m2:.0} ms, slowest {x2:.0} ms");
    println!("after 50 chips switched: working set {w:.0} MB, peak {p:.0} MB");
    let n = d.total_rows() as u32;
    let mut tool = Vec::new();
    for k in 0..20u32 {
        let ids: Vec<u32> = (1..n).filter(|i| i % (k + 2) == 0).collect();
        let t = Instant::now();
        let plan = d.plan_show_ids(&ids, true);
        d.apply_rows(plan).unwrap();
        tool.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let refused = d.plan_show_ids(&[1, 2, 3], true);
    let full = d.apply_rows(refused).is_err();
    let (m3, x3) = stat(&mut tool);
    let (w, p) = mem();
    println!("20 tool chips: median {m3:.0} ms, slowest {x3:.0} ms; a 21st refused: {full}");
    println!("after 50 rule chips and 20 tool chips: working set {w:.0} MB, peak {p:.0} MB");
    let mut sw = Vec::new();
    for i in 0..70 {
        let t = Instant::now();
        d.change_filter(&ChipChange::Toggle { index: i }).unwrap();
        d.change_filter(&ChipChange::Toggle { index: i }).unwrap();
        sw.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let (m4, x4) = stat(&mut sw);
    let (w, p) = mem();
    let c = d.filter_chips();
    println!("each of 70 chips off and on: median {m4:.0} ms, slowest {x4:.0} ms; {} chips, {} rows shown", c.chips.len(), c.shown);
    println!("at the end: working set {w:.0} MB, peak {p:.0} MB");
}
