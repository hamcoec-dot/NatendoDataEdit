//! Time and peak heap of "what changed since this checkpoint" on a 1M-row file after one big change: Replace all over
//! every row, a Clean-up of a column, and Remove duplicates removing about half the rows (B4, part 2).
//!
//! ```text
//! cargo run --release -p natendo-engine --example since_bench -- [rows] [repeats]
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use natendo_engine::clean::{CleanSpec, Cleaner, Spaces, Step};
use natendo_engine::find::{FindSpec, Pattern, PatternKind, Scope};
use natendo_engine::reshape::Duplicates;
use natendo_engine::{Document, OpenOptions, Progress};

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
    let repeats: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(3);
    let dir = std::env::temp_dir().join("nde-since-bench");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("sb-{rows}.csv"));
    if !path.exists() {
        let mut w = BufWriter::with_capacity(1 << 20, std::fs::File::create(&path).unwrap());
        writeln!(w, "id,name,amount,city\r").unwrap();
        for i in 0..rows {
            // Half the rows repeat the id of the row before (so Remove duplicates on column 0 removes half).
            let k = if i % 2 == 1 { i - 1 } else { i };
            write!(
                w,
                "{k},  name {}  ,{}.{:02},City {}\r\n",
                k % 9973,
                k % 1000,
                k % 100,
                k % 311
            )
            .unwrap();
        }
        w.flush().unwrap();
    }
    let mut d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
    println!("{rows} rows; one big change after a checkpoint, then the diff since it.");
    let p = Progress::new();
    let report = |what: &str, d: &Document, mark: u64, wm: u32| {
        let mut best = f64::MAX;
        let mut plan = None;
        let mut peak_extra = 0usize;
        for _ in 0..repeats {
            let live0 = LIVE.load(Ordering::Relaxed);
            PEAK.store(live0, Ordering::Relaxed);
            let t = Instant::now();
            let r = d.since_plan(mark, wm, &p).unwrap();
            best = best.min(t.elapsed().as_secs_f64() * 1000.0);
            peak_extra = peak_extra.max(PEAK.load(Ordering::Relaxed) - live0);
            plan = Some(r);
        }
        let plan = plan.unwrap();
        let t = Instant::now();
        let page = d.since_page(&plan, mark, wm, 0, 50).unwrap();
        let first_page = t.elapsed().as_secs_f64() * 1000.0;
        let t = Instant::now();
        let far = d
            .since_page(
                &plan,
                mark,
                wm,
                plan.totals.cells_changed.saturating_sub(50),
                50,
            )
            .unwrap();
        let last_page = t.elapsed().as_secs_f64() * 1000.0;
        let tt = &plan.totals;
        println!(
            "{what}: plan best {best:.0} ms, peak extra heap {:.1} MB; first page {first_page:.0} ms ({} cells), last page {last_page:.0} ms ({} cells); {} rows changed, {} cells, {} rows removed, {} steps",
            peak_extra as f64 / 1048576.0,
            page.len(),
            far.len(),
            tt.rows_changed,
            tt.cells_changed,
            tt.rows_removed,
            tt.steps
        );
    };
    // Replace all over every row: every cell with a letter n gets an N... (every row changes).
    let (mark, wm) = (d.step_mark(), d.next_row_id());
    let spec = FindSpec {
        pattern: Pattern {
            text: "name".into(),
            kind: PatternKind::Plain,
            ..Default::default()
        },
        scope: Scope::All,
        skip_first: true,
        ..Default::default()
    };
    let t = Instant::now();
    d.replace_all(&spec, "NAME").unwrap();
    println!(
        "(Replace all itself took {:.0} ms)",
        t.elapsed().as_secs_f64() * 1000.0
    );
    report("Replace all over every row", &d, mark, wm);
    assert!(d.go_to_mark(mark));
    d.drop_redo();
    // A Clean-up of one column (trim the spaces).
    let (mark, wm) = (d.step_mark(), d.next_row_id());
    let cs = CleanSpec {
        cleaner: Cleaner {
            version: 1,
            name: String::new(),
            steps: vec![Step::Spaces { how: Spaces::Trim }],
        },
        scope: Scope::Columns { cols: vec![1] },
        skip_first: true,
        read: Default::default(),
    };
    let t = Instant::now();
    let plan = d.plan_clean(&cs, 0.0, 1, 0, &p).unwrap();
    d.apply_clean(plan).unwrap();
    println!(
        "(Clean-up itself took {:.0} ms)",
        t.elapsed().as_secs_f64() * 1000.0
    );
    report("Clean-up of one column", &d, mark, wm);
    assert!(d.go_to_mark(mark));
    d.drop_redo();
    // Remove duplicates removing about half the rows.
    let (mark, wm) = (d.step_mark(), d.next_row_id());
    let dup = Duplicates {
        cols: vec![0],
        keep_first: true,
        ..Default::default()
    };
    let t = Instant::now();
    let plan = d.plan_remove_duplicates(&dup, &p).unwrap();
    d.apply_rows(plan).unwrap();
    println!(
        "(Remove duplicates itself took {:.0} ms; {} rows left)",
        t.elapsed().as_secs_f64() * 1000.0,
        d.row_count()
    );
    report("Remove duplicates (about half the rows)", &d, mark, wm);
}
