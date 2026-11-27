use super::*;
use crate::OpenOptions;
use std::sync::Arc;

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

fn rows(d: &Document) -> Vec<String> {
    (0..d.row_count())
        .map(|r| {
            (0..d.col_count())
                .map(|c| d.cell(r, c).unwrap_or_default())
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect()
}

fn p() -> Progress {
    Progress::new()
}

const A: &str = "id,city,zip\r\n1,Ames,50010\r\n2,Bly,97622\r\n3,Cody,82414\r\n";

#[test]
fn columns_go_in_with_their_titles_and_one_undo() {
    let src = doc(A);
    let mut dst = doc("name,state\r\nAnn,IA\r\nBo,OR\r\nCy,WY\r\n");
    let out = src.take_cols(&[1, 2], true, &p()).unwrap();
    assert_eq!(out.titles, ["city", "zip"]);
    assert_eq!((out.rows, out.cols[1].get(2)), (3, "82414"));
    let before = rows(&dst);
    let (plan, rep) = dst.plan_insert_cols(1, &out, true, &p()).unwrap();
    assert_eq!(
        (rep.cols, rep.rows, rep.added_rows, rep.lengths_differ),
        (2, 3, 0, false)
    );
    dst.apply_join(plan).unwrap();
    assert_eq!(
        rows(&dst),
        [
            "name,city,zip,state",
            "Ann,Ames,50010,IA",
            "Bo,Bly,97622,OR",
            "Cy,Cody,82414,WY"
        ]
    );
    let after = rows(&dst);
    assert!(dst.undo());
    assert_eq!(rows(&dst), before, "one Ctrl+Z takes it all out");
    assert!(dst.redo());
    assert_eq!(rows(&dst), after);
}

#[test]
fn no_title_row_there_leaves_the_titles_out() {
    let src = doc(A);
    let mut dst = doc("Ann,IA\r\nBo,OR\r\nCy,WY\r\n");
    let out = src.take_cols(&[1], true, &p()).unwrap();
    let (plan, rep) = dst.plan_insert_cols(2, &out, false, &p()).unwrap();
    assert!(rep.title_left_out);
    dst.apply_join(plan).unwrap();
    assert_eq!(rows(&dst), ["Ann,IA,Ames", "Bo,OR,Bly", "Cy,WY,Cody"]);
}

#[test]
fn a_longer_column_grows_the_target_and_undo_takes_the_rows_back() {
    let src = doc(A);
    let mut dst = doc("name\r\nAnn\r\n");
    let out = src.take_cols(&[1], true, &p()).unwrap();
    let (plan, rep) = dst.plan_insert_cols(1, &out, true, &p()).unwrap();
    assert_eq!((rep.added_rows, rep.lengths_differ), (2, true));
    dst.apply_join(plan).unwrap();
    assert_eq!(rows(&dst), ["name,city", "Ann,Ames", ",Bly", ",Cody"]);
    assert!(dst.undo());
    assert_eq!(rows(&dst), ["name", "Ann"]);
    // A shorter one leaves the cells below empty.
    let mut big = doc("n\r\n1\r\n2\r\n3\r\n4\r\n5\r\n");
    let (plan, rep) = big.plan_insert_cols(1, &out, true, &p()).unwrap();
    assert!(rep.lengths_differ && rep.added_rows == 0);
    big.apply_join(plan).unwrap();
    assert_eq!(rows(&big)[4..], ["4,", "5,"]);
}

#[test]
fn hidden_rows_of_the_source_are_carried_and_counted() {
    let mut src = doc(A);
    src.hide_rows(2, 1).unwrap(); // Bly
    assert_eq!(src.row_count(), 3);
    let out = src.take_cols(&[1], true, &p()).unwrap();
    assert_eq!((out.rows, out.hidden), (3, 1));
    assert_eq!(out.cols[0].get(1), "Bly");
}

#[test]
fn protection_follows_the_columns_it_was_on() {
    let src = doc(A);
    let mut dst = doc("name,state\r\nAnn,IA\r\n");
    dst.set_protected(&[1]);
    let out = src.take_cols(&[1], true, &p()).unwrap();
    let (plan, _) = dst.plan_insert_cols(1, &out, true, &p()).unwrap();
    dst.apply_join(plan).unwrap();
    assert_eq!(
        dst.protected_cols(),
        [2],
        "state moved right and kept its protection"
    );
    assert_eq!(dst.cell(1, 1).as_deref(), Some("Ames"));
}

#[test]
fn rows_go_by_position_or_under_matching_titles() {
    let src = doc(A);
    let out = src.take_rows(&[1, 3], true, &p()).unwrap();
    assert_eq!(out.rows, 2);
    // Same titles in another order: by title.
    let mut dst = doc("zip,id,city\r\n10001,9,Nyc\r\n");
    let (plan, rep) = dst
        .plan_insert_rows(None, &out, true, Wider::AddColumns, &p())
        .unwrap();
    assert!(rep.by_title);
    dst.apply_join(plan).unwrap();
    assert_eq!(
        rows(&dst),
        ["zip,id,city", "10001,9,Nyc", "50010,1,Ames", "82414,3,Cody"]
    );
    // After a chosen row.
    let mut dst = doc("id,city,zip\r\n7,X,1\r\n8,Y,2\r\n");
    let (plan, rep) = dst
        .plan_insert_rows(Some(1), &out, true, Wider::AddColumns, &p())
        .unwrap();
    assert!(!rep.by_title, "same order: nothing to say");
    dst.apply_join(plan).unwrap();
    assert_eq!(
        rows(&dst),
        [
            "id,city,zip",
            "7,X,1",
            "1,Ames,50010",
            "3,Cody,82414",
            "8,Y,2"
        ]
    );
    assert!(dst.undo());
    assert_eq!(rows(&dst), ["id,city,zip", "7,X,1", "8,Y,2"]);
}

#[test]
fn rows_by_title_is_refused_unless_the_sets_match_cleanly() {
    let out = doc("a,b,c\r\n1,2,3\r\n")
        .take_rows(&[1], true, &p())
        .unwrap();
    // Each refusal falls back to by position.
    for (why, t) in [
        ("a title missing", "a,b,x\r\n"),
        ("one more column", "a,b,c,d\r\n"),
        ("a title twice", "a,a,b\r\n"),
        ("the same apart from case", "a,B,b\r\n"),
        ("a blank title", "a,,c\r\n"),
    ] {
        assert!(doc(t).rows_by_title(&out).is_none(), "{why}");
    }
    let twice = doc("a,a,b\r\n1,2,3\r\n")
        .take_rows(&[1], true, &p())
        .unwrap();
    assert!(
        doc("a,b,a\r\n").rows_by_title(&twice).is_none(),
        "twice in the source"
    );
    // Ignoring case is fine.
    assert_eq!(doc("C,A,B\r\n").rows_by_title(&out), Some(vec![1, 2, 0]));
}

#[test]
fn wider_rows_add_columns_or_are_cut() {
    let out = doc("a,b,c\r\n1,2,3\r\n")
        .take_rows(&[1], false, &p())
        .unwrap();
    let mut add = doc("x\r\n9\r\n");
    let (plan, rep) = add
        .plan_insert_rows(None, &out, false, Wider::AddColumns, &p())
        .unwrap();
    assert_eq!(rep.added_cols, 2);
    add.apply_join(plan).unwrap();
    assert_eq!(rows(&add), ["x,,", "9,,", "1,2,3"]);
    let mut cut = doc("x\r\n9\r\n");
    let (plan, _) = cut
        .plan_insert_rows(None, &out, false, Wider::Cut, &p())
        .unwrap();
    cut.apply_join(plan).unwrap();
    assert_eq!(rows(&cut), ["x", "9", "1"]);
}

#[test]
fn a_cancel_copies_out_nothing() {
    let mut text = String::from("a,b\r\n");
    for i in 0..20_000 {
        text.push_str(&format!("{i},v{i}\r\n"));
    }
    let src = doc(&text);
    let prog = Arc::new(Progress::new());
    prog.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        src.take_cols(&[1], true, &prog),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        src.take_rows(&(1..10_000).collect::<Vec<_>>(), true, &prog),
        Err(Error::Cancelled)
    ));
}

#[test]
fn packed_cells_read_back() {
    let mut pk = Packed::default();
    for v in ["", "é", "", "abc"] {
        pk.push(v);
    }
    assert_eq!(
        (pk.len(), pk.get(0), pk.get(1), pk.get(2), pk.get(3)),
        (4, "", "é", "", "abc")
    );
}
