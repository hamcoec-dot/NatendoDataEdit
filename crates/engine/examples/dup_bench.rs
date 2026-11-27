//! Duplicate tab's own times on the file `dup_base gen` writes:
//! `cargo run --release --example dup_bench -- <dir> <dup|cancel|mem>`
//! - dup: duplicate the open file (median of 5), against `dup_base open` (S1);
//! - cancel: Cancel at about half of the copy; how long it takes to stop, and nothing kept;
//! - mem: the extra peak working set of one copy (Windows), against what the app's memory check counts.

use natendo_engine::{Document, OpenOptions, Progress};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

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
    let what = args.get(2).map(String::as_str).unwrap_or("dup");
    let d = Document::open_indexed(&dir.join("big.csv"), OpenOptions::default(), None).unwrap();
    match what {
        "dup" => {
            let mut times = Vec::new();
            for _ in 0..5 {
                let t = Instant::now();
                let c = d.duplicate(&Progress::new()).unwrap();
                times.push(t.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(c.row_count(), d.row_count());
            }
            let mut s = times.clone();
            s.sort_by(f64::total_cmp);
            println!("dup {:.1} ms (median of 5; {:.1?})", s[2], times);
        }
        "cancel" => {
            let prog = Arc::new(Progress::new());
            let pr = prog.clone();
            let stopper = std::thread::spawn(move || loop {
                let total = pr.total.load(Ordering::Relaxed);
                if total > 0 && pr.done.load(Ordering::Relaxed) * 2 >= total {
                    let t = Instant::now();
                    pr.cancel.store(true, Ordering::SeqCst);
                    return t;
                }
                std::hint::spin_loop();
            });
            let r = d.duplicate(&prog);
            let at = stopper.join().unwrap();
            println!(
                "cancel: stopped {:.1} ms after Cancel ({})",
                at.elapsed().as_secs_f64() * 1000.0,
                if r.is_err() {
                    "nothing kept"
                } else {
                    "FINISHED"
                }
            );
        }
        "mem" => {
            let base = peak_mb();
            let need = d.duplicate_bytes() as f64 / 1e6;
            let c = d.duplicate(&Progress::new()).unwrap();
            println!(
                "mem: peak {base:.0} MB open, {:.0} MB with the copy: +{:.0} MB (the app's check counts {need:.0} MB)",
                peak_mb(),
                peak_mb() - base
            );
            drop(c);
        }
        _ => panic!("dup, cancel or mem"),
    }
}
