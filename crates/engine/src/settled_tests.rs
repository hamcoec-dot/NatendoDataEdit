//! "Settled by" rules (made from Find relationships findings). Made-up data only.

use super::*;
use crate::filter::{ColumnFilter, Rule};
use crate::relations::{RelationKind, RelationsSpec};
use crate::sort::{SortKey, SortKind, SortSpec};
use crate::OpenOptions;
use proptest::prelude::*;

/// Read as comma-separated: tiny random tables (a few rows of blanks and spaces) can
/// otherwise be guessed another way, and these tests are about the rule, not the guess.
fn doc(text: &str) -> Document {
    Document::from_bytes(
        text.as_bytes().to_vec(),
        OpenOptions {
            dialect: Some(crate::Dialect::CSV),
            ..Default::default()
        },
        Some("csv"),
    )
    .unwrap()
}

fn settled(col: u32, by: u32, match_case: bool) -> Check {
    Check::new(Some(col), CheckKind::SettledBy { by, match_case })
}

fn spec(checks: Vec<Check>) -> CheckSpec {
    CheckSpec {
        checks,
        keep_first: true,
        ..Default::default()
    }
}

fn run(d: &Document, s: &CheckSpec) -> CheckReport {
    d.run_checks(s, &Progress::new()).unwrap()
}

/// Shown data rows (1-based, after the titles) that fail.
fn failing_rows(d: &Document, rep: &CheckReport) -> Vec<u64> {
    (1..d.row_count())
        .filter(|&r| bit(&rep.failing, d.id_at(r).unwrap()))
        .collect()
}

const PRECINCTS: &str = "precinct,ward\r\nP1,W1\r\nP1,W1\r\nP2,W3\r\nP1,W2\r\nP2,W4\r\nP1,W1\r\nP3,W5\r\n,W9\r\nP2,\r\nP2,W3\r\n";

#[test]
fn rows_off_their_groups_usual_value_fail() {
    let d = doc(PRECINCTS);
    let rep = run(&d, &spec(vec![settled(1, 0, true)]));
    // P1: W1 three times, W2 once (row 4). P2: W3 twice (rows 3, 10), W4 once (row 5).
    // A blank precinct (row 8), a blank ward (row 9) and P3 alone pass.
    assert_eq!(failing_rows(&d, &rep), [4, 5]);
    assert_eq!(rep.per_check, [2]);
    assert_eq!(rep.listed[0].value, "W2; most rows with P1 have W1");
    assert_eq!(rep.listed[1].value, "W4; most rows with P2 have W3");
    assert_eq!(rep.listed[0].col, Some(1));
}

#[test]
fn a_tie_goes_to_the_value_met_first_in_the_order_shown() {
    let mut d = doc("p,w\r\nP1,B\r\nP1,A\r\n");
    let rep = run(&d, &spec(vec![settled(1, 0, true)]));
    assert_eq!(failing_rows(&d, &rep), [2]);
    // Sorted by w: A comes first, so B is the one off.
    d.sort(&SortSpec {
        keys: vec![SortKey {
            col: 1,
            descending: false,
            kind: SortKind::Text,
            match_case: false,
        }],
        keep_first: true,
        ..Default::default()
    })
    .unwrap();
    let rep = run(&d, &spec(vec![settled(1, 0, true)]));
    assert_eq!(d.cell(2, 1).unwrap(), "B");
    assert_eq!(failing_rows(&d, &rep), [2]);
}

#[test]
fn case_and_spaces() {
    let d = doc("p,w\r\nP1,W1\r\n p1 ,w1\r\nP1, W1 \r\n");
    // Trimmed always; case matters unless told otherwise.
    assert_eq!(
        failing_rows(&d, &run(&d, &spec(vec![settled(1, 0, true)]))),
        Vec::<u64>::new()
    );
    let d = doc("p,w\r\nP1,W1\r\nP1,w1\r\nP1,W1\r\n");
    assert_eq!(
        failing_rows(&d, &run(&d, &spec(vec![settled(1, 0, true)]))),
        [2]
    );
    assert!(failing_rows(&d, &run(&d, &spec(vec![settled(1, 0, false)]))).is_empty());
}

#[test]
fn hidden_rows_are_not_counted() {
    let mut d = doc("p,w,k\r\nP1,W1,x\r\nP1,W2,y\r\nP1,W2,y\r\nP1,W1,x\r\nP1,W1,x\r\n");
    // All shown: W1 three times wins, the two W2 rows fail.
    assert_eq!(
        failing_rows(&d, &run(&d, &spec(vec![settled(1, 0, true)]))),
        [2, 3]
    );
    let plan = d
        .plan_filter(
            &FilterSpec {
                columns: vec![ColumnFilter {
                    col: 2,
                    rule: Rule::Values {
                        values: vec!["y".into()],
                        blanks: false,
                    },
                }],
                keep_first: true,
                ..Default::default()
            },
            &Progress::new(),
        )
        .unwrap();
    d.apply_filter(plan).unwrap();
    // Only the two W2 rows show: nothing breaks among them.
    assert!(failing_rows(&d, &run(&d, &spec(vec![settled(1, 0, true)]))).is_empty());
}

