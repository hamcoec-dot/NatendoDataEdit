//! Moving rows while a filter is on or rows are hidden (gap #5): the shown
//! rows move, hidden rows stay where they are.

use super::*;
use crate::filter::FilterSpec;
use crate::notes::{Note, NoteBook};
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

/// `n` rows after the titles: "r<i>", a key "k<i % keys>", and a number.
fn doc(n: u32, keys: u32) -> Document {
    let mut s = String::from("name,k,num\n");
    for i in 1..=n {
        s.push_str(&format!("r{i},k{},{}\n", i % keys, (i * 7) % 13));
    }
    Document::from_bytes(s.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
}

fn only(values: &[String]) -> FilterSpec {
    let list = values
        .iter()
        .map(|v| format!("\"{v}\""))
        .collect::<Vec<_>>()
        .join(",");
    serde_json::from_str(&format!(
        r#"{{"columns":[{{"col":1,"rule":{{"type":"values","values":[{list}],"blanks":false}}}}],"keep_first":true}}"#
    ))
    .unwrap()
}

fn order(d: &Document) -> Vec<u32> {
    d.ids().collect()
}

/// Every row's cells, by id.
fn cells(d: &Document) -> BTreeMap<u32, Vec<String>> {
    d.ids()
        .map(|id| (id, d.physical_values(id).unwrap()))
        .collect()
}

fn shown(d: &Document) -> BTreeSet<u32> {
    d.ids().filter(|&id| d.is_shown(id)).collect()
}

/// The first column of the shown rows.
fn names(d: &Document) -> Vec<String> {
    (0..d.row_count())
        .map(|r| d.row_values(r).unwrap()[0].clone())
        .collect()
}

/// Checks (a)-(f) of the hidden-rows guarantee for one move.
fn check_move(d: &mut Document, at: u64, count: u64, to: u64) -> Result<(), TestCaseError> {
    let before = order(d);
    let before_cells = cells(d);
    let before_shown = shown(d);
    let rows = d.row_count();
    let total = d.total_rows();
    let moving: Vec<u32> = (at..at + count).map(|r| d.id_at(r).unwrap()).collect();
    let r = d.move_rows_at(at, count, to);
    prop_assert!(r.is_ok(), "{r:?}");
    let after = order(d);
    let moved: BTreeSet<u32> = moving.iter().copied().collect();
    // (a) the rows that did not move keep their order.
    let stay = |o: &[u32]| {
        o.iter()
            .copied()
            .filter(|id| !moved.contains(id))
            .collect::<Vec<_>>()
    };
    prop_assert_eq!(stay(&before), stay(&after));
    // (b) the moved rows keep their order and sit together, from shown row `to`.
    let first = after.iter().position(|id| moved.contains(id)).unwrap();
    prop_assert_eq!(&after[first..first + moving.len()], &moving[..]);
    prop_assert_eq!(d.id_at(to), Some(moving[0]));
    // (c) every row's cells are the same; (d) the same number of rows.
    prop_assert_eq!(cells(d), before_cells);
    prop_assert_eq!((d.row_count(), d.total_rows()), (rows, total));
    // (e) the same rows are shown, and the shown list kept up in place
    // matches one made afresh.
    prop_assert_eq!(shown(d), before_shown);
    let fresh: Vec<u32> = d.ids().filter(|&id| d.is_shown(id)).collect();
    let kept: Vec<u32> = (0..d.row_count()).map(|r| d.id_at(r).unwrap()).collect();
    prop_assert_eq!(kept, fresh);
    // (f) Undo restores the order exactly; Redo gives the same move again.
    if r.unwrap().is_some() {
        let listed = |d: &Document| {
            (0..d.row_count())
                .map(|r| d.id_at(r).unwrap())
                .collect::<Vec<u32>>()
        };
        let fresh = |d: &Document| d.ids().filter(|&id| d.is_shown(id)).collect::<Vec<u32>>();
        d.undo();
        prop_assert_eq!(order(d), before);
        prop_assert_eq!(listed(d), fresh(d));
        d.redo();
        prop_assert_eq!(order(d), after);
        prop_assert_eq!(listed(d), fresh(d));
    } else {
        prop_assert_eq!(order(d), before);
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(160))]
    #[test]
    fn shown_rows_move_and_hidden_rows_stay(
        n in 1u32..300,
        keys in 1u32..6,
        pick in proptest::collection::btree_set(0u32..6, 0..4),
        hide in proptest::collection::vec((0u64..300, 1u64..6), 0..4),
        moves in proptest::collection::vec((0u64..400, 1u64..8, 0u64..400), 1..6),
    ) {
        let mut d = doc(n, keys);
        for &(a, k) in &hide {
            let rows = d.row_count();
            if rows > 1 {
                let a = 1 + a % (rows - 1);
                let _ = d.hide_rows(a, k.min(rows - a));
            }
        }
        if !pick.is_empty() {
            let vals: Vec<String> = pick.iter().map(|k| format!("k{k}")).collect();
            d.set_filter(&only(&vals)).unwrap();
        }
        for &(a, k, t) in &moves {
            let rows = d.row_count();
            if rows < 2 {
                break;
            }
            let at = 1 + a % (rows - 1);
            let count = k.min(rows - at);
            let room = rows - count;
            let to = 1.max(t % (room + 1)).min(room);
            check_move(&mut d, at, count, to)?;
        }
    }
}

