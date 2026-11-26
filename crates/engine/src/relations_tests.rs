use std::sync::atomic::Ordering;

use super::*;
use crate::OpenOptions;

fn doc(text: String) -> Document {
    Document::from_bytes(text.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
}

fn table(titles: &str, rows: impl Iterator<Item = Vec<String>>) -> Document {
    let mut s = format!("{titles}\r\n");
    for r in rows {
        s.push_str(&r.join(","));
        s.push_str("\r\n");
    }
    doc(s)
}

fn spec(cols: &[u32]) -> RelationsSpec {
    RelationsSpec {
        cols: cols.to_vec(),
        ..Default::default()
    }
}

fn run(d: &Document, s: &RelationsSpec) -> RelationsReport {
    d.find_relations(s, &Progress::new()).unwrap()
}

fn random(mut seed: u64) -> impl FnMut(u64) -> u64 {
    move |n| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    }
}

fn find(r: &RelationsReport, a: u32, b: u32) -> Option<&Finding> {
    r.findings
        .iter()
        .find(|f| f.kind != RelationKind::CoBlank && f.a == a && f.b == b)
}

/// ZIP, State (3 broken rows in 2 ZIPs), a unique ID, a constant.
fn voters(rows: usize) -> Document {
    table(
        "ZIP,State,ID,Country",
        (0..rows).map(|i| {
            let zip = i % 50;
            let state = if [7, 77, 177].contains(&i) {
                "XX".to_string()
            } else {
                format!("S{}", zip % 5)
            };
            vec![format!("Z{zip:02}"), state, format!("V{i}"), "US".into()]
        }),
    )
}

#[test]
fn a_planted_dependency_with_its_exceptions() {
    let mut d = voters(1000);
    let r = run(&d, &spec(&[0, 1, 2, 3]));
    assert_eq!(r.findings.len(), 1, "{:?}", r.findings);
    let f = r.findings[0].clone();
    assert_eq!(
        (f.kind, f.a, f.b, f.exact),
        (RelationKind::Depends, 0, 1, true)
    );
    assert_eq!(
        (
            f.there.coverage,
            f.there.agreeing,
            f.there.values,
            f.there.broken_values
        ),
        (1000, 997, 50, 2)
    );
    assert!(!f.few_rows && !f.few_per_value);
    assert_eq!(r.left_out, [(2, LeftOut::Unique), (3, LeftOut::Constant)]);
    let plan = d
        .plan_relation_exceptions(&r, f.kind, f.a, f.b, &Progress::new())
        .unwrap();
    d.apply_rows(plan).unwrap();
    let shown: Vec<String> = (0..d.row_count())
        .map(|i| d.row_values(i).unwrap()[2].clone())
        .collect();
    assert_eq!(shown, ["ID", "V7", "V77", "V177"]);
    // The table shows other rows now: the old scan is not used.
    assert!(d
        .plan_relation_exceptions(&r, f.kind, f.a, f.b, &Progress::new())
        .is_err());
}

#[test]
fn one_to_one_is_one_line_with_both_directions() {
    let d = table(
        "Code,Name",
        (0..300).map(|i| {
            let c = i % 6;
            let name = if i == 40 { 1 } else { c };
            vec![format!("C{c}"), format!("N{name}")]
        }),
    );
    let r = run(&d, &spec(&[0, 1]));
    assert_eq!(r.findings.len(), 1, "{:?}", r.findings);
    let f = &r.findings[0];
    assert_eq!((f.kind, f.a, f.b), (RelationKind::OneToOne, 0, 1));
    let back = f.back.unwrap();
    assert_eq!((f.there.agreeing, back.agreeing), (299, 299));
    // Exceptions of both directions: row 41 breaks Code -> Name (C4 with N1)
    // and Name -> Code (N1 with C4): the same single row.
    let plan = d
        .plan_relation_exceptions(&r, f.kind, f.a, f.b, &Progress::new())
        .unwrap();
    assert_eq!(plan.rows, 1);
}

#[test]
fn blank_together() {
    let d = table(
        "Phone,Type,Name",
        (0..500).map(|i| {
            let blank = i % 10 == 0;
            vec![
                if blank {
                    String::new()
                } else {
                    format!("5{i}")
                },
                if blank || i == 3 {
                    String::new()
                } else {
                    format!("T{}", i % 3)
                },
                format!("n{}", i % 7),
            ]
        }),
    );
    let r = run(&d, &spec(&[0, 1, 2]));
    let f = r
        .findings
        .iter()
        .find(|f| f.kind == RelationKind::CoBlank)
        .unwrap();
    assert_eq!(
        (f.a, f.b, f.there.agreeing, f.there.coverage),
        (0, 1, 50, 51)
    );
    let plan = d
        .plan_relation_exceptions(&r, f.kind, 0, 1, &Progress::new())
        .unwrap();
    assert_eq!(plan.rows, 1);
}

