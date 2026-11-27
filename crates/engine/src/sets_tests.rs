use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::filter::{ColumnFilter, FilterSpec, Rule};
use crate::join::{JoinSource, JoinWrite, KeepRows};
use crate::OpenOptions;

fn doc(text: &str) -> Document {
    Document::from_bytes(
        text.as_bytes().to_vec(),
        OpenOptions::default(),
        Some("csv"),
    )
    .unwrap()
}

fn list(cols: &[u32]) -> SetList {
    SetList {
        cols: cols.to_vec(),
        header: true,
    }
}

fn spec(n: usize, cols: &[u32]) -> SetsSpec {
    SetsSpec {
        lists: (0..n).map(|_| list(cols)).collect(),
        ..Default::default()
    }
}

fn run(docs: &[&Document], s: &SetsSpec) -> SetsResult {
    compare_lists(docs, s, &Progress::new()).unwrap()
}

fn rows(t: Table) -> Vec<Vec<String>> {
    if t.rows == 0 {
        return Vec::new();
    }
    let d = t.into_document().unwrap();
    (0..d.row_count())
        .map(|r| d.row_values(r).unwrap())
        .collect()
}

#[test]
fn three_lists_by_slice() {
    let a = doc("id,n\r\n1,a\r\n2,b\r\n2,c\r\n3,d\r\n,e\r\n");
    let b = doc("id,n\r\n2,f\r\n3,g\r\n4,h\r\n");
    let c = doc("id,n\r\n3,i\r\n5,j\r\n");
    let r = run(&[&a, &b, &c], &spec(3, &[0]));
    let got: Vec<(u8, u64, Vec<u64>)> = r
        .report
        .slices
        .iter()
        .map(|s| (s.mask, s.keys, s.rows.clone()))
        .collect();
    assert_eq!(
        got,
        [
            (1, 1, vec![1, 0, 0]), // 1: A only
            (2, 1, vec![0, 1, 0]), // 4: B only
            (3, 1, vec![2, 1, 0]), // 2: A and B
            (4, 1, vec![0, 0, 1]), // 5: C only
            (5, 0, vec![0, 0, 0]),
            (6, 0, vec![0, 0, 0]),
            (7, 1, vec![1, 1, 1]), // 3: all
        ]
    );
    assert_eq!(
        r.report.lists[0],
        ListTotals {
            rows: 5,
            keys: 3,
            blank: 1
        }
    );
    let t = a
        .sets_slice_table(&list(&[0]), &r.row_masks[0], 3, &Progress::new())
        .unwrap();
    assert_eq!(rows(t), [["id", "n"], ["2", "b"], ["2", "c"]]);
    let t = a
        .sets_keys_table(
            &spec(3, &[0]),
            &[false],
            0,
            &r.row_masks[0],
            3,
            &Progress::new(),
        )
        .unwrap();
    assert_eq!(rows(t), [["id"], ["2"]]);
}

#[test]
fn key_rules_are_the_joins() {
    let a = doc("k,x\r\nÖlund,1\r\n 00123 ,2\r\nAB-012,3\r\n");
    let b = doc("k,x\r\nölund,1\r\n123,2\r\nab 12,3\r\n");
    let both = |s: &SetsSpec| run(&[&a, &b], s).report.slices[2].keys;
    let mut s = spec(2, &[0]);
    assert_eq!(both(&s), 1, "capitals ignored, Unicode too");
    s.match_case = true;
    assert_eq!(both(&s), 0);
    s.match_case = false;
    s.loose = true;
    assert_eq!(both(&s), 3);
    assert_eq!(
        s.rules_text(),
        "capitals ignored, spaces trimmed, loose keys"
    );
}

#[test]
fn an_id_column_stays_exact_with_loose_keys() {
    let mut a = doc("k,x\r\n00123,1\r\n");
    let b = doc("k,x\r\n123,1\r\n");
    let types = crate::coltype::ColumnTypes::default().with(0, crate::coltype::ColumnType::Id);
    a.set_column_types(types);
    let mut s = spec(2, &[0]);
    s.loose = true;
    assert_eq!(run(&[&a, &b], &s).report.slices[2].keys, 0);
}

#[test]
fn partly_blank_composite_keys_are_keys() {
    let a = doc("i,w\r\n1,\r\n,\r\n,2\r\n");
    let b = doc("i,w\r\n1,\r\n , \r\n");
    let r = run(&[&a, &b], &spec(2, &[0, 1]));
    assert_eq!((r.report.lists[0].keys, r.report.lists[0].blank), (2, 1));
    assert_eq!((r.report.lists[1].keys, r.report.lists[1].blank), (1, 1));
    assert_eq!(r.report.slices[2].keys, 1);
}

