//! Positions custom row heights follow while a filter is on or rows are
//! hidden: shown rows of document positions, document positions of shown
//! rows, and the positions an insert or delete changed (gap #4).

use natendo_engine::filter::FilterSpec;
use natendo_engine::{Document, OpenOptions};

/// 12 rows (row 0 the titles); `k` is "x" on rows 2, 3, 6, 7, 8 and 11.
fn doc() -> Document {
    let mut s = String::from("n,k\n");
    for r in 1..12 {
        let k = if [2, 3, 6, 7, 8, 11].contains(&r) {
            "x"
        } else {
            "y"
        };
        s.push_str(&format!("r{r},{k}\n"));
    }
    Document::from_bytes(s.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
}

fn only(value: &str) -> FilterSpec {
    serde_json::from_str(&format!(
        r#"{{"columns":[{{"col":1,"rule":{{"type":"values","values":["{value}"],"blanks":false}}}}],"keep_first":true}}"#
    ))
    .unwrap()
}

/// The first column of every row, in document order.
fn names(d: &mut Document) -> Vec<String> {
    let spec = only("x");
    let had = d.is_filtered();
    if had {
        d.clear_filter();
    }
    let out = (0..d.row_count())
        .map(|r| d.row_values(r).unwrap()[0].clone())
        .collect();
    if had {
        d.set_filter(&spec).unwrap();
    }
    out
}

#[test]
fn shown_rows_and_document_rows_answer_each_other() {
    let mut d = doc();
    // Every row shown: the positions themselves, None past the end.
    assert_eq!(
        d.shown_rows_at(&[0, 5, 11, 12]),
        [Some(0), Some(5), Some(11), None]
    );
    assert_eq!(d.doc_rows(3, 2), [3, 4]);
    d.set_filter(&only("x")).unwrap();
    // Shown: 0 (titles), 2, 3, 6, 7, 8, 11.
    assert_eq!(d.doc_rows(0, 100), [0, 2, 3, 6, 7, 8, 11]);
    assert_eq!(d.doc_rows(2, 2), [3, 6]);
    assert!(d.doc_rows(9, 2).is_empty());
    assert_eq!(
        d.shown_rows_at(&[0, 1, 2, 6, 11, 40]),
        [Some(0), None, Some(1), Some(3), Some(6), None]
    );
    // Rows hidden by hand too.
    d.clear_filter();
    d.hide_rows(4, 3).unwrap();
    assert_eq!(d.shown_rows_at(&[3, 4, 7]), [Some(3), None, Some(4)]);
}

#[test]
fn an_insert_reports_where_its_rows_went() {
    let mut d = doc();
    d.set_filter(&only("x")).unwrap();
    // Before the shown row 3 (document row 6).
    assert_eq!(d.insert_rows_at(3, vec![vec!["new".into()]]).unwrap(), 6);
    // After the last shown row (document row 12 now): appended after it.
    let shown = d.row_count();
    assert_eq!(
        d.insert_rows_at(shown, vec![vec!["end".into()]]).unwrap(),
        13
    );
    let all = names(&mut d);
    assert_eq!(all[6], "new");
    assert_eq!(all[13], "end");
    // Nothing shown but the titles... and with nothing shown at all.
    let mut e = doc();
    e.set_filter(&serde_json::from_str::<FilterSpec>(
        r#"{"columns":[{"col":1,"rule":{"type":"values","values":["none"],"blanks":false}}],"keep_first":false}"#,
    ).unwrap()).unwrap();
    assert_eq!(e.row_count(), 0);
    let at = e.insert_rows_at(0, vec![vec!["only".into()]]).unwrap();
    assert_eq!(at, 12, "appended after the last row");
    // Without a filter, the position asked for.
    let mut f = doc();
    assert_eq!(f.insert_rows_at(4, vec![vec!["a".into()]; 2]).unwrap(), 4);
}

#[test]
fn a_delete_reports_its_runs_over_the_gaps() {
    let mut d = doc();
    d.set_filter(&only("x")).unwrap();
    // Shown rows 1..6 are document rows 2, 3, 6, 7, 8, 11.
    let runs = d.delete_rows_at(1, 5).unwrap();
    assert_eq!(runs, [(2, 2), (6, 3)]);
    let all = names(&mut d);
    assert_eq!(all, ["n", "r1", "r4", "r5", "r9", "r10", "r11"]);
    // Without a filter: one run.
    let mut e = doc();
    assert_eq!(e.delete_rows_at(3, 4).unwrap(), [(3, 4)]);
    assert!(e.delete_rows_at(0, 0).unwrap().is_empty());
}
