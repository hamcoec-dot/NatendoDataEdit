//! File contracts (C9) speed and memory, on a file `contract_base gen` wrote:
//! `cargo run --release --example contract_bench -- <file.csv> <drift|plain|match|cancel|mem>`
//! - drift: the drift pass with types (amount, joined), allowed values (party, status), blank shares on
//!   all 10 columns and the contract's two rules (compare with `contract_base checks` on main);
//! - plain: the same without rules (compare with `contract_base tally` on main);
//! - match: reading 20 contract files and recognising a file among them;
//! - cancel: how long a drift pass takes to stop once cancelled (an edit cancels it);
//! - mem: the process's memory before and after a drift pass (Windows working set).

use natendo_engine::checkfile::ChecksFile;
use natendo_engine::coltype::ColumnType;
use natendo_engine::contract::{pair, recognise, Contract, DraftSpec, DriftOptions};
use natendo_engine::validate::{Check, CheckKind};
use natendo_engine::{Document, OpenOptions, Progress};
use std::sync::Arc;
use std::time::Instant;

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

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

fn rules() -> Vec<Check> {
    vec![
        Check::new(Some(5), CheckKind::Required),
        Check::new(
            Some(7),
            serde_json::from_str(r#"{"type":"number"}"#).expect("a number rule"),
        ),
    ]
}

fn contract(d: &Document, with_rules: bool) -> Contract {
    let mut c = d
        .contract_draft(
            &DraftSpec {
                name: "Monthly voters".into(),
                keep_first: true,
                allowed_cols: vec![3, 8],
                ..Default::default()
            },
            &Progress::new(),
        )
        .unwrap()
        .contract;
    c.columns[7].ty = Some(ColumnType::Number);
    c.columns[6].ty = Some(ColumnType::Date(None));
    if with_rules {
        c.checks = Some(ChecksFile::for_rules(d, &rules(), true));
    }
    c
}

fn opts() -> DriftOptions {
    DriftOptions {
        keep_first: true,
        ..Default::default()
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let d = Document::open_indexed(args[0].as_ref(), OpenOptions::default(), None).unwrap();
    let titles = d.rule_titles(true);
    match args[1].as_str() {
        mode @ ("drift" | "plain") => {
            let c = contract(&d, mode == "drift");
            let p = pair(&c, &titles);
            let mut times = Vec::new();
            for _ in 0..5 {
                let t = Instant::now();
                let r = d.drift(&c, &p, &opts(), &Progress::new()).unwrap();
                times.push(t.elapsed().as_secs_f64() * 1000.0);
                std::hint::black_box(r);
            }
            let m = median(times.clone());
            println!("{mode}: median {m:.1} ms ({times:.0?})");
        }
        "match" => {
            let base = contract(&d, true);
            let dir = std::env::temp_dir().join("nd-contract-bench");
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            for i in 0..20 {
                let mut c = base.clone();
                c.name = format!("Contract {i}");
                c.recognise.pattern = Some(format!("export_{i}_*.csv"));
                for col in c.columns.iter_mut() {
                    col.title = format!(
                        "{}{}",
                        col.title,
                        if i == 19 {
                            String::new()
                        } else {
                            format!("_{i}")
                        }
                    );
                }
                std::fs::write(
                    dir.join(format!("Contract {i}.ndcontract")),
                    c.to_bytes().unwrap(),
                )
                .unwrap();
            }
            let t = Instant::now();
            let list: Vec<Contract> = std::fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .map(|e| Contract::from_bytes(&std::fs::read(e.path()).unwrap()).unwrap())
                .collect();
            let load = t.elapsed().as_secs_f64() * 1000.0;
            let t = Instant::now();
            let n = 1000;
            let mut found = 0;
            for _ in 0..n {
                found += recognise(&list, Some("voters_feb.csv"), Some(&titles)).len();
            }
            let each = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
            println!("match: reading 20 contracts {load:.2} ms; recognising a file among them {each:.3} ms ({} found)", found / n);
            let _ = std::fs::remove_dir_all(&dir);
        }
        "cancel" => {
            let c = contract(&d, true);
            let p = pair(&c, &titles);
            let d = Arc::new(d);
            let mut times = Vec::new();
            for _ in 0..5 {
                let progress = Arc::new(Progress::new());
                let (d2, c2, p2, pr2) = (d.clone(), c.clone(), p.clone(), progress.clone());
                let h = std::thread::spawn(move || d2.drift(&c2, &p2, &opts(), &pr2).is_err());
                std::thread::sleep(std::time::Duration::from_millis(50));
                let t = Instant::now();
                progress
                    .cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                let stopped = h.join().unwrap();
                assert!(
                    stopped,
                    "the pass ended before the cancel: use a bigger file"
                );
                times.push(t.elapsed().as_secs_f64() * 1000.0);
            }
            println!(
                "cancel: stops in median {:.1} ms ({times:.1?})",
                median(times.clone())
            );
        }
        "mem" => {
            let c = contract(&d, true);
            let p = pair(&c, &titles);
            let (before, peak0) = mem();
            let t = Instant::now();
            let r = d.drift(&c, &p, &opts(), &Progress::new()).unwrap();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            let (after, peak) = mem();
            println!(
                "mem: {} rows; working set {before:.0} MB before, {after:.0} MB after (peak {peak0:.0} then {peak:.0} MB); drift {ms:.0} ms; report {} bytes as JSON",
                r.rows,
                serde_json::to_string(&r).unwrap().len()
            );
        }
        _ => panic!("drift, plain, match, cancel or mem"),
    }
}