/// Slice counts against sets built the plain way, and the slices partition
/// the union.
#[test]
fn counts_agree_with_a_model() {
    const CELLS: [&str; 9] = ["", " ", "a", "A", " a ", "007", "7", "x-1", "Ölund"];
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n) as usize
    };
    for round in 0..120 {
        let n = [2, 3, 5][round % 3];
        let width = round % 2 + 1;
        let mut s = spec(n, &[0, 1][..width]);
        s.match_case = next(2) == 0;
        s.trim = next(2) == 0;
        let tables: Vec<Vec<(String, String)>> = (0..n)
            .map(|_| {
                (0..next(40))
                    .map(|_| {
                        let mut cell = || {
                            if next(3) == 0 {
                                CELLS[next(9)].to_string()
                            } else {
                                format!("k{}", next(15))
                            }
                        };
                        (cell(), cell())
                    })
                    .collect()
            })
            .collect();
        let docs: Vec<Document> = tables
            .iter()
            .map(|t| {
                let mut text = String::from("a,b\r\n");
                for (a, b) in t {
                    text.push_str(&format!("\"{a}\",\"{b}\"\r\n"));
                }
                doc(&text)
            })
            .collect();
        let refs: Vec<&Document> = docs.iter().collect();
        let r = run(&refs, &s);
        let norm = |v: &str| {
            let v = if s.trim { v.trim() } else { v };
            if s.match_case {
                v.to_string()
            } else {
                v.to_lowercase()
            }
        };
        let key = |row: &(String, String)| -> Option<Vec<String>> {
            let parts = [&row.0, &row.1];
            parts[..width]
                .iter()
                .any(|p| !p.trim().is_empty())
                .then(|| parts[..width].iter().map(|p| norm(p)).collect())
        };
        let mut masks: BTreeMap<Vec<String>, u8> = BTreeMap::new();
        for (i, t) in tables.iter().enumerate() {
            for k in t.iter().filter_map(key) {
                *masks.entry(k).or_default() |= 1 << i;
            }
        }
        let mut union = 0;
        for slice in &r.report.slices {
            let keys: BTreeSet<&Vec<String>> = masks
                .iter()
                .filter(|(_, &m)| m == slice.mask)
                .map(|(k, _)| k)
                .collect();
            assert_eq!(slice.keys, keys.len() as u64, "round {round} {slice:?}");
            union += slice.keys;
            for (i, t) in tables.iter().enumerate() {
                let want = t
                    .iter()
                    .filter(|row| key(row).is_some_and(|k| keys.contains(&k)))
                    .count() as u64;
                assert_eq!(slice.rows[i], want, "round {round} list {i} {slice:?}");
            }
        }
        assert_eq!(union, masks.len() as u64);
        for (i, t) in tables.iter().enumerate() {
            let in_slices: u64 = r.report.slices.iter().map(|s| s.rows[i]).sum();
            assert_eq!(in_slices + r.report.lists[i].blank, t.len() as u64);
        }
    }
}

/// With two lists the slices are the join's matched and unmatched rows.
#[test]
fn two_lists_agree_with_the_join() {
    const CELLS: [&str; 10] = [
        "", "-", "a", "A ", "00123", "123", "AB-012", "ab 12", "1.50", "1.5",
    ];
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n) as usize
    };
    for round in 0..80 {
        let mut table = || {
            let mut text = String::from("k,n\r\n");
            for r in 0..next(30) + 1 {
                text.push_str(&format!("\"{}\",{r}\r\n", CELLS[next(10)]));
            }
            text
        };
        let (a, b) = (table(), table());
        let (da, db) = (doc(&a), doc(&b));
        let mut s = spec(2, &[0]);
        s.loose = round % 2 == 0;
        s.trim = round % 4 < 2;
        s.match_case = round % 8 < 4;
        let r = run(&[&da, &db], &s);
        for (keep, mask) in [(KeepRows::Matched, 3), (KeepRows::Unmatched, 1)] {
            let join = JoinSpec {
                keys: vec![0],
                other_keys: vec![0],
                keep_first: true,
                other_header: true,
                match_case: s.match_case,
                trim: s.trim,
                loose: s.loose,
                write: JoinWrite::Nothing,
                keep_rows: keep,
                ..Default::default()
            };
            let mut joined = doc(&a);
            let source = JoinSource::build(&db, &join, &Progress::new()).unwrap();
            // "No rows would change" means every row is kept.
            match joined.plan_join(&source, &join, &Progress::new()) {
                Ok((plan, _)) => joined.apply_join(plan).unwrap(),
                Err(e) => assert!(e.to_string().contains("no rows would change"), "{e}"),
            }
            // The join keeps blank-key rows with the unmatched ones; here
            // they are counted apart. Column "n" is the row's place.
            let want: Vec<Vec<String>> = (0..joined.row_count())
                .map(|i| joined.row_values(i).unwrap())
                .filter(|row| {
                    row[1] == "n" || r.row_masks[0][row[1].parse::<usize>().unwrap()] != 0
                })
                .collect();
            let blanks = r.row_masks[0].iter().filter(|&&m| m == 0).count() as u64;
            assert_eq!(blanks, r.report.lists[0].blank);
            let t = da
                .sets_slice_table(&s.lists[0], &r.row_masks[0], mask, &Progress::new())
                .unwrap();
            assert_eq!(rows(t), want, "round {round} {keep:?} {a:?} {b:?}");
        }
    }
}

