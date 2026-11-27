//! The cross-tab move's own times on the files `xfer_base gen` writes:
//! `cargo run --release --example xfer_bench -- <dir> [copy|move|rows|mem|cancel]`
//! - copy: copy src's column 2 into dst at column 3 (take_cols, plan, apply), median of 5 (G1);
//! - move: the copy plus Delete column in src (G2);
//! - rows: 10,000 rows into dst at the end (G3);
//! - mem: peak working set while copying a column of short values (G5, Windows);
//! - cancel: Cancel at about half of the copy-out; how long it takes to stop, and nothing written (G6).

use natendo_engine::xfer::Wider;
use natendo_engine::{Document, OpenOptions, Progress};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn open(p: &Path) -> Document {
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
    let what = args.get(2).map(String::as_str).unwrap_or("copy");
    let p = Progress::new();
    match what {
        "copy" | "move" => {
            let mut times = Vec::new();
            for _ in 0..5 {
                let mut s = open(&dir.join("src.csv"));
                let mut d = open(&dir.join("dst.csv"));
                let t = Instant::now();
                let out = s.take_cols(&[2], true, &p).unwrap();
                let (plan, _) = d.plan_insert_cols(3, &out, true, &p).unwrap();
                d.apply_join(plan).unwrap();
                if what == "move" {
                    s.delete_cols(2, 1).unwrap();
                }
                times.push(ms(t));
            }
            println!(
                "{what} {:.1} ms (median of 5; {:.1?})",
                median(times.clone()),
                times
            );
        }
        "rows" => {
            let mut times = Vec::new();
            for _ in 0..5 {
                let s = open(&dir.join("src.csv"));
                let mut d = open(&dir.join("dst.csv"));
                let t = Instant::now();
                let rows: Vec<u64> = (1..10_001).collect();
                let out = s.take_rows(&rows, true, &p).unwrap();
                let (plan, _) = d
                    .plan_insert_rows(None, &out, true, Wider::Cut, &p)
                    .unwrap();
                d.apply_join(plan).unwrap();
                times.push(ms(t));
            }
            println!(
                "rows {:.1} ms (median of 5; {:.1?})",
                median(times.clone()),
                times
            );
        }
        "mem" => {
            let s = open(&dir.join("src.csv"));
            let mut d = open(&dir.join("dst.csv"));
            let base = peak_mb();
            let out = s.take_cols(&[0], true, &p).unwrap();
            let held: usize = out.cols.iter().map(|c| c.bytes()).sum();
            let after_take = peak_mb();
            let (plan, _) = d.plan_insert_cols(3, &out, true, &p).unwrap();
            d.apply_join(plan).unwrap();
            println!(
                "mem: peak {base:.0} MB open, {after_take:.0} MB after copy-out (column held {:.0} MB), {:.0} MB after insert",
                held as f64 / 1e6,
                peak_mb()
            );
        }
        "cancel" => {
            let s = open(&dir.join("src.csv"));
            let prog = Arc::new(Progress::new());
            let pr = prog.clone();
            let total = s.row_count();
            let stopper = std::thread::spawn(move || loop {
                if pr.done.load(Ordering::Relaxed) * 2 >= total {
                    let t = Instant::now();
                    pr.cancel.store(true, Ordering::SeqCst);
                    return t;
                }
                std::hint::spin_loop();
            });
            let r = s.take_cols(&[2], true, &prog);
            let at = stopper.join().unwrap();
            println!(
                "cancel: stopped {:.1} ms after Cancel, at {:.0}% ({})",
                ms(at),
                100.0 * prog.done.load(Ordering::Relaxed) as f64 / total as f64,
                if r.is_err() {
                    "nothing copied"
                } else {
                    "FINISHED"
                }
            );
        }
        _ => panic!("copy, move, rows, mem or cancel"),
    }
}
