//! "Show me the rule" (C10). Made-up data only.

use super::*;
use crate::example::learn;
use crate::expr::{to_text, Ctx, Expr};
use crate::recipe::RecipeOp;
use crate::OpenOptions;
use proptest::prelude::*;
use std::borrow::Cow;

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

fn spec(col: u32) -> ExampleSpec {
    ExampleSpec {
        col,
        skip_first: true,
        ..Default::default()
    }
}

/// first,last,email,short: the short form is "first letter of first + . + last".
const NAMES: &str = "first,last,email,short\r\nAnn,Moss,ann.moss@example.com,A.Moss\r\nBo,Lane,bo.lane@example.com,B.Lane\r\nCy,Hart,cy.hart@example.com,\r\nDi,Fox,di.fox@example.com,\r\nEd,Ray,ed.ray@example.com,\r\n";

fn col_values(d: &Document, col: u32) -> Vec<String> {
    (1..d.row_count())
        .map(|r| d.cell(r, col).unwrap())
        .collect()
}

#[test]
fn learn_describe_and_read_back_give_the_same_rule() {
    let d = doc(NAMES);
    let rep = d.example_learn(&spec(3)).unwrap();
    let p = rep.program.clone().unwrap();
    assert_eq!(
        rep.words,
        "first letter of first + \".\" + last".to_string()
    );
    let json = serde_json::to_string(&p).unwrap();
    let back: Program = serde_json::from_str(&json).unwrap();
    assert_eq!(back, p);
    assert_eq!(rep.examples, 2);
    assert!(rep.conflict.is_none());
}

#[test]
fn an_edited_rule_previews_what_apply_writes() {
    let mut d = doc(NAMES);
    let mut p = d.example_learn(&spec(3)).unwrap().program.unwrap();
    // Capitals for the last name.
    if let Some(Piece::From { case, .. }) = p.pieces.last_mut() {
        *case = Case::Upper;
    }
    let pre = d.example_preview(&spec(3), &p).unwrap();
    let shown: Vec<String> = pre.rows.iter().map(|r| r.result.clone().unwrap()).collect();
    assert_eq!(shown, ["C.HART", "D.FOX", "E.RAY"]);
    assert_eq!(pre.rows[0].row, 3);
    assert_eq!(pre.rows[0].reads, [(0, "Cy".into()), (1, "Hart".into())]);
    let s = ExampleSpec {
        program: Some(p),
        ..spec(3)
    };
    let (plan, rep) = d.plan_fill_example(&s, &Progress::new()).unwrap();
    assert_eq!(rep.filled, 3);
    d.apply_cells(plan).unwrap();
    assert_eq!(
        col_values(&d, 3),
        ["A.Moss", "B.Lane", "C.HART", "D.FOX", "E.RAY"]
    );
}

#[test]
fn the_preview_reads_only_the_first_rows() {
    let mut t = String::from("a,b\r\nx1,X1\r\n");
    for i in 2..5_000 {
        t += &format!("x{i},\r\n");
    }
    let d = doc(&t);
    let p = d.example_learn(&spec(1)).unwrap().program.unwrap();
    let pre = d.example_preview(&spec(1), &p).unwrap();
    assert_eq!(pre.rows.len(), PREVIEW_ROWS);
    assert_eq!(pre.looked, PREVIEW_LOOK);
}