#[test]
fn only_shown_rows_count_and_changes_are_refused() {
    let mut a = doc("k,f\r\n1,y\r\n2,n\r\n3,y\r\n");
    let b = doc("k,f\r\n2,y\r\n3,y\r\n");
    let filter = FilterSpec {
        keep_first: true,
        columns: vec![ColumnFilter {
            enabled: true,
            negate: false,
            name: None,
            col: 1,
            rule: Rule::Values {
                values: vec!["y".into()],
                blanks: false,
            },
        }],
        ..Default::default()
    };
    let plan = a.plan_filter(&filter, &Progress::new()).unwrap();
    a.apply_filter(plan).unwrap();
    let r = run(&[&a, &b], &spec(2, &[0]));
    assert_eq!(r.report.lists[0].rows, 2);
    assert_eq!(r.report.slices[0].keys, 1);
    assert_eq!(r.report.slices[1].keys, 1);
    let before = a.revision();
    a.clear_filter();
    assert_ne!(a.revision(), before);
    assert!(a
        .sets_slice_table(&list(&[0]), &r.row_masks[0], 1, &Progress::new())
        .is_err());
}

#[test]
fn slices_have_names_and_a_table() {
    let names: Vec<String> = ["A", "B", "C"].map(String::from).to_vec();
    assert_eq!(slice_name(1, &names), "A only");
    assert_eq!(slice_name(3, &names), "A and B, not C");
    assert_eq!(slice_name(7, &names), "A, B and C");
    assert_eq!(slice_name(3, &names[..2]), "A and B");
    let a = doc("k\r\n1\r\n2\r\n");
    let b = doc("k,x\r\n2,y\r\n,z\r\n");
    let r = run(&[&a, &b], &spec(2, &[0]));
    let t = rows(report_table(&r.report, &names[..2]));
    assert_eq!(
        t,
        [
            vec!["Slice", "Different keys", "Rows in A", "Rows in B"],
            vec!["A only", "1", "1", "0"],
            vec!["A and B", "1", "1", "1"],
            vec!["Total of A", "2", "2", ""],
            vec!["Blank keys in A", "", "0", ""],
            vec!["Total of B", "1", "", "1"],
            vec!["Blank keys in B", "", "", "1"],
        ]
    );
}

#[test]
fn bad_specs_budget_and_cancel() {
    let a = doc("k\r\n1\r\n");
    let p = Progress::new();
    let refused = |s: SetsSpec| s.check().unwrap_err().to_string();
    assert!(refused(spec(1, &[0])).contains("2 to 5"));
    assert!(refused(spec(6, &[0])).contains("2 to 5"));
    assert!(refused(spec(2, &[])).contains("key columns"));
    let mut uneven = spec(2, &[0]);
    uneven.lists[1].cols.push(0);
    assert!(refused(uneven).contains("same number"));
    assert!(matches!(
        compare_lists(&[&a, &a], &spec(2, &[3]), &p),
        Err(Error::OutOfRange)
    ));
    let many = doc("k,x\r\n1,a\r\n2,b\r\n3,c\r\n");
    let mut b = SetsBuilder::new(&spec(2, &[0]), vec![false]).unwrap();
    b.max_keys = 2;
    assert!(b
        .add(&many, &p)
        .unwrap_err()
        .to_string()
        .contains("different keys"));
    let mut b = SetsBuilder::new(&spec(2, &[0]), vec![false]).unwrap();
    b.max_bytes = 100;
    assert!(matches!(b.add(&many, &p), Err(Error::OutOfMemory(_))));
    p.cancel.store(true, Ordering::Relaxed);
    assert!(matches!(
        compare_lists(&[&many, &many], &spec(2, &[0]), &p),
        Err(Error::Cancelled)
    ));
}
