//! SPIKE (read-only, not for merge): time and memory of "what changed since this checkpoint" worked out from the
//! undo steps, on a 1M-row file, for 1, 100 and 1,000 steps (cell edits, a delete, inserts, sorts).
//!
//! ```text
//! cargo run --release -p natendo-engine --example since_spike -- [rows] [repeats]
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use natendo_engine::sort::{SortKey, SortSpec};
use natendo_engine::{Document, OpenOptions};

/// Counts live and peak heap bytes so the spike's own memory can be reported.
struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            let now = LIVE.fetch_add(l.size(), Ordering::Relaxed) + l.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l);
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = System.realloc(p, l, new);
        if !q.is_null() {
            if new >= l.size() {
                let now = LIVE.fetch_add(new - l.size(), Ordering::Relaxed) + (new - l.size());
                PEAK.fetch_max(now, Ordering::Relaxed);
            } else {
                LIVE.fetch_sub(l.size() - new, Ordering::Relaxed);
            }
        }
        q
    }
}
#[global_allocator]
static A: Counting = Counting;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rows: u64 = args
        .first()
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);
    let repeats: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(5);
    let dir = std::env::temp_dir().join("nde-since-spike");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("s-{rows}.csv"));
    if !path.exists() {
        let mut w = BufWriter::with_capacity(1 << 20, std::fs::File::create(&path).unwrap());
        writeln!(w, "id,name,amount,city\r").unwrap();
        for i in 0..rows {
            write!(
                w,
                "{i},name {},{}.{:02},City {}\r\n",
                i % 9973,
                i % 1000,
                i % 100,
                i % 311
            )
            .unwrap();
        }
        w.flush().unwrap();
    }
    let mut d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
    println!("{rows} rows. Per scenario: steps made after the checkpoint, then the diff since it.");
    let mut x: u64 = 99;
    let mut rnd = move |n: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x % n
    };
    for steps in [1usize, 100, 1000] {
        let hist_before = d.history_bytes();
        let mark = d.step_mark();
        let wm = d.next_row_id();
        let hist_marked = d.history_bytes();
        let mut deletes = 0;
        let mut inserts = 0;
        let mut sorts = 0;
        for s in 0..steps {
            let n = d.row_count();
            if steps >= 100 && s % 500 == 50 {
                d.sort(&SortSpec {
                    keys: vec![SortKey {
                        col: 3,
                        descending: false,
                        kind: Default::default(),
                        match_case: false,
                    }],
                    keep_first: true,
                    ..Default::default()
                })
                .unwrap();
                sorts += 1;
            } else if steps >= 100 && s % 10 == 3 {
                d.delete_rows(1 + rnd(n - 1), 1).unwrap();
                deletes += 1;
            } else if steps >= 100 && s % 20 == 7 {
                d.insert_rows(1 + rnd(n - 1), vec![vec!["new".into(), "x".into(), "1".into(), "y".into()]])
                    .unwrap();
                inserts += 1;
            } else {
                d.set_cell(1 + rnd(n - 1), (1 + rnd(3)) as u32, format!("e{s}"))
                    .unwrap();
            }
        }
        let hist_after = d.history_bytes();
        let mut best = f64::MAX;
        let mut res = None;
        let mut peak_extra = 0usize;
        for _ in 0..repeats {
            let live0 = LIVE.load(Ordering::Relaxed);
            PEAK.store(live0, Ordering::Relaxed);
            let t = Instant::now();
            let r = d.spike_changes_since(mark, wm).unwrap();
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
            peak_extra = peak_extra.max(PEAK.load(Ordering::Relaxed) - live0);
            res = Some(r);
        }
        let r = res.unwrap();
        println!(
            "{steps:>5} steps ({deletes} deletes, {inserts} inserts, {sorts} sorts): diff best {best:.2} ms; peak extra heap {:.0} KB; touched rows {}, rows changed {}, cells changed {}, rows added {}, removed {}, reordered {}",
            peak_extra as f64 / 1024.0,
            r.touched_rows,
            r.rows_changed,
            r.cells_changed,
            r.rows_added,
            r.rows_removed,
            r.reordered
        );
        println!(
            "        history bytes: before the mark {hist_before}, with the mark sitting {hist_marked} (a mark copies nothing), after the steps {hist_after}"
        );
        // Back to the start for the next scenario.
        assert!(d.go_to_mark(mark));
        d.drop_redo();
    }
}