#[test]
fn a_mostly_one_value_column_is_not_settled_by_everything() {
    let mut next = random(11);
    let d = table(
        "A,State",
        (0..2000).map(|_| {
            vec![
                format!("a{}", next(40)),
                if next(100) < 97 { "TN" } else { "GA" }.to_string(),
            ]
        }),
    );
    let r = run(&d, &spec(&[0, 1]));
    assert!(r.findings.is_empty(), "{:?}", r.findings);
    assert_eq!(r.mostly_one, [(0, 1)]);
}

#[test]
fn random_columns_report_nothing() {
    for seed in 0..200u64 {
        let mut next = random(seed * 2 + 1);
        let (x, y, z) = (next(60) + 2, next(8) + 2, next(30) + 2);
        let d = table(
            "A,B,C,D",
            (0..600).map(|_| {
                vec![
                    format!("a{}", next(x)),
                    format!("b{}", next(y)),
                    format!("c{}", next(z)),
                    // Skewed: mostly one value.
                    if next(100) < 97 {
                        "TN".into()
                    } else {
                        format!("d{}", next(4))
                    },
                ]
            }),
        );
        let r = run(&d, &spec(&[0, 1, 2, 3]));
        assert!(r.findings.is_empty(), "seed {seed}: {:?}", r.findings);
    }
}

/// Every first value in exactly two rows: where agreement comes by chance.
#[test]
fn two_rows_a_value_report_nothing_by_chance() {
    for seed in 0..200u64 {
        let mut next = random(seed * 2 + 3);
        let kinds = seed % 9 + 2;
        let skewed = seed % 2 == 0;
        let d = table(
            "A,B",
            (0..600).map(|i| {
                let b = if skewed && next(100) < 60 {
                    0
                } else {
                    next(kinds)
                };
                vec![format!("a{}", i / 2), format!("b{b}")]
            }),
        );
        let r = run(&d, &spec(&[0, 1]));
        assert!(r.findings.is_empty(), "seed {seed} at 95: {:?}", r.findings);
        let mut s = spec(&[0, 1]);
        s.min_percent = 80.0;
        for f in run(&d, &s).findings {
            assert!(
                f.few_per_value || f.there.coverage >= f.there.values * 3,
                "seed {seed} at 80 unlabelled: {f:?}"
            );
        }
    }
}

#[test]
fn few_rows_and_few_per_value_are_said() {
    // 40 values twice each, settling B, among 2,000 values met once.
    let d = table(
        "A,B",
        (0..2080).map(|i| {
            let a = if i < 80 { i / 2 } else { 1000 + i };
            vec![format!("a{a}"), format!("b{}", a % 20)]
        }),
    );
    let mut s = spec(&[0, 1]);
    s.min_percent = 95.0;
    let r = run(&d, &s);
    // A is not "unique": 2,040 values in 2,080 rows is under the limit.
    let f = find(&r, 0, 1);
    assert!(f.is_none(), "more than half the rows have their own value");
    let d = table(
        "A,B",
        (0..1000).map(|i| {
            let a = if i < 60 { i / 2 } else { 100 + (i - 60) / 2 };
            let b = if i < 60 { a % 20 } else { i % 977 };
            vec![
                format!("a{a}"),
                if i < 60 {
                    format!("b{b}")
                } else {
                    String::new()
                },
            ]
        }),
    );
    let r = run(&d, &s);
    let f = find(&r, 0, 1).unwrap();
    assert!(f.few_rows && f.few_per_value, "{f:?}");
}

#[test]
fn a_sample_is_verified_and_finds_what_lies_past_it() {
    // The first 200,000 rows hold one value of each column; the dependency
    // shows only in the rows after, which every 50th row reaches.
    let n = 400_000;
    let d = table(
        "A,B",
        (0..n).map(|i| {
            if i < 200_000 {
                vec![String::new(), String::new()]
            } else {
                let a = (i / 50) % 40;
                let b = if i == 399_999 { 99 } else { a % 4 };
                vec![format!("a{a}"), format!("b{b}")]
            }
        }),
    );
    let r = run(&d, &spec(&[0, 1]));
    assert_eq!(r.sampled, 200_000 + 4_000);
    assert_eq!(r.rows, n as u64);
    let f = find(&r, 0, 1).expect("found through every 50th row");
    assert!(f.exact);
    // Counted over every row, the one odd row included.
    assert_eq!((f.there.coverage, f.there.agreeing), (200_000, 199_999));
}

#[test]
fn bad_specs_budget_and_cancel() {
    let d = voters(100);
    let refused = |s: RelationsSpec| d.find_relations(&s, &Progress::new()).unwrap_err();
    assert!(refused(spec(&[0])).to_string().contains("2 to 40"));
    assert!(refused(spec(&[0, 0])).to_string().contains("different"));
    let mut s = spec(&[0, 1]);
    s.min_percent = 50.0;
    assert!(refused(s).to_string().contains("80 to 100"));
    assert!(matches!(refused(spec(&[0, 9])), Error::OutOfRange));
    let p = Progress::new();
    p.cancel.store(true, Ordering::Relaxed);
    assert!(matches!(
        d.find_relations(&spec(&[0, 1]), &p),
        Err(Error::Cancelled)
    ));
}
