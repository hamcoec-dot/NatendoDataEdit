use super::*;
use crate::clean::{CaseTo, CleanSpec, Cleaner, Step};
use crate::document::since::{SinceKind, SinceTotals};
use crate::find::Scope;
use crate::sort::{SortKey, SortSpec};
use crate::SaveOptions;
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Act {
    Edit(u64, u32, u8),
    Insert(u64, u8),
    Delete(u64, u8),
    Move(u64, u8, u64),
    Sort(u32, bool),
    Hide(u64, u8),
    Upper(u32),
    Undo,
    Redo,
}

fn act() -> impl Strategy<Value = Act> {
    prop_oneof![
        4 => (0..40u64, 0..3u32, 0..20u8).prop_map(|(r, c, v)| Act::Edit(r, c, v)),
        1 => (0..40u64, 1..4u8).prop_map(|(a, n)| Act::Insert(a, n)),
        1 => (0..40u64, 1..4u8).prop_map(|(a, n)| Act::Delete(a, n)),
        1 => (0..40u64, 1..4u8, 0..40u64).prop_map(|(a, n, t)| Act::Move(a, n, t)),
        1 => (0..3u32, any::<bool>()).prop_map(|(c, d)| Act::Sort(c, d)),
        1 => (0..40u64, 1..4u8).prop_map(|(a, n)| Act::Hide(a, n)),
        1 => (0..3u32).prop_map(Act::Upper),
        2 => Just(Act::Undo),
        1 => Just(Act::Redo),
    ]
}

fn apply(d: &mut Document, a: &Act) {
    let n = d.row_count();
    let at = |r: u64| if n <= 1 { 1 } else { 1 + r % (n - 1) };
    let _ = match a {
        Act::Edit(r, c, v) if n > 1 => d.set_cell(at(*r), *c, format!("v{v}")),
        Act::Insert(r, k) => {
            let rows = (0..*k)
                .map(|i| vec![format!("new{i}"), "b".into(), "c".into()])
                .collect();
            d.insert_rows(at(*r).min(n), rows)
        }
        Act::Delete(r, k) if n > 2 => {
            let a = at(*r);
            d.delete_rows(a, u64::from(*k).min(n - a))
        }
        Act::Move(r, k, t) if n > 3 => {
            let a = at(*r);
            let count = u64::from(*k).min(n - a);
            let room = (n - count).max(1);
            let to = (1 + t % room).min(n - count);
            d.move_rows(a, count, to)
        }
        Act::Sort(c, desc) => {
            let spec = SortSpec {
                keys: vec![SortKey {
                    col: *c,
                    descending: *desc,
                    kind: Default::default(),
                    match_case: false,
                }],
                keep_first: true,
                ..Default::default()
            };
            d.sort(&spec).map(|_| ())
        }
        Act::Hide(r, k) if n > 2 => {
            let a = at(*r);
            d.hide_rows(a, u64::from(*k).min(n - a)).map(|_| ())
        }
        Act::Upper(c) => {
            let spec = CleanSpec {
                cleaner: Cleaner {
                    version: 1,
                    name: String::new(),
                    steps: vec![Step::Case { to: CaseTo::Upper }],
                },
                scope: Scope::Columns { cols: vec![*c] },
                skip_first: true,
                read: Default::default(),
            };
            match d.plan_clean(&spec, 20_000.0, 1, 0, &Progress::new()) {
                Ok(p) => d.apply_clean(p),
                Err(e) => Err(e),
            }
        }
        Act::Undo => {
            d.undo();
            Ok(())
        }
        Act::Redo => {
            d.redo();
            Ok(())
        }
        _ => Ok(()),
    };
}

fn shown(d: &Document) -> Vec<Vec<String>> {
    (0..d.row_count())
        .map(|r| d.row_values(r).unwrap())
        .collect()
}

type SinceCells = Vec<(SinceKind, Option<u64>, Option<u32>, String, String)>;

fn since(d: &Document, mark: u64) -> Option<(SinceTotals, SinceCells)> {
    let plan = d.since_plan(mark, 0, true, &Progress::new()).ok()?;
    let mut cells: SinceCells = d
        .since_page(&plan, mark, 0, 0, 500)
        .ok()?
        .into_iter()
        .map(|c| (c.kind, c.row, c.col, c.old, c.new))
        .collect();
    cells.sort_by_key(|a| format!("{a:?}"));
    Some((plan.totals, cells))
}

fn dir(tag: &str) -> std::path::PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "nd-rebase-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn table(rows: usize) -> Vec<u8> {
    let mut s = String::from("a,b,c\r\n");
    for i in 0..rows {
        s += &format!("x{},{},z{}\r\n", (i * 7) % 13, (i * 5) % 11, i);
    }
    s.into_bytes()
}

