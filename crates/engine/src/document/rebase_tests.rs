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
    DelCol(u32),
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
        1 => (0..3u32).prop_map(Act::DelCol),
        2 => Just(Act::Undo),
        1 => Just(Act::Redo),
    ]
}

fn apply(d: &mut Document, a: &Act) {
    let n = d.row_count();
    let at = |r: u64| if n <= 1 { 1 } else { 1 + r % (n - 1) };
    let _ = match a {
        Act::Edit(r, c, v) if n > 1 => d.set_cell(at(*r), *c, value(*v)),
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
        // Counted on the title row without empty cells at its end, which a saved file and its twin agree on.
        Act::DelCol(c) if shown(d)[0].len() > 1 => {
            let w = shown(d)[0].len() as u32;
            d.plan_delete_cols(&[c % w])
                .and_then(|p| d.apply_cols(p).map(|_| ()))
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

/// The rows as shown, without empty cells at their ends: after a column step a saved file and its unsaved twin can
/// differ in how far a row is padded (the saved one has no column map), never in a value.
fn shown(d: &Document) -> Vec<Vec<String>> {
    (0..d.row_count())
        .map(|r| {
            let mut v = d.row_values(r).unwrap();
            while v.last().is_some_and(String::is_empty) {
                v.pop();
            }
            v
        })
        .collect()
}

type SinceCells = Vec<(SinceKind, Option<u64>, Option<u32>, String, String)>;

fn since(d: &Document, mark: u64) -> Option<(SinceTotals, SinceCells)> {
    let plan = d.since_plan(mark, true, &Progress::new()).ok()?;
    let mut cells: SinceCells = d
        .since_page(&plan, mark, 0, 500)
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

/// A typed value: plain, or one that needs quoting or decoding when written (C2).
fn value(v: u8) -> String {
    match v % 7 {
        0 => "a,b".into(),
        1 => "say \"hi\"".into(),
        2 => "two\r\nlines".into(),
        3 => "  edge  ".into(),
        4 => String::new(),
        5 => "Zoë Ñandú".into(),
        _ => format!("v{v}"),
    }
}

/// The file a run starts from (C2): 0 plain UTF-8; 1 UTF-8 with a BOM and cells with quotes, separators, line
/// breaks, edge spaces, empty and non-ASCII values; 2 the same cells in Windows-1252.
fn start_file(kind: u8, rows: usize) -> (Vec<u8>, OpenOptions) {
    if kind == 0 {
        return (table(rows), OpenOptions::default());
    }
    let mut s = String::from("a,b,c\r\n");
    for i in 0..rows {
        let b = match i % 5 {
            0 => "\"x, y\"".to_string(),
            1 => "\"he said \"\"no\"\"\"".to_string(),
            2 => "\"line\r\nbreak\"".to_string(),
            3 => " spaced ".to_string(),
            _ => String::new(),
        };
        s += &format!("Zoë{},{b},z{i}\r\n", (i * 7) % 13);
    }
    if kind == 1 {
        let mut b = vec![0xEF, 0xBB, 0xBF];
        b.extend_from_slice(s.as_bytes());
        (b, OpenOptions::default())
    } else {
        let enc = crate::encoding::Encoding::Legacy(encoding_rs::WINDOWS_1252);
        let mut out = Vec::new();
        crate::encoding::encode(&s, enc, &mut out).unwrap();
        (
            out,
            OpenOptions {
                encoding: Some(enc),
                ..Default::default()
            },
        )
    }
}

fn table(rows: usize) -> Vec<u8> {
    let mut s = String::from("a,b,c\r\n");
    for i in 0..rows {
        s += &format!("x{},{},z{}\r\n", (i * 7) % 13, (i * 5) % 11, i);
    }
    s.into_bytes()
}

/// Lets the twin's history go where the save let the saved document's go (a column step, or the budget).
fn cut_like(y: &mut Document, k: HistoryKept) {
    if k.cleared {
        y.undo.clear();
        y.redo.clear();
        y.undo_ids.clear();
        y.redo_ids.clear();
        y.base_mark = next_step_id();
    } else {
        let u = k.undo_cut();
        if u > 0 {
            y.base_mark = y.undo_ids[u - 1];
        }
        y.undo.drain(..u);
        y.undo_ids.drain(..u);
        y.clean_depth = y.clean_depth.and_then(|d| d.checked_sub(u));
        y.redo.drain(..k.redo_cut());
        y.redo_ids.drain(..k.redo_cut());
    }
    y.undo_bytes = y.undo.iter().map(Op::cost).sum();
    y.redo_bytes = y.redo.iter().map(Op::cost).sum();
}

/// One run: the same steps with and without a save in the middle.
fn check(acts: &[Act], save_at: usize, mark_at: usize, kind: u8) -> Result<(), TestCaseError> {
    let d0 = dir("p");
    let (fx, fy) = (d0.join("x.csv"), d0.join("y.csv"));
    let (bytes, opts) = start_file(kind, 25);
    std::fs::write(&fx, &bytes).unwrap();
    std::fs::write(&fy, &bytes).unwrap();
    let mut x = Document::open_indexed(&fx, opts.clone(), None).unwrap();
    let mut y = Document::open_indexed(&fy, opts, None).unwrap();
    let mut marks = (x.step_mark(), y.step_mark());
    for (i, a) in acts.iter().enumerate() {
        if i == mark_at {
            marks = (x.step_mark(), y.step_mark());
        }
        if i == save_at {
            let before = since(&x, marks.0);
            x.save(SaveOptions::default()).unwrap();
            prop_assert!(!x.is_modified());
            if let Some(k) = x.last_history() {
                cut_like(&mut y, k);
            }
            // A mark older than a cut is gone (on the twin too); every other one sees the same changes.
            prop_assert_eq!(x.step_reach(marks.0).is_some(), y.step_reach(marks.1).is_some());
            if x.step_reach(marks.0).is_some() {
                prop_assert_eq!(
                    &before,
                    &since(&x, marks.0),
                    "since, before and after the save"
                );
            }
        }
        apply(&mut x, a);
        apply(&mut y, a);
        prop_assert_eq!(shown(&x), shown(&y), "after step {} {:?}", i, a);
    }
    let col_step = acts.iter().any(|a| matches!(a, Act::DelCol(_)));
    if !col_step {
        // (After a column step the saved file has no column map and its twin does, so a cell typed past the last
        // column is "a cell changed" in one and "a column added" in the other: same values, other wording.)
        prop_assert_eq!(since(&x, marks.0), since(&y, marks.1), "since at the end");
    }
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
    if col_step {
        // (Padding aside, see `shown`.)
        prop_assert_eq!(shown(&x), shown(&y));
    } else {
        prop_assert_eq!(std::fs::read(&gx).unwrap(), std::fs::read(&gy).unwrap());
    }
    drop((x, y));
    let _ = std::fs::remove_dir_all(d0);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1000, ..ProptestConfig::default() })]
    #[test]
    fn a_save_in_the_middle_changes_nothing_undo_redo_or_checkpoints_see(
        acts in prop::collection::vec(act(), 1..30),
        save in 0..30usize,
        mark in 0..30usize,
        kind in 0..3u8,
    ) {
        let save = save % acts.len();
        check(&acts, save, mark % acts.len(), kind)?;
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
    let run = |on: bool| -> (u128, Option<HistoryKept>) {
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
        (ms, if on { d.last_history() } else { None })
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

fn open_bytes(tag: &str, bytes: &[u8]) -> (std::path::PathBuf, Document) {
    let d0 = dir(tag);
    let f = d0.join("t.csv");
    std::fs::write(&f, bytes).unwrap();
    let d = Document::open_indexed(&f, OpenOptions::default(), None).unwrap();
    (d0, d)
}

fn upper(d: &mut Document, c: u32) {
    apply(d, &Act::Upper(c));
}

/// U2/B1: what the history really holds after the save is never above what was worked out before it, on a file of
/// many 1-3 character cells and on one of long cells.
#[test]
fn the_estimate_is_not_below_what_is_held() {
    let short = {
        let mut s = (0..20).map(|c| format!("c{c}")).collect::<Vec<_>>().join(",") + "\r\n";
        for i in 0..400 {
            s += &((0..20).map(|c| format!("{}", (i + c) % 7)).collect::<Vec<_>>().join(",") + "\r\n");
        }
        s
    };
    let long = {
        let mut s = String::from("a,b,c\r\n");
        for i in 0..400 {
            s += &format!("{},{},{}\r\n", "x".repeat(40 + i % 60), "é".repeat(30), i);
        }
        s
    };
    for (tag, bytes) in [("short", short), ("long", long)] {
        let (d0, mut d) = open_bytes(tag, bytes.as_bytes());
        // Cell edits, a bulk clean (Patch states), row deletes (None states), a sort, hidden rows, an insert.
        for r in 1..60u64 {
            d.set_cell(r * 5, 1, "z".into()).unwrap();
        }
        upper(&mut d, 0);
        d.delete_rows(3, 40).unwrap();
        apply(&mut d, &Act::Sort(2, true));
        d.hide_rows(10, 5).unwrap();
        apply(&mut d, &Act::Insert(4, 3));
        assert!(d.undo() && d.undo(), "some steps on the redo side too");
        let est = d.prepare_rebase().ok().expect("kept").estimate;
        d.save(SaveOptions::default()).unwrap();
        let real = d.undo_bytes + d.redo_bytes;
        let k = d.last_history().unwrap();
        assert!(!k.cleared && k.undo_cut() == 0 && k.redo == 2, "{tag}: {k:?}");
        assert!(real <= est, "{tag}: real {real} > estimate {est}");
        eprintln!("{tag}: real {real}, estimate {est} ({:.2}x)", est as f64 / real as f64);
        drop(d);
        let _ = std::fs::remove_dir_all(d0);
    }
}

/// U2: with a small budget the newest steps that fit are kept, the older ones let go, and it says how many.
#[test]
fn a_save_keeps_the_newest_steps_that_fit() {
    let (d0, mut x) = open_bytes("fit", &table(40));
    let (d1, mut y) = open_bytes("fit2", &table(40));
    for i in 0..30u64 {
        for d in [&mut x, &mut y] {
            d.set_cell(1 + i, 1, format!("edit{i}")).unwrap();
        }
    }
    let one = x.rebased_cost(x.undo.last().unwrap(), 3, 1, &|_| true);
    x.undo_budget = one * 10 + one / 2;
    x.save(SaveOptions::default()).unwrap();
    let k = x.last_history().unwrap();
    assert_eq!((k.undo, k.undo_cut_memory, k.cleared), (10, 20, false), "{k:?}");
    assert!(x.undo_bytes <= x.undo_budget);
    for _ in 0..10 {
        assert!(x.undo() && y.undo());
        assert_eq!(shown(&x), shown(&y));
    }
    assert!(!x.undo(), "the older steps were let go");
    drop((x, y));
    let _ = std::fs::remove_dir_all(d0);
    let _ = std::fs::remove_dir_all(d1);
}

#[test]
fn nothing_fits_clears_the_history_as_before() {
    let (d0, mut d) = open_bytes("none", &table(10));
    d.set_cell(1, 1, "x".into()).unwrap();
    d.undo_budget = 10;
    d.save(SaveOptions::default()).unwrap();
    let k = d.last_history().unwrap();
    assert!(k.cleared && k.undo == 0, "{k:?}");
    assert!(!d.undo() && !d.is_modified());
    drop(d);
    let _ = std::fs::remove_dir_all(d0);
}

/// B2: three whole-column cleans of a big file with a budget for about one and a half: the newest is kept, the two
/// older ones are let go and counted.
#[test]
fn three_whole_column_cleans_keep_the_newest_that_fits() {
    let mut s = String::from("a,b,c\r\n");
    for i in 0..20_000 {
        s += &format!("name{i},street {i},town{}\r\n", i % 40);
    }
    let (d0, mut d) = open_bytes("cleans", s.as_bytes());
    for c in 0..3 {
        upper(&mut d, c);
    }
    let all = d.prepare_rebase().ok().unwrap().estimate;
    d.undo_budget = all / 2;
    d.save(SaveOptions::default()).unwrap();
    let k = d.last_history().unwrap();
    assert_eq!((k.undo, k.undo_cut_memory), (1, 2), "{k:?}");
    assert_eq!(d.cell(2, 2).unwrap(), "TOWN1");
    assert!(d.undo());
    assert_eq!(d.cell(2, 2).unwrap(), "town1", "the newest clean undoes");
    assert_eq!(d.cell(2, 1).unwrap(), "STREET 1", "the older cleans stay");
    assert!(!d.undo());
    drop(d);
    let _ = std::fs::remove_dir_all(d0);
}

/// B3: a column step waiting on the redo side is cut there too, at the step nearest the present.
#[test]
fn the_redo_side_is_cut_at_a_column_step() {
    let (d0, mut d) = open_bytes("redo", &table(5));
    let plan = d.plan_delete_cols(&[2]).unwrap();
    d.apply_cols(plan).unwrap();
    assert!(d.undo());
    d.save(SaveOptions {
        rewrite: true,
        ..Default::default()
    })
    .unwrap();
    let k = d.last_history().unwrap();
    assert_eq!((k.redo, k.redo_cut_columns), (0, 1), "{k:?}");
    assert!(!d.redo(), "the column delete cannot be redone after the save");
    assert_eq!(d.col_count(), 3);
    // An edit, then a column delete, then both undone: redo holds [column delete, edit], the edit nearest the
    // present. The save keeps the edit's redo and lets the column delete go.
    d.set_cell(1, 0, "e".into()).unwrap();
    let plan = d.plan_delete_cols(&[2]).unwrap();
    d.apply_cols(plan).unwrap();
    assert!(d.undo() && d.undo());
    d.save(SaveOptions {
        rewrite: true,
        ..Default::default()
    })
    .unwrap();
    let k = d.last_history().unwrap();
    assert_eq!((k.redo, k.redo_cut_columns), (1, 1), "{k:?}");
    assert!(d.redo());
    assert_eq!(d.cell(1, 0).unwrap(), "e");
    assert!(!d.redo());
    assert_eq!(d.col_count(), 3);
    let _ = std::fs::remove_dir_all(d0);
}

/// U3: a kept "take changes from a newer file" plan is refused after a save (it names the rows of the old file).
#[test]
fn a_kept_updates_plan_is_refused_after_a_save() {
    use crate::updates::{compare_newer, UpdateChoice, UpdateSpec};
    let (d0, mut d) = open_bytes("upd", b"id,name\r\n1,Ann\r\n2,Bo\r\n");
    let newer = Document::from_bytes(b"id,name\r\n1,Anne\r\n2,Bo\r\n".to_vec(), OpenOptions::default(), Some("csv")).unwrap();
    d.set_cell(2, 1, "Bob".into()).unwrap();
    let spec = UpdateSpec {
        keys: vec![0],
        keep_first: true,
        ..Default::default()
    };
    let (ups, _) = compare_newer(&d, &newer, &spec, &Progress::new()).unwrap();
    assert!(d.plan_updates(&ups, &UpdateChoice::default()).is_ok());
    d.save(SaveOptions::default()).unwrap();
    assert!(d.has_history(), "the edit is still undoable");
    assert!(matches!(d.plan_updates(&ups, &UpdateChoice::default()), Err(Error::Changed)));
    drop(d);
    let _ = std::fs::remove_dir_all(d0);
}

/// C3: a clean, a save, edits to the same rows, then two undos: the values from before the clean come back.
#[test]
fn a_clean_saved_then_edited_undoes_to_the_values_before_it() {
    let (bytes, opts) = start_file(1, 40);
    let d0 = dir("c3");
    let f = d0.join("t.csv");
    std::fs::write(&f, &bytes).unwrap();
    let mut d = Document::open_indexed(&f, opts, None).unwrap();
    let before: Vec<Vec<String>> = shown(&d);
    upper(&mut d, 0);
    let cleaned = shown(&d);
    assert_ne!(before, cleaned);
    d.save(SaveOptions::default()).unwrap();
    d.set_cells((1..20u64).map(|r| (r, 0, format!("edited {r}"))).collect())
        .unwrap();
    assert!(d.undo(), "the edits");
    assert_eq!(shown(&d), cleaned);
    assert!(d.undo(), "the clean, across the save");
    assert_eq!(shown(&d), before, "the values from before the clean");
    assert!(d.redo() && d.redo());
    assert_eq!(d.cell(1, 0).unwrap(), "edited 1");
    drop(d);
    let _ = std::fs::remove_dir_all(d0);
}
