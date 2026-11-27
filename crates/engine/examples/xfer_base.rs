//! The baselines of the cross-tab move speed gates, with APIs main has too, so the same file builds on
//! main and on the branch and the two can be timed side by side:
//! `cargo run --release --example xfer_base -- <dir> <gen|insert|paste|delete> [rows]`
//! - gen: write `src.csv` and `dst.csv` (made-up, 10 columns) of `rows` rows (1,000,000 by default);
//! - insert: an empty Insert column in dst (median of 5);
//! - paste: main's way to copy a column across: read src's column 2, insert a column in dst, paste it;
//! - delete: Delete column in src (a move's second half on main);
//! - compute: a new computed column `[zip]` in dst (a plain column of 1M written values on main);
//! - computemem: its peak working set (Windows), once.

use natendo_engine::ops::PasteSpec;
use natendo_engine::{Document, OpenOptions, Progress};
use std::io::Write;
use std::path::Path;
use std::time::Instant;

pub fn gen(dir: &Path, rows: u32) {
    for (name, salt) in [("src.csv", 1u32), ("dst.csv", 7)] {
        let mut f = std::io::BufWriter::new(std::fs::File::create(dir.join(name)).unwrap());
        writeln!(f, "id,first,city,zip,phone,party,amount,joined,status,town").unwrap();
        for i in 0..rows {
            let k = i.wrapping_mul(salt);
            writeln!(
                f,
                "{i},N{},City {},{:05},555-{:04},P{},{}.{:02},2026-{:02}-{:02},s{},T{}",
                k % 97,
                k % 300,
                k % 99_999,
                k % 10_000,
                k % 5,
                k % 5000,
                k % 100,
                1 + k % 12,
                1 + k % 28,
                k % 3,
                k % 40
            )
            .unwrap();
        }
    }
}

pub fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

pub fn open(p: &Path) -> Document {
    Document::open_indexed(p, OpenOptions::default(), None).unwrap()
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

#[cfg(windows)]
fn peak_mb() -> f64 {
    #[repr(C)]
    #[allow(non_snake_case)]
    struct Pmc {
        cb: u32,
        PageFaultCount: u32,
        PeakWorkingSetSize: usize,
        WorkingSetSize: usize,
        a: usize,
        b: usize,
        c: usize,
        d: usize,
        PagefileUsage: usize,
        PeakPagefileUsage: usize,
    }
    #[link(name = "psapi")]
    extern "system" {
        fn GetProcessMemoryInfo(h: isize, p: *mut Pmc, cb: u32) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
    }
    let mut p: Pmc = unsafe { std::mem::zeroed() };
    p.cb = std::mem::size_of::<Pmc>() as u32;
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut p, p.cb) };
    p.PeakWorkingSetSize as f64 / 1e6
}
#[cfg(not(windows))]
fn peak_mb() -> f64 {
    0.0
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = Path::new(&args[1]);
    let what = args.get(2).map(String::as_str).unwrap_or("insert");
    if what == "gen" {
        gen(
            dir,
            args.get(3)
                .and_then(|s| s.parse().ok())
                .unwrap_or(1_000_000),
        );
        return;
    }
    if what == "computemem" {
        let mut d = open(&dir.join("dst.csv"));
        let base = peak_mb();
        let spec = natendo_engine::expr::ComputeSpec {
            expr: "[id]".into(),
            title: "id2".into(),
            at: Some(3),
            keep_first: true,
            hidden: true,
            ..Default::default()
        };
        let (plan, _) = d.plan_compute(&spec, &Progress::new()).unwrap();
        d.apply_cols(plan).unwrap();
        println!(
            "computemem: peak {base:.0} MB open, {:.0} MB after the computed column",
            peak_mb()
        );
        return;
    }
    let mut times = Vec::new();
    for _ in 0..5 {
        match what {
            "insert" => {
                let mut d = open(&dir.join("dst.csv"));
                let t = Instant::now();
                d.insert_cols(3, 1).unwrap();
                times.push(ms(t));
            }
            "paste" => {
                let s = open(&dir.join("src.csv"));
                let mut d = open(&dir.join("dst.csv"));
                let t = Instant::now();
                let grid: Vec<Vec<String>> = (0..s.row_count())
                    .map(|r| vec![s.cell(r, 2).unwrap_or_default()])
                    .collect();
                d.insert_cols(3, 1).unwrap();
                let spec = PasteSpec {
                    row: 0,
                    col: 3,
                    ..Default::default()
                };
                let plan = d.plan_paste(&spec, grid, &Progress::new()).unwrap();
                d.apply_cells(plan).unwrap();
                times.push(ms(t));
            }
            "compute" => {
                let mut d = open(&dir.join("dst.csv"));
                let t = Instant::now();
                let spec = natendo_engine::expr::ComputeSpec {
                    expr: "[zip]".into(),
                    title: "zip2".into(),
                    at: Some(3),
                    keep_first: true,
                    hidden: true,
                    ..Default::default()
                };
                let (plan, _) = d.plan_compute(&spec, &Progress::new()).unwrap();
                d.apply_cols(plan).unwrap();
                times.push(ms(t));
            }
            "delete" => {
                let mut s = open(&dir.join("src.csv"));
                let t = Instant::now();
                s.delete_cols(2, 1).unwrap();
                times.push(ms(t));
            }
            _ => panic!("gen, insert, paste or delete"),
        }
    }
    println!(
        "{what} {:.1} ms (median of 5; {:.1?})",
        median(times.clone()),
        times
    );
}