#[test]
fn odd_results_are_found() {
    // code = the part of id before "-". Three odd ones: a piece that comes out empty ("-77": not
    // filled), another shape ("AB.12"), and another shape far longer than the examples.
    let d = doc("id,code
A.B-1,A.B
C.D-2,C.D
E.F-3,E.F
-77,
AB.12-3,
ABCDEFGHIJ.KLMNOP-9,
G.H-4,
xy,
");
    let p = d.example_learn(&spec(1)).unwrap().program.unwrap();
    let o = d
        .example_outliers(&spec(1), &p, None, &Progress::new())
        .unwrap();
    assert_eq!((o.fills, o.missed), (3, 2), "{p:?} {o:?}");
    let rows: Vec<u64> = o.rows.iter().map(|x| x.row).collect();
    assert_eq!(rows, [5, 6]);
    assert_eq!((o.odd_shape, o.odd_length), (2, 1));
    let mut d = d;
    let (plan, rep) = d.plan_fill_example(&spec(1), &Progress::new()).unwrap();
    assert_eq!(rep.program, p);
    d.apply_cells(plan).unwrap();
    let after = d
        .example_outliers(&spec(1), &p, Some(&rep.example_ids), &Progress::new())
        .unwrap();
    assert_eq!(
        (after.fills, after.odd_rows),
        (3, 2),
        "after the fill, the same odd cells: {after:?}"
    );
}

#[test]
fn an_empty_piece_inside_a_result_is_odd() {
    // out = piece 1 + "/" + piece 2 (split at "-"); "e f--z" gives an empty piece 2.
    let d = doc("v,out
a b-c d-x,a b/c d
g h-i j-y,g h/i j
e f--z,
k l-m n-w,
");
    let p = d.example_learn(&spec(1)).unwrap().program.unwrap();
    let o = d
        .example_outliers(&spec(1), &p, None, &Progress::new())
        .unwrap();
    assert_eq!(o.empty_piece, 1, "{p:?} {o:?}");
    assert_eq!(o.rows[0].row, 3);
}

/// first,last,short with typos in the examples (made-up names).
fn conflict_doc(shorts: &[&str]) -> Document {
    let names = [
        ("Ann", "Moss"),
        ("Bo", "Lane"),
        ("Cy", "Hart"),
        ("Di", "Fox"),
        ("Ed", "Ray"),
        ("Flo", "Oak"),
    ];
    let mut t = String::from("first,last,short\r\n");
    for (i, (f, l)) in names.iter().enumerate() {
        t += &format!("{f},{l},{}\r\n", shorts.get(i).copied().unwrap_or(""));
    }
    doc(&t)
}

#[test]
fn the_example_that_breaks_the_rule_is_named() {
    // A typo in the last example.
    let d = conflict_doc(&["A Moss", "B Lane", "C Hart", "Di Fox"]);
    let rep = d.example_learn(&spec(2)).unwrap();
    let c = rep.conflict.unwrap();
    assert_eq!((c.row, c.value.as_str()), (4, "Di Fox"));
    assert_eq!(rep.words, "first letter of first + \" \" + last");
    // A typo in the first.
    let d = conflict_doc(&["Ann Moss", "B Lane", "C Hart", "D Fox"]);
    let c = d.example_learn(&spec(2)).unwrap().conflict.unwrap();
    assert_eq!((c.row, c.value.as_str()), (1, "Ann Moss"));
    // Two bad ones: none is named, as before.
    let d = conflict_doc(&["Ann Moss", "B Lane", "C Hart", "Di Fox"]);
    let rep = d.example_learn(&spec(2)).unwrap();
    assert!(rep.conflict.is_none() && rep.program.is_none());
}

#[test]
fn leave_it_out_learns_without_it_and_keeps_its_cell() {
    let mut d = conflict_doc(&["A Moss", "B Lane", "C Hart", "Di Fox"]);
    let c = d.example_learn(&spec(2)).unwrap().conflict.unwrap();
    let s = ExampleSpec {
        leave_out: vec![c.id],
        ..spec(2)
    };
    let (plan, rep) = d.plan_fill_example(&s, &Progress::new()).unwrap();
    assert_eq!(rep.filled, 2);
    d.apply_cells(plan).unwrap();
    assert_eq!(
        col_values(&d, 2),
        ["A Moss", "B Lane", "C Hart", "Di Fox", "E Ray", "F Oak"]
    );
}

#[test]
fn no_time_names_nothing() {
    let d = conflict_doc(&["A Moss", "B Lane", "C Hart", "Di Fox"]);
    let ex = d.example_rows(&spec(2)).unwrap();
    let got = learn_or_conflict(&ex.examples(), &ex.sources, std::time::Duration::ZERO).unwrap();
    assert!(got.conflict.is_none() && got.program.is_none());
}

#[test]
fn bad_rules_are_refused() {
    let d = doc(NAMES);
    let p = d.example_learn(&spec(3)).unwrap().program.unwrap();
    assert!(p.validate(4, 3).is_ok());
    // A column moved or deleted since: out of the table.
    let err = p.validate(1, 3).unwrap_err().to_string();
    assert!(err.contains("learn it again"), "{err}");
    let reads_itself = Program {
        pieces: vec![Piece::From {
            col: 3,
            cut: Cut::Whole,
            index: 0,
            first: false,
            case: Case::Keep,
        }],
    };
    assert!(reads_itself.validate(4, 3).is_err());
    let letter_split = Program {
        pieces: vec![Piece::From {
            col: 0,
            cut: Cut::Split { by: 'x' },
            index: 0,
            first: false,
            case: Case::Keep,
        }],
    };
    assert!(letter_split.validate(4, 3).is_err());
    let many = Program {
        pieces: vec![Piece::Text { text: "-".into() }; MAX_PIECES + 1],
    };
    assert!(many.validate(4, 3).is_err());
    assert!(Program { pieces: vec![] }.validate(4, 3).is_err());
}

#[test]
fn a_changed_table_refuses_apply() {
    let mut d = doc(NAMES);
    let rep = d.example_learn(&spec(3)).unwrap();
    d.set_cells(vec![(1, 0, "Anne".into())]).unwrap();
    let s = ExampleSpec {
        program: rep.program,
        revision: Some(rep.revision),
        ..spec(3)
    };
    let err = d
        .plan_fill_example(&s, &Progress::new())
        .unwrap_err()
        .to_string();
    assert!(err.contains("the table changed"), "{err}");
}

#[test]
fn recipes_keep_the_rule() {
    let p = doc(NAMES).example_learn(&spec(3)).unwrap().program.unwrap();
    // A step with a rule replays it with no examples to learn from.
    let mut d =
        doc("first,last,email,short\r\nGus,Elm,g@example.com,\r\nHal,Ash,h@example.com,\r\n");
    let op = RecipeOp::FillExample {
        spec: ExampleSpec {
            program: Some(p.clone()),
            ..spec(3)
        },
    };
    let json = serde_json::to_string(&op).unwrap();
    let op: RecipeOp = serde_json::from_str(&json).unwrap();
    d.run_recipe_step(&op, 0.0, 0, &Progress::new()).unwrap();
    assert_eq!(col_values(&d, 3), ["G.Elm", "H.Ash"]);
    // An old step (no rule) learns again: here there is nothing to learn from.
    let old: RecipeOp =
        serde_json::from_str(r#"{"op":"fill_example","spec":{"col":3,"skip_first":true}}"#)
            .unwrap();
    let mut d2 = doc("first,last,email,short\r\nGus,Elm,g@example.com,\r\n");
    assert!(d2.run_recipe_step(&old, 0.0, 0, &Progress::new()).is_err());
    // A rule that no longer fits the table is the step's error.
    let mut d3 = doc("first\r\nGus\r\n");
    assert!(d3.run_recipe_step(&op, 0.0, 0, &Progress::new()).is_err());
}

// ---- the formula form ----

fn eval(expr: &str, row: &[String]) -> String {
    let names: Vec<String> = (0..row.len()).map(|i| format!("c{i}")).collect();
    let e = Expr::parse(expr, &names).unwrap_or_else(|err| panic!("{expr}: {err}"));
    let cells: Vec<Cow<str>> = e
        .columns()
        .iter()
        .map(|&c| Cow::Borrowed(row[c as usize].as_str()))
        .collect();
    let slots: Vec<Option<usize>> = (0..cells.len()).map(Some).collect();
    let ctx = Ctx {
        cells: &cells,
        slots: &slots,
        row: 1,
        now: 0.0,
        read: Default::default(),
        prev: None,
        next: None,
        refs: None,
        types: &[],
    };
    to_text(&e.eval(&ctx)).into_owned()
}

fn names(c: u32) -> String {
    format!("c{c}")
}

#[test]
fn a_learned_rule_as_a_formula() {
    let p = doc(NAMES).example_learn(&spec(3)).unwrap().program.unwrap();
    let f = p.to_expression(&names, &|_| false).unwrap();
    let row = |a: &str, b: &str| vec![a.to_string(), b.to_string(), String::new(), String::new()];
    assert_eq!(eval(&f, &row("Cy", "Hart")), "C.Hart");
    assert_eq!(
        eval(&f, &row("", "Hart")),
        "",
        "a missing piece: empty, as the fill leaves it"
    );
    // A column read as numbers has no formula form.
    let why = p.to_expression(&names, &|c| c == 1).unwrap_err();
    assert!(why.contains("numbers or dates"), "{why}");
}

fn cell() -> impl Strategy<Value = String> {
    let atoms = prop::sample::select(vec![
        "ann", "Bo", "MOSS", "é", "ß", "İ", "ı", "😀", "42", "007", "x1", "o'neil", "-", "@", ".",
        "/", " ", "  ", "\t", "\u{a0}", "²", "",
    ]);
    prop::collection::vec(atoms, 0..7).prop_map(|v| v.concat())
}

fn piece() -> impl Strategy<Value = Piece> {
    let cut = prop_oneof![
        Just(Cut::Whole),
        Just(Cut::Part),
        Just(Cut::Word),
        Just(Cut::Number),
        Just(Cut::Run),
        prop::sample::select(vec!['-', '@', '.', '/']).prop_map(|by| Cut::Split { by }),
    ];
    let case = prop::sample::select(vec![Case::Keep, Case::Upper, Case::Lower, Case::Title]);
    prop_oneof![
        1 => "[a-z .\"'-]{1,3}".prop_map(|text| Piece::Text { text }),
        4 => (0u32..3, cut, -4i32..4, any::<bool>(), case).prop_map(|(col, cut, index, first, case)| Piece::From {
            col,
            cut,
            index: if cut == Cut::Whole { 0 } else { index },
            first,
            case,
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]
    /// Every rule that has a formula form gives the same result as the rule (an empty cell when a
    /// piece is missing), on made-up values with accents, ß, Turkish İ and ı, emoji, tabs and
    /// non-breaking spaces (25 rows each: 10,000 in all).
    #[test]
    fn the_formula_gives_what_the_rule_gives(
        pieces in prop::collection::vec(piece(), 1..4),
        rows in prop::collection::vec(prop::collection::vec(cell(), 3), 25),
    ) {
        let p = Program { pieces };
        let Ok(f) = p.to_expression(&names, &|_| false) else { return Ok(()); };
        for row in &rows {
            let want = p.run(&|c| row.get(c as usize).cloned()).unwrap_or_default();
            prop_assert_eq!(eval(&f, row), want, "{} on {:?}", f, row);
        }
    }
}

#[test]
fn learning_is_unchanged_for_the_old_tests() {
    // The examples the fill learns from are the same as before: the first filled cells.
    let ex: Vec<Example> = vec![Example {
        row: vec!["Ann", "Moss"],
        out: "A.Moss",
    }];
    assert_eq!(
        learn(&ex, &[0, 1]).unwrap().unwrap().describe(&names),
        "first letter of c0 + \".\" + c1"
    );
}

#[test]
fn the_formula_for_a_table() {
    let mut d = doc("first,last,num,short\r\nAnn,Lee,1,A.Lee\r\nBo,Kim,2,B.Kim\r\nCy,Hart,3,\r\n");
    let p = d.example_learn(&spec(3)).unwrap().program.unwrap();
    let read = crate::value::ReadOptions::default();
    let f = d.example_formula(&spec(3), &p, read).unwrap();
    assert!(f.contains("[first]") && f.contains("[last]"), "{f}");
    // Same titles: refused with the reason.
    let same = doc("x,x,num,short\r\nAnn,Lee,1,A.Lee\r\nBo,Kim,2,B.Kim\r\n");
    let q = same.example_learn(&spec(3)).unwrap().program.unwrap();
    let why = same
        .example_formula(&spec(3), &q, read)
        .unwrap_err()
        .to_string();
    assert!(why.contains("same title"), "{why}");
    // A column read as numbers: refused.
    d.set_column_types(crate::coltype::ColumnTypes::from_list(vec![
        crate::coltype::ColumnType::Number,
    ]));
    let why = d
        .example_formula(&spec(3), &p, read)
        .unwrap_err()
        .to_string();
    assert!(why.contains("numbers or dates"), "{why}");
}