/// One run: the same steps with and without a save in the middle.
fn check(acts: &[Act], save_at: usize, mark_at: usize) -> Result<(), TestCaseError> {
    let d0 = dir("p");
    let (fx, fy) = (d0.join("x.csv"), d0.join("y.csv"));
    let bytes = table(25);
    std::fs::write(&fx, &bytes).unwrap();
    std::fs::write(&fy, &bytes).unwrap();
    let mut x = Document::open_indexed(&fx, OpenOptions::default(), None).unwrap();
    let mut y = Document::open_indexed(&fy, OpenOptions::default(), None).unwrap();
    let mut marks = (x.step_mark(), y.step_mark());
    for (i, a) in acts.iter().enumerate() {
        if i == mark_at {
            marks = (x.step_mark(), y.step_mark());
        }
        if i == save_at {
            let before = since(&x, marks.0);
            x.save(SaveOptions::default()).unwrap();
            prop_assert!(!x.is_modified());
            prop_assert_eq!(
                &before,
                &since(&x, marks.0),
                "since, before and after the save"
            );
        }
        apply(&mut x, a);
        apply(&mut y, a);
        prop_assert_eq!(shown(&x), shown(&y), "after step {} {:?}", i, a);
    }
    prop_assert_eq!(since(&x, marks.0), since(&y, marks.1), "since at the end");
    // Back to the mark across the save, and forward again.
    prop_assert_eq!(x.go_to_mark(marks.0), y.go_to_mark(marks.1));
    prop_assert_eq!(shown(&x), shown(&y), "at the mark");
    while x.undo() {
        prop_assert!(y.undo());
        prop_assert_eq!(shown(&x), shown(&y), "undoing");
    }
    prop_assert!(!y.undo());
    while x.redo() {
        prop_assert!(y.redo());
        prop_assert_eq!(shown(&x), shown(&y), "redoing");
    }
    prop_assert!(!y.redo());
    let (gx, gy) = (d0.join("x2.csv"), d0.join("y2.csv"));
    for (d, g) in [(&mut x, &gx), (&mut y, &gy)] {
        d.save(SaveOptions {
            path: Some(g.clone()),
            ..Default::default()
        })
        .unwrap();
    }
    prop_assert_eq!(std::fs::read(&gx).unwrap(), std::fs::read(&gy).unwrap());
    drop((x, y));
    let _ = std::fs::remove_dir_all(d0);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]
    #[test]
    fn a_save_in_the_middle_changes_nothing_undo_redo_or_checkpoints_see(
        acts in prop::collection::vec(act(), 1..30),
        save in 0..30usize,
        mark in 0..30usize,
    ) {
        let save = save % acts.len();
        check(&acts, save, mark % acts.len())?;
    }
}

#[test]
fn a_column_step_cuts_the_history_there() {
    let d0 = dir("cut");
    let f = d0.join("c.csv");
    std::fs::write(&f, table(5)).unwrap();
    let mut d = Document::open_indexed(&f, OpenOptions::default(), None).unwrap();
    let start = d.step_mark();
    d.set_cell(1, 0, "before".into()).unwrap();
    let plan = d.plan_delete_cols(&[2]).unwrap();
    d.apply_cols(plan).unwrap();
    let after_col = d.step_mark();
    d.set_cell(2, 0, "after".into()).unwrap();
    d.save(SaveOptions::default()).unwrap();
    assert_eq!(
        d.step_reach(start),
        None,
        "older than the column step: gone"
    );
    assert_eq!(d.step_reach(after_col), Some(-1));
    assert!(d.undo());
    assert_eq!(d.cell(2, 0).unwrap(), "x7");
    assert!(d.is_modified());
    assert!(!d.undo(), "nothing before the cut");
    assert!(d.redo());
    assert_eq!(d.cell(2, 0).unwrap(), "after");
    assert!(!d.is_modified());
    let _ = std::fs::remove_dir_all(d0);
}

/// Spike timings: `cargo test --release -p natendo-engine --lib rebase_timing -- --ignored --nocapture`.
#[test]
#[ignore]
fn rebase_timing() {
    let rows: usize = std::env::var("ND_ROWS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let edits: usize = std::env::var("ND_EDITS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);
    let d0 = dir("time");
    let src = table(rows);
    let run = |on: bool| -> (u128, Option<RebaseReport>) {
        let f = d0.join(format!("t{on}.csv"));
        std::fs::write(&f, &src).unwrap();
        let mut d = Document::open_indexed(&f, OpenOptions::default(), None).unwrap();
        let cells: Vec<(u64, u32, String)> = (0..edits)
            .map(|i| (1 + (i as u64 * 7919) % rows as u64, 1, format!("e{i}")))
            .collect();
        d.set_cells(cells).unwrap();
        let spec = SortSpec {
            keys: vec![SortKey {
                col: 0,
                descending: false,
                kind: Default::default(),
                match_case: false,
            }],
            keep_first: true,
            ..Default::default()
        };
        d.sort(&spec).unwrap();
        set_rebase(on);
        let t = std::time::Instant::now();
        d.save(SaveOptions::default()).unwrap();
        let ms = t.elapsed().as_millis();
        set_rebase(true);
        if on {
            let t = std::time::Instant::now();
            assert!(d.undo() && d.undo());
            eprintln!(
                "undo of the sort and the edits after the save: {} ms",
                t.elapsed().as_millis()
            );
        }
        (ms, if on { last_rebase() } else { None })
    };
    let (mut off, mut on, mut rep) = (Vec::new(), Vec::new(), None);
    for _ in 0..5 {
        off.push(run(false).0);
        let (ms, r) = run(true);
        on.push(ms);
        rep = r;
    }
    off.sort();
    on.sort();
    eprintln!(
        "{rows} rows, {edits} edits + a sort: save today {} ms, with rebase {} ms ({:.3}x); rebase {:?}",
        off[2],
        on[2],
        on[2] as f64 / off[2] as f64,
        rep
    );
    let _ = std::fs::remove_dir_all(d0);
}
