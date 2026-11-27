//! The fill handle following the user's custom orders.

use super::*;

fn order(name: &str, items: &[&str], one_cell: bool) -> FillOrder {
    FillOrder {
        name: name.into(),
        items: items.iter().map(|s| s.to_string()).collect(),
        one_cell,
    }
}

fn run(
    orders: &[FillOrder],
    start: &[&str],
    mode: FillMode,
    from: i64,
    n: i64,
) -> (Vec<String>, Option<usize>) {
    let start: Vec<String> = start.iter().map(|s| s.to_string()).collect();
    let s = Series::new_with(
        &start,
        mode,
        ReadOptions::default(),
        &mut OrderMatch::new(orders),
    )
    .unwrap();
    (
        (from..from + n).filter_map(|i| s.at(i)).collect(),
        s.order(),
    )
}

fn priority() -> FillOrder {
    order("Priority", &["Low", "Medium", "High"], true)
}

#[test]
fn one_cell_follows_a_ticked_order_and_wraps() {
    let (v, used) = run(&[priority()], &["low"], FillMode::Auto, 1, 4);
    assert_eq!(v, ["Medium", "High", "Low", "Medium"]);
    assert_eq!(used, Some(0));
    // Filling back (up or left).
    let (v, _) = run(&[priority()], &["Low"], FillMode::Auto, -3, 3);
    assert_eq!(v, ["Low", "Medium", "High"]);
}

#[test]
fn one_cell_copies_on_an_order_not_ticked() {
    // An order made from a column of names: dragging one name must copy it,
    // never write other people's names down the column.
    let names = order("Names", &["Smith", "Jones", "Brown", "Lee"], false);
    let (v, used) = run(&[names], &["Smith"], FillMode::Auto, 1, 3);
    assert_eq!(v, ["Smith", "Smith", "Smith"]);
    assert_eq!(used, None);
}

#[test]
fn two_evenly_stepped_starts_follow_any_order() {
    let names = order("Names", &["Smith", "Jones", "Brown", "Lee"], false);
    let (v, used) = run(
        std::slice::from_ref(&names),
        &["Smith", "Brown"],
        FillMode::Auto,
        2,
        3,
    );
    assert_eq!(v, ["Smith", "Brown", "Smith"]);
    assert_eq!(used, Some(0));
    let (v, _) = run(
        std::slice::from_ref(&names),
        &["Jones", "Brown", "Lee"],
        FillMode::Auto,
        3,
        2,
    );
    assert_eq!(v, ["Smith", "Jones"]);
    // Not evenly stepped, or the same value twice: copied.
    let (v, used) = run(
        std::slice::from_ref(&names),
        &["Smith", "Jones", "Lee"],
        FillMode::Auto,
        3,
        3,
    );
    assert_eq!(v, ["Smith", "Jones", "Lee"]);
    assert_eq!(used, None);
    let (_, used) = run(&[names], &["Smith", "smith"], FillMode::Auto, 2, 2);
    assert_eq!(used, None);
}

#[test]
fn the_first_order_holding_the_value_wins() {
    let a = order("Sizes", &["Small", "Medium", "Large"], true);
    let (v, used) = run(&[a.clone(), priority()], &["Medium"], FillMode::Auto, 1, 1);
    assert_eq!((v, used), (vec!["Large".to_string()], Some(0)));
    let (v, used) = run(&[priority(), a.clone()], &["Medium"], FillMode::Auto, 1, 1);
    assert_eq!((v, used), (vec!["High".to_string()], Some(0)));
    // A later order is used when the first cannot read every start.
    let (v, used) = run(&[a, priority()], &["Medium", "High"], FillMode::Auto, 2, 1);
    assert_eq!((v, used), (vec!["Low".to_string()], Some(1)));
}

#[test]
fn case_accents_and_spaces_fold_and_new_cells_take_the_order_spelling() {
    let o = order("Rooms", &["Café", "Bar", "Hall"], true);
    let (v, _) = run(&[o], &["  CAFE "], FillMode::Auto, 1, 2);
    assert_eq!(v, ["Bar", "Hall"]);
}

