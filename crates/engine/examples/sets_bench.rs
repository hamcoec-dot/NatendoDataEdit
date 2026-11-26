//! Speed of Compare lists beside five value counts of the same columns, on
//! made-up rows, alternating, best of 5:
//! `cargo run --release -p natendo-engine --example sets_bench`

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use natendo_engine::counts::CountsSpec;
use natendo_engine::filter::{ColumnFilter, FilterSpec, Rule};
use natendo_engine::find::{FindSpec, Pattern, Scope};
use natendo_engine::sets::{compare_lists, SetList, SetsBuilder, SetsSpec};
use natendo_engine::{Document, OpenOptions, Progress};

const ROWS: u64 = 1_000_000;
const LISTS: u64 = 5;
const ROUNDS: usize = 5;

/// List `i` holds a run of keys; the runs overlap so there are `distinct` keys
/// in all.
fn table(i: u64, distinct: u64) -> Document {
    let mut s = String::from("Voter,Ward\r\n");
    let step = (distinct - ROWS.min(distinct)) / (LISTS - 1);
    for r in 0..ROWS {
        let k = if distinct >= ROWS * LISTS {
            i * ROWS + r
        } else {
            i * step + (r * 7_919) % ROWS.min(distinct)
        };
        s.push_str(&format!("V-{k:09},{}\r\n", r % 40));
    }
    Document::from_bytes(s.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
}

fn time(f: impl FnOnce()) -> Duration {
    let t = Instant::now();
    f();
    t.elapsed()
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn main() {
    let spec = SetsSpec {
        lists: (0..LISTS)
            .map(|_| SetList {
                cols: vec![0],
                header: true,
            })
            .collect(),
        ..Default::default()
    };
    for distinct in [3_000_000, ROWS * LISTS] {
        let docs: Vec<Document> = (0..LISTS).map(|i| table(i, distinct)).collect();
        let refs: Vec<&Document> = docs.iter().collect();
        let counts = CountsSpec {
            cols: vec![0],
            keep_first: true,
        };
        let (mut whole, mut counted) = (Duration::MAX, Duration::MAX);
        let (mut reads, mut tally) = (Duration::MAX, Duration::MAX);
        let (mut keys, mut bytes) = (0, 0);
        for _ in 0..ROUNDS {
            counted = counted.min(time(|| {
                for d in &docs {
                    d.value_counts(&counts, &Progress::new()).unwrap();
                }
            }));
            let p = Progress::new();
            let mut read = Duration::ZERO;
            let mut count = Duration::ZERO;
            whole = whole.min(time(|| {
                let mut b = SetsBuilder::new(&spec, vec![false]).unwrap();
                read = time(|| {
                    for d in &docs {
                        b.add(d, &p).unwrap();
                    }
                });
                (keys, bytes) = b.used();
                count = time(|| {
                    b.finish(&p).unwrap();
                });
            }));
            reads = reads.min(read);
            tally = tally.min(count);
        }
        println!("{LISTS} lists x {ROWS} rows, {keys} different keys");
        println!(
            "five value_counts {:.0} ms; comparison {:.0} ms (reading {:.0}, counting {:.0}): ratio {:.3} (gate 1.50)",
            ms(counted),
            ms(whole),
            ms(reads),
            ms(tally),
            ms(whole) / ms(counted)
        );
        println!("keys charged {} MB", bytes >> 20);
        let r = compare_lists(&refs, &spec, &Progress::new()).unwrap();
        let mask = r
            .report
            .slices
            .iter()
            .max_by_key(|s| s.rows[0])
            .unwrap()
            .mask;
        // Like for like: today's way to put the same number of rows in a new
        // tab is to filter, copy the shown rows and open them.
        let mut base = table(0, distinct);
        let (mut open, mut today) = (Duration::MAX, Duration::MAX);
        let mut slice_rows = 0;
        for _ in 0..ROUNDS {
            // Rows are numbered 0 to 39 in turn: the wards below `wards`
            // are as many rows as the slice holds.
            let rows_in_slice = r.report.slices[usize::from(mask) - 1].rows[0];
            let wards = (rows_in_slice * 40).div_ceil(ROWS);
            let filter = FilterSpec {
                keep_first: true,
                columns: vec![ColumnFilter {
                    col: 1,
                    rule: Rule::Values {
                        values: (0..wards).map(|w| w.to_string()).collect(),
                        blanks: false,
                    },
                }],
                ..Default::default()
            };
            let every = FindSpec {
                pattern: Pattern {
                    text: "V-".into(),
                    ..Default::default()
                },
                scope: Scope::Columns { cols: vec![0] },
                skip_first: true,
                ..Default::default()
            };
            base.clear_filter();
            let mut copied = 0;
            today = today.min(time(|| {
                let plan = base.plan_filter(&filter, &Progress::new()).unwrap();
                base.apply_filter(plan).unwrap();
                let t = base.matching_table(&every, &Progress::new()).unwrap();
                copied = t.rows;
                t.into_document().unwrap();
            }));
            open = open.min(time(|| {
                let t = docs[0]
                    .sets_slice_table(&spec.lists[0], &r.row_masks[0], mask, &Progress::new())
                    .unwrap();
                slice_rows = t.rows;
                t.into_document().unwrap();
            }));
            assert!(
                copied.abs_diff(slice_rows) < ROWS / 30,
                "{copied} {slice_rows}"
            );
        }
        println!(
            "open a slice of {slice_rows} rows {:.0} ms; filter, copy the shown rows and open them {:.0} ms: ratio {:.2} (gate 1.20)",
            ms(open),
            ms(today),
            ms(open) / ms(today)
        );
        let mut worst = Duration::ZERO;
        for part in 1..10 {
            let progress = Progress::new();
            std::thread::scope(|s| {
                let at = whole.mul_f64(f64::from(part) / 10.0);
                let progress = &progress;
                let h = s.spawn(move || {
                    std::thread::sleep(at);
                    progress.cancel.store(true, Ordering::Relaxed);
                    Instant::now()
                });
                let _ = compare_lists(&refs, &spec, progress);
                let ended = Instant::now();
                worst = worst.max(ended.saturating_duration_since(h.join().unwrap()));
            });
        }
        println!("slowest stop after Cancel {:.0} ms", ms(worst));
    }
}