#[test]
fn marks_land_on_the_rule_column() {
    let d = doc(PRECINCTS);
    let s = spec(vec![settled(1, 0, true)]);
    let rep = run(&d, &s);
    let marks = d
        .check_marks(0, d.row_count(), 0, 2, &s, &rep.repeats)
        .unwrap();
    let at: Vec<(u64, Option<u32>)> = marks.iter().map(|m| (m.row, m.col)).collect();
    assert_eq!(at, [(4, Some(1)), (5, Some(1))]);
}

#[test]
fn the_same_column_is_refused_and_a_name_is_made() {
    let d = doc(PRECINCTS);
    assert!(d
        .run_checks(&spec(vec![settled(1, 1, true)]), &Progress::new())
        .is_err());
    assert!(check_spec(&spec(vec![settled(1, 1, true)])).is_err());
    assert_eq!(settled(1, 0, true).describe(), "Settled by column 1");
}

#[test]
fn too_many_values_does_not_run_and_says_why() {
    // Every precinct its own value: the text of the different values passes the (lowered) limit.
    let mut t = String::from("id,w\r\n");
    for i in 0..30 {
        t += &format!("A{i},W{}\r\n", i % 3);
    }
    let d = doc(&t);
    crate::relations::VALUE_BYTES.with(|c| c.set(500));
    let rep = run(&d, &spec(vec![settled(1, 0, true)]));
    crate::relations::VALUE_BYTES.with(|c| c.set(512 << 20));
    assert_eq!(rep.not_run.len(), 1);
    assert!(
        rep.not_run[0].why.contains("too many different values"),
        "{}",
        rep.not_run[0].why
    );
    assert_eq!(rep.per_check, [0]);
    // Nothing is shown as if it passed.
    assert!(d.plan_show_problems(&rep, true, false).is_err());
}

/// The scanner's own exception rows for one finding (ids), without the title row.
fn scanner_odd(d: &Document, kind: RelationKind) -> Vec<u32> {
    let rep = d
        .find_relations(
            &RelationsSpec {
                cols: vec![0, 1],
                ..Default::default()
            },
            &Progress::new(),
        )
        .unwrap();
    match d.plan_relation_exceptions(&rep, kind, 0, 1, &Progress::new()) {
        Ok(plan) => {
            let view = plan.view.expect("a view");
            (1..d.row_count())
                .map(|r| d.id_at(r).unwrap())
                .filter(|&id| view.passes(id))
                .collect()
        }
        Err(_) => Vec::new(),
    }
}

fn check_odd(d: &Document, check: Check) -> Vec<u32> {
    let rep = run(d, &spec(vec![check]));
    (1..d.row_count())
        .map(|r| d.id_at(r).unwrap())
        .filter(|&id| bit(&rep.failing, id))
        .collect()
}

fn value() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => prop::sample::select(vec!["a", "b", "c", "A", " a ", "d"]).prop_map(String::from),
        1 => Just(String::new()),
        1 => Just("  ".to_string()),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]
    /// A rule made from a finding flags exactly the rows the scanner says break it, on the
    /// table as it is, sorted, filtered, and sorted and filtered (R2, R3).
    #[test]
    fn the_rule_flags_the_rows_the_scanner_names(
        rows in prop::collection::vec((value(), value(), any::<bool>()), 2..60),
        view in 0u8..4,
        desc in any::<bool>(),
    ) {
        let mut t = String::from("a,b,k\r\n");
        for (a, b, k) in &rows {
            t += &format!("{a},{b},{}\r\n", if *k { "x" } else { "y" });
        }
        let mut d = doc(&t);
        if view & 1 == 1 {
            let plan = d.plan_filter(&FilterSpec {
                columns: vec![ColumnFilter { col: 2, rule: Rule::Values { values: vec!["x".into()], blanks: false } }],
                keep_first: true,
                ..Default::default()
            }, &Progress::new()).unwrap();
            if d.apply_filter(plan).is_err() {
                return Ok(());
            }
        }
        if view & 2 == 2 {
            d.sort(&SortSpec {
                keys: vec![SortKey { col: 1, descending: desc, kind: SortKind::Text, match_case: true }],
                keep_first: true,
                ..Default::default()
            }).unwrap();
        }
        if d.row_count() < 2 {
            return Ok(());
        }
        prop_assert_eq!(check_odd(&d, settled(1, 0, true)), scanner_odd(&d, RelationKind::Depends));
        let blank = Check::new(None, CheckKind::Expression { expr: "ISBLANK(TRIM([a])) = ISBLANK(TRIM([b]))".into() });
        prop_assert_eq!(check_odd(&d, blank), scanner_odd(&d, RelationKind::CoBlank));
    }
}

#[test]
fn blank_together_counts_cells_of_spaces_as_blank() {
    let d = doc("a,b\r\n  ,\r\nx,  \r\n,y\r\nx,y\r\n");
    let blank = Check::new(
        None,
        CheckKind::Expression {
            expr: "ISBLANK(TRIM([a])) = ISBLANK(TRIM([b]))".into(),
        },
    );
    let rep = run(&d, &spec(vec![blank]));
    // Row 1: both blank (spaces): passes. Rows 2, 3: one blank: fail. Row 4: both filled.
    assert_eq!(failing_rows(&d, &rep), [2, 3]);
}