#[test]
fn numbers_dates_months_and_numbered_text_never_look_at_orders() {
    let o = order(
        "Odd",
        &["Jan", "Spring", "10", "20", "2026-01-01", "Room 1", "Hall"],
        true,
    );
    let os = std::slice::from_ref(&o);
    assert_eq!(run(os, &["Jan"], FillMode::Auto, 1, 1).0, ["Feb"]);
    assert_eq!(run(os, &["10"], FillMode::Auto, 1, 1).0, ["10"]);
    assert_eq!(
        run(os, &["2026-01-01"], FillMode::Auto, 1, 1).0,
        ["2026-01-02"]
    );
    assert_eq!(run(os, &["Room 1"], FillMode::Auto, 1, 1).0, ["Room 2"]);
    // Numbered text that makes no series is copied, not looked up.
    assert_eq!(
        run(os, &["Room 1", "Room 4", "Room 5"], FillMode::Auto, 3, 1).0,
        ["Room 1"]
    );
    // Plain words still follow.
    assert_eq!(run(os, &["Spring"], FillMode::Auto, 1, 1).0, ["10"]);
}

#[test]
fn ctrl_drag_and_other_modes_copy() {
    let (v, used) = run(&[priority()], &["Low"], FillMode::AutoSwap, 1, 2);
    assert_eq!(v, ["Low", "Low"]);
    assert_eq!(used, None);
    let (v, _) = run(&[priority()], &["Low"], FillMode::Copy, 1, 2);
    assert_eq!(v, ["Low", "Low"]);
}

#[test]
fn many_lookups_index_the_orders_with_the_same_answers() {
    // Letters only: text with a digit is a numbered series, never an order.
    let word = |n: usize| -> String {
        n.to_string()
            .bytes()
            .map(|b| (b'a' + (b - b'0')) as char)
            .collect()
    };
    // 300 orders of 40 made-up items each, a few items shared between them.
    let orders: Vec<FillOrder> = (0..300)
        .map(|j| {
            let items: Vec<String> = (0..40)
                .map(|i| format!("w{}x{}", word((j * 7 + i) % 500), word(i % 3)))
                .collect();
            FillOrder {
                name: format!("o{j}"),
                items,
                one_cell: j % 2 == 0,
            }
        })
        .collect();
    let mut shared = OrderMatch::new(&orders);
    for k in 0..200 {
        let start = vec![format!("W{}X{}", word(k % 500), word(k % 3)).to_uppercase()];
        let a =
            Series::new_with(&start, FillMode::Auto, ReadOptions::default(), &mut shared).unwrap();
        let b = Series::new_with(
            &start,
            FillMode::Auto,
            ReadOptions::default(),
            &mut OrderMatch::new(&orders),
        )
        .unwrap();
        assert_eq!(a.order(), b.order(), "{start:?}");
        assert_eq!(
            (0..5).map(|i| a.at(i)).collect::<Vec<_>>(),
            (0..5).map(|i| b.at(i)).collect::<Vec<_>>()
        );
    }
    assert!(
        shared.index.is_some(),
        "the orders were indexed after many lookups"
    );
}

#[test]
fn forward_then_back_returns_the_start() {
    let items: Vec<String> = (0..7)
        .map(|i| format!("step {}", ["a", "b", "c", "d", "e", "f", "g"][i]))
        .collect();
    let o = FillOrder {
        name: "Steps".into(),
        items: items.clone(),
        one_cell: true,
    };
    for p in 0..7 {
        for step in 1..7 {
            let start = vec![items[p].clone(), items[(p + step) % 7].clone()];
            let s = Series::new_with(
                &start,
                FillMode::Auto,
                ReadOptions::default(),
                &mut OrderMatch::new(std::slice::from_ref(&o)),
            )
            .unwrap();
            assert_eq!(s.order(), Some(0));
            for k in 0..20i64 {
                let fwd = s.at(k).unwrap();
                let back = Series::new_with(
                    std::slice::from_ref(&fwd),
                    FillMode::Auto,
                    ReadOptions::default(),
                    &mut OrderMatch::new(std::slice::from_ref(&o)),
                )
                .unwrap();
                assert_eq!(back.at(0).unwrap(), fwd);
            }
            assert_eq!(s.at(0).unwrap(), items[p]);
            assert_eq!(s.at(-7).unwrap(), items[p]);
        }
    }
}