#[test]
fn five_thousand_rows_with_a_filter_and_hidden_rows() {
    let mut d = doc(5000, 3);
    d.hide_rows(100, 40).unwrap();
    d.set_filter(&only(&["k1".into(), "k2".into()])).unwrap();
    let mut rnd = 12345u64;
    let mut next = |m: u64| {
        rnd = rnd
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (rnd >> 33) % m
    };
    for _ in 0..40 {
        let rows = d.row_count();
        let at = 1 + next(rows - 1);
        let count = (1 + next(300)).min(rows - at);
        let to = 1.max(next(rows - count + 1)).min(rows - count);
        check_move(&mut d, at, count, to).unwrap();
    }
}

#[test]
fn out_of_range_moves_change_nothing() {
    let mut d = doc(20, 2);
    d.set_filter(&only(&["k1".into()])).unwrap();
    let rows = d.row_count(); // titles + 10
    let before = order(&d);
    let steps = d.undo.len();
    assert!(
        matches!(d.move_rows_at(1, 3, rows - 2), Err(Error::OutOfRange)),
        "to past the rows left"
    );
    assert!(
        matches!(d.move_rows_at(rows - 2, 3, 1), Err(Error::OutOfRange)),
        "block past the end"
    );
    assert!(matches!(
        d.move_rows_at(u64::MAX, 2, 1),
        Err(Error::OutOfRange)
    ));
    assert_eq!(order(&d), before);
    assert_eq!(d.undo.len(), steps);
}

#[test]
fn where_the_rows_go() {
    // Shown with k1: titles, r1, r3, r5, ... r19 (document rows 1, 3, 5, ...).
    let mut d = doc(20, 2);
    d.set_filter(&only(&["k1".into()])).unwrap();
    // r3..r7 (shown 2..5, hidden rows between) to the top of the data:
    // straight before r1; nothing hidden moves.
    let r = d.move_rows_at(2, 3, 1).unwrap().unwrap();
    assert_eq!(r, (vec![(3, 1), (5, 1), (7, 1)], 1));
    assert_eq!(names(&d)[..6], ["name", "r3", "r5", "r7", "r1", "r9"]);
    d.clear_filter();
    assert_eq!(
        names(&d)[..10],
        ["name", "r3", "r5", "r7", "r1", "r2", "r4", "r6", "r8", "r9"]
    );
    // To the end: straight after the last shown row (r19), before r20.
    let mut e = doc(20, 2);
    e.set_filter(&only(&["k1".into()])).unwrap();
    let rows = e.row_count();
    e.move_rows_at(1, 2, rows - 2).unwrap();
    e.clear_filter();
    assert_eq!(names(&e)[16..], ["r18", "r19", "r1", "r3", "r20"]);
}

#[test]
fn a_gathered_block_then_moves_as_one_run() {
    let mut d = doc(30, 3);
    d.set_filter(&only(&["k1".into()])).unwrap();
    // The first move of rows with hidden rows between them is a new order...
    d.move_rows_at(2, 3, 3).unwrap();
    assert!(matches!(d.undo.last(), Some(Op::SetOrder(_))));
    // ...and each move after it one MoveRows.
    let mut at = 3;
    for to in [4, 5, 4, 3, 2] {
        d.move_rows_at(at, 3, to)
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(
            matches!(d.undo.last(), Some(Op::MoveRows { .. })),
            "to {to}"
        );
        at = to;
    }
    // Twenty moves down (as Alt+Down held), then twenty Undos.
    let mut e = doc(200, 4);
    e.hide_rows(10, 5).unwrap();
    e.set_filter(&only(&["k1".into(), "k3".into()])).unwrap();
    let start = order(&e);
    for step in 0..20 {
        e.move_rows_at(3 + step, 4, 4 + step).unwrap();
        if step > 0 {
            assert!(matches!(e.undo.last(), Some(Op::MoveRows { .. })));
        }
    }
    for _ in 0..20 {
        assert!(e.undo());
    }
    assert_eq!(order(&e), start);
}

#[test]
fn a_rows_note_goes_with_it() {
    let mut d = doc(30, 3);
    let book = NoteBook {
        key_cols: vec![0],
        header: true,
        notes: [(
            "r7".to_string(),
            Note {
                flag: 1,
                text: "made-up note".into(),
            },
        )]
        .into_iter()
        .collect(),
        ..NoteBook::default()
    };
    d.set_filter(&only(&["k1".into()])).unwrap();
    // Shown: titles, r1, r4, r7, r10, ...: move r4..r10 (hidden rows between) to the end.
    let rows = d.row_count();
    d.move_rows_at(2, 3, rows - 3).unwrap();
    let at = (0..d.row_count())
        .find(|&r| d.note_key(&book, r).unwrap() == "r7")
        .unwrap();
    assert_eq!(at, rows - 2);
    assert!(book.notes.contains_key(&d.note_key(&book, at).unwrap()));
    assert!(d.row_noted(&book, d.id_at(at).unwrap()));
}

#[test]
fn changes_since_a_mark_call_it_a_reorder() {
    let mut d = doc(60, 3);
    let mark = d.step_mark();
    d.set_filter(&only(&["k2".into()])).unwrap();
    d.move_rows_at(1, 5, 7).unwrap();
    assert!(matches!(d.undo.last(), Some(Op::SetOrder(_))));
    d.clear_filter();
    let t = d.since_plan(mark, true, &Progress::new()).unwrap().totals;
    assert!(t.reordered);
    assert_eq!(
        (
            t.rows_changed,
            t.cells_changed,
            t.rows_added,
            t.rows_removed
        ),
        (0, 0, 0, 0)
    );
}