#[test]
fn an_old_fill_without_orders_still_reads() {
    let spec: crate::ops::FillSpec = serde_json::from_str(
        r#"{"row":1,"col":0,"rows":1,"cols":1,"dir":"down","count":3,"mode":{"kind":"auto"}}"#,
    )
    .unwrap();
    assert!(spec.orders.is_empty() && !spec.saved_orders);
    // ...and a fill without orders writes neither field.
    let j = serde_json::to_string(&spec).unwrap();
    assert!(!j.contains("orders"), "{j}");
}

mod in_a_document {
    use super::*;
    use crate::document::{Document, Progress};
    use crate::ops::{Dir, FillSpec};
    use crate::{Dialect, OpenOptions};

    fn doc(text: &str) -> Document {
        let o = OpenOptions {
            dialect: Some(Dialect::CSV),
            ..Default::default()
        };
        Document::from_bytes(text.as_bytes().to_vec(), o, Some("csv")).unwrap()
    }

    fn down(row: u64, col: u32, rows: u64, count: u64) -> FillSpec {
        FillSpec {
            row,
            col,
            rows,
            cols: 1,
            dir: Dir::Down,
            count,
            mode: FillMode::Auto,
            ..Default::default()
        }
    }

    #[test]
    fn a_fill_names_the_order_it_followed() {
        let mut d = doc("level,who\nLow,ann\n,\n,\n,\n");
        let mut spec = down(1, 0, 1, 3);
        spec.orders = vec![priority()];
        let plan = d.plan_fill(&spec, &Progress::new()).unwrap();
        assert_eq!(plan.order.as_deref(), Some("Priority"));
        d.apply_cells(plan).unwrap();
        let col: Vec<String> = (1..5)
            .map(|r| d.row_values(r).unwrap()[0].clone())
            .collect();
        assert_eq!(col, ["Low", "Medium", "High", "Low"]);
        // Dragging one name with no ticked order copies it.
        let mut spec = down(1, 1, 1, 3);
        spec.orders = vec![order("Names", &["ann", "bob", "cy"], false)];
        let plan = d.plan_fill(&spec, &Progress::new()).unwrap();
        assert_eq!(plan.order, None);
        d.apply_cells(plan).unwrap();
        let col: Vec<String> = (1..5)
            .map(|r| d.row_values(r).unwrap()[1].clone())
            .collect();
        assert_eq!(col, ["ann", "ann", "ann", "ann"]);
    }

    #[test]
    fn only_the_orders_a_fill_uses_are_picked() {
        let d = doc("a,b\nLow,x\nHigh,Jones\n");
        let saved = vec![
            order("Names", &["Smith", "Jones"], false),
            priority(),
            order("Other", &["x", "y"], true),
        ];
        let p = Progress::new();
        assert_eq!(
            d.fill_orders_used(&down(1, 0, 1, 2), &saved, &p).unwrap(),
            [1]
        );
        // One unticked name: nothing.
        assert!(d
            .fill_orders_used(&down(2, 1, 1, 2), &saved, &p)
            .unwrap()
            .is_empty());
        // Across, each row its own line: rows 1 and 2 from columns a..b.
        let across = FillSpec {
            row: 1,
            col: 1,
            rows: 1,
            cols: 1,
            dir: Dir::Right,
            count: 2,
            mode: FillMode::Auto,
            ..Default::default()
        };
        assert_eq!(d.fill_orders_used(&across, &saved, &p).unwrap(), [2]);
        // Copy and Ctrl+drag never use one.
        let mut copy = down(1, 0, 1, 2);
        copy.mode = FillMode::AutoSwap;
        assert!(d.fill_orders_used(&copy, &saved, &p).unwrap().is_empty());
        // Out of range is an error, as for the fill itself.
        assert!(d.fill_orders_used(&down(9, 0, 1, 2), &saved, &p).is_err());
    }
}
