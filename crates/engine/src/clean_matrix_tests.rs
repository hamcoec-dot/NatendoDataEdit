use super::*;
use crate::filter::{ColumnFilter, FilterSpec, Rule};
use crate::OpenOptions;
use std::sync::atomic::Ordering;

fn doc(b: &str) -> Document {
    Document::from_bytes(b.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap()
}

fn rows(d: &Document) -> Vec<Vec<String>> {
    (0..d.row_count())
        .map(|r| d.row_values(r).unwrap())
        .collect()
}

const NOW: f64 = 20_783.5;

fn entry(col: u32, steps: Vec<Step>) -> MatrixEntry {
    MatrixEntry {
        col,
        title: String::new(),
        steps,
        from_file: String::new(),
    }
}

fn matrix(entries: Vec<MatrixEntry>) -> CleanMatrix {
    CleanMatrix {
        entries,
        skip_first: true,
        read: ReadOptions::default(),
    }
}

fn sp(how: Spaces) -> Step {
    Step::Spaces { how }
}

fn case(to: CaseTo) -> Step {
    Step::Case { to }
}

fn sanitize() -> Step {
    Step::Sanitize {
        options: crate::xray::SanitizeOptions::usual(),
    }
}

/// Plan and apply: the cells changed and each column's counts.
fn run(d: &mut Document, m: &CleanMatrix) -> (u64, Vec<MatrixColumn>) {
    let p = d.plan_clean_matrix(m, NOW, 7, &Progress::new()).unwrap();
    let out = (p.plan.changed, p.columns);
    d.apply_clean(p.plan).unwrap();
    out
}

/// The same entries as today's runs: one `plan_clean` per column.
fn one_by_one(d: &mut Document, m: &CleanMatrix) -> u64 {
    let mut changed = 0;
    for e in &m.entries {
        let spec = CleanSpec {
            cleaner: e.cleaner(),
            scope: Scope::Columns { cols: vec![e.col] },
            skip_first: m.skip_first,
            read: m.read,
        };
        let p = d.plan_clean(&spec, NOW, 7, 0, &Progress::new()).unwrap();
        changed += p.changed;
        d.apply_clean(p).unwrap();
    }
    changed
}

#[test]
fn a_matrix_equals_the_columns_cleaned_one_by_one() {
    const CELLS: [&str; 12] = [
        "",
        "  ann  smith ",
        "BOB\u{a0}O'BRIEN",
        "Sm\u{0456}th\u{200b}",
        "a\r\nb",
        "12/31/2025",
        "2025-02-03 10:00",
        "(423) 555-0123 x",
        "\u{201c}q\u{201d}\u{7}",
        "ÉMILE  zola",
        "x",
        "00123",
    ];
    let steps = [
        sanitize(),
        Step::Remove {
            what: Remove::Kind {
                of: Kind::Invisible,
            },
        },
        sp(Spaces::LineBreaks),
        sp(Spaces::Trim),
        sp(Spaces::Collapse),
        Step::Digits { keep: true },
        case(CaseTo::Upper),
        case(CaseTo::Lower),
        case(CaseTo::Title),
        Step::Dates {
            format: "yyyy-mm-dd".into(),
            from: String::new(),
            past: false,
            ambiguous: None,
            review: None,
        },
        Step::Number {
            start: 1.0,
            step: 1.0,
            digits: 3,
            at_end: true,
            separator: "-".into(),
            replace: false,
        },
        Step::Random {
            length: 4,
            chars: Charset::Letters,
        },
    ];
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n) as usize
    };
    for round in 0..150 {
        let cols = next(5) + 1;
        let n = next(30) + 1;
        let mut text = String::new();
        for r in 0..=n {
            let line: Vec<String> = (0..cols)
                .map(|c| {
                    let v = if r == 0 {
                        format!(" Title {c} ")
                    } else if c == 0 {
                        format!("{}", next(3))
                    } else {
                        CELLS[next(CELLS.len() as u64)].to_string()
                    };
                    format!("\"{}\"", v.replace('"', "\"\""))
                })
                .collect();
            text.push_str(&line.join(","));
            text.push_str("\r\n");
        }
        let mut entries = Vec::new();
        for c in 0..cols as u32 {
            if next(3) == 0 && c > 0 {
                continue;
            }
            let mut own = Vec::new();
            for _ in 0..next(4) + 1 {
                own.push(steps[next(steps.len() as u64)].clone());
            }
            entries.push(entry(c, own));
        }
        // The order the entries are listed in changes nothing.
        entries.reverse();
        let mut m = matrix(entries);
        m.skip_first = round % 2 == 0;
        let (mut a, mut b) = (doc(&text), doc(&text));
        if round % 3 == 0 {
            let filter = FilterSpec {
                keep_first: m.skip_first,
                columns: vec![ColumnFilter {
                    col: 0,
                    rule: Rule::Values {
                        values: vec!["1".into(), "2".into()],
                        blanks: false,
                    },
                }],
                ..Default::default()
            };
            for d in [&mut a, &mut b] {
                let p = d.plan_filter(&filter, &Progress::new()).unwrap();
                d.apply_filter(p).unwrap();
            }
        }
        let before = a.row_count();
        let preview = a
            .preview_clean_matrix(&m, NOW, 7, &Progress::new())
            .unwrap();
        let got = run(&mut a, &m);
        let singly = one_by_one(&mut b, &m);
        assert_eq!(a.row_count(), before);
        assert_eq!(rows(&a), rows(&b), "round {round}: {m:?}");
        assert_eq!(got.0, singly, "round {round}");
        assert_eq!(preview.columns, got.1, "round {round}");
    }
}

/// The matrix's date tick: the review screen's careful choice, the same
/// step Data > Clean runs.
fn iso_dates() -> Step {
    Step::Dates {
        format: "yyyy-mm-dd".into(),
        from: String::new(),
        past: false,
        ambiguous: Some(Ambiguous::Leave),
        review: Some(DateReview::default()),
    }
}

#[test]
fn the_date_tick_leaves_what_it_cannot_be_sure_of() {
    let cells = [
        "12/31/2025",
        "31.12.2025",
        "Jan 5, 2026",
        "03/04/2026",
        "20260105",
        "1/5/26",
        "soon",
        "2026-01-05",
    ];
    let mut text = String::from("d,x\r\n");
    for c in cells {
        text.push_str(&format!("\"{c}\",1\r\n"));
    }
    let mut d = doc(&text);
    let m = matrix(vec![entry(0, vec![sp(Spaces::Trim), iso_dates()])]);
    let preview = d
        .preview_clean_matrix(&m, NOW, 7, &Progress::new())
        .unwrap();
    // Counted although Trim ran first on the same cell: 03/04/2026 and
    // 1/5/26 can both be read two ways.
    assert_eq!(preview.columns[0].ambiguous_left, 2);
    let mut same = doc(&text);
    one_by_one(&mut same, &m);
    run(&mut d, &m);
    assert_eq!(rows(&d), rows(&same));
    let got: Vec<String> = rows(&d).iter().skip(1).map(|r| r[0].clone()).collect();
    assert_eq!(
        got,
        [
            "2025-12-31",
            "2025-12-31",
            "2026-01-05",
            "03/04/2026",
            "20260105",
            "1/5/26",
            "soon",
            "2026-01-05",
        ]
    );
    // The recorded step keeps the choice, whatever the file's day order.
    assert_eq!(m.recorded().entries[0].steps[1], iso_dates());
}

#[test]
fn one_undo_puts_every_column_back() {
    let text = "a,b\r\n x , y \r\n";
    let mut d = doc(text);
    let m = matrix(vec![
        entry(0, vec![sp(Spaces::Trim)]),
        entry(1, vec![sp(Spaces::Trim), case(CaseTo::Upper)]),
    ]);
    run(&mut d, &m);
    assert_eq!(rows(&d)[1], ["x", "Y"]);
    assert!(d.undo());
    assert_eq!(rows(&d)[1], [" x ", " y "]);
    assert!(!d.is_modified());
}

#[test]
fn protected_columns_are_counted_apart_and_left_alone() {
    let mut d = doc("a,b\r\n x , y \r\n z ,w\r\n");
    d.set_protected(&[1]);
    let m = matrix(vec![
        entry(0, vec![sp(Spaces::Trim)]),
        entry(1, vec![sp(Spaces::Trim)]),
    ]);
    let preview = d
        .preview_clean_matrix(&m, NOW, 7, &Progress::new())
        .unwrap();
    assert_eq!(
        (preview.columns[0].changed, preview.columns[0].protected),
        (2, 0)
    );
    assert_eq!(
        (preview.columns[1].changed, preview.columns[1].protected),
        (0, 1)
    );
    assert!(preview.examples.iter().all(|x| x.col == 0));
    assert_eq!(preview.examples.len(), 2);
    assert_eq!(preview.revision, d.revision());
    let p = d.plan_clean_matrix(&m, NOW, 7, &Progress::new()).unwrap();
    assert_eq!(p.plan.changed, 2);
    d.apply_clean(p.plan).unwrap();
    assert_eq!(d.protected_skipped(), 1);
    assert_eq!(rows(&d)[1], ["x", " y "]);
}

#[test]
fn the_preview_shows_three_changes_of_each_column() {
    let d = doc("a,b\r\n 1, 2\r\n 3, 4\r\n 5, 6\r\n 7, 8\r\n 9,10\r\n");
    let m = matrix(vec![
        entry(0, vec![sp(Spaces::Trim)]),
        entry(1, vec![sp(Spaces::Trim)]),
    ]);
    let p = d
        .preview_clean_matrix(&m, NOW, 7, &Progress::new())
        .unwrap();
    for c in 0..2 {
        assert_eq!(
            p.examples.iter().filter(|x| x.col == c).count(),
            MATRIX_EXAMPLES
        );
    }
    assert_eq!(p.columns[0].changed, 5);
    assert_eq!(p.columns[1].changed, 4);
}

#[test]
fn a_bad_matrix_is_refused() {
    let ok = entry(0, vec![sp(Spaces::Trim)]);
    let refused = |m: CleanMatrix| m.check().unwrap_err().to_string();
    assert!(refused(matrix(vec![])).contains("at least one"));
    assert!(refused(matrix(vec![ok.clone(), ok.clone()])).contains("twice"));
    assert!(refused(matrix(vec![entry(0, vec![])])).contains("nothing ticked"));
    assert!(refused(matrix(vec![entry(
        0,
        vec![sp(Spaces::Trim); MAX_STEPS + 1]
    )]))
    .contains("at most"));
    let scramble: Step = serde_json::from_str(r#"{"tool":"scramble"}"#).unwrap();
    assert!(refused(matrix(vec![entry(0, vec![scramble])])).contains("scramble"));
    let big = Step::Add {
        text: "x".repeat(MAX_REPLACEMENT_CHARS),
        at: At::End,
    };
    let many: Vec<MatrixEntry> = (0..MAX_MATRIX_BYTES / MAX_REPLACEMENT_CHARS + 2)
        .map(|c| entry(c as u32, vec![big.clone()]))
        .collect();
    assert!(refused(matrix(many)).contains("too large"));
    // Unknown tools and fields.
    for bad in [
        r#"{"entries":[{"col":0,"steps":[{"tool":"format_disk"}]}]}"#,
        r#"{"entries":[{"col":0,"steps":[],"run":"x"}]}"#,
        r#"{"entries":[],"shell":"x"}"#,
    ] {
        assert!(serde_json::from_str::<CleanMatrix>(bad).is_err(), "{bad}");
    }
    let d = doc("a\r\n1\r\n");
    assert!(matches!(
        d.plan_clean_matrix(
            &matrix(vec![entry(4, vec![sp(Spaces::Trim)])]),
            NOW,
            7,
            &Progress::new()
        ),
        Err(Error::OutOfRange)
    ));
}

#[test]
fn columns_are_found_by_title() {
    let mut e = entry(0, vec![sp(Spaces::Trim)]);
    e.title = "Name".into();
    let m = matrix(vec![e]);
    let moved = doc("id, name \r\n1,x\r\n");
    assert_eq!(moved.matrix_by_title(&m).unwrap().entries[0].col, 1);
    let missing = doc("id,town\r\n1,x\r\n");
    assert!(missing
        .matrix_by_title(&m)
        .unwrap_err()
        .to_string()
        .contains("no column titled \"Name\""));
    // Twice: only the saved place will do.
    let twice = doc("Name,name\r\n1,x\r\n");
    assert_eq!(twice.matrix_by_title(&m).unwrap().entries[0].col, 0);
    let elsewhere = doc("id,Name,name\r\n1,x,y\r\n");
    assert!(elsewhere
        .matrix_by_title(&m)
        .unwrap_err()
        .to_string()
        .contains("more than one"));
}

#[test]
fn cancel_leaves_no_plan() {
    let p = Progress::new();
    p.cancel.store(true, Ordering::Relaxed);
    let d = doc("a\r\n 1\r\n");
    let m = matrix(vec![entry(0, vec![sp(Spaces::Trim)])]);
    assert!(matches!(
        d.plan_clean_matrix(&m, NOW, 7, &p),
        Err(Error::Cancelled)
    ));
}

#[test]
fn a_recipe_step_runs_the_stored_order_on_a_moved_column() {
    use crate::recipe::{Recipe, RecipeOp, RECIPE_VERSION};
    let mut e = entry(0, vec![sp(Spaces::Trim), case(CaseTo::Upper)]);
    e.title = "Name".into();
    e.from_file = "names.ndclean".into();
    let recipe = Recipe {
        version: RECIPE_VERSION,
        name: String::new(),
        columns: Vec::new(),
        header: true,
        steps: vec![RecipeOp::CleanMatrix {
            spec: matrix(vec![e]),
        }],
        left_out: Vec::new(),
        questions: Vec::new(),
    };
    assert!(recipe.steps[0].holds_values());
    let text = recipe.steps[0].describe();
    assert!(text.contains("Name (from names.ndclean: "), "{text}");
    let loaded = Recipe::from_json(&recipe.to_json().unwrap()).unwrap();
    assert_eq!(loaded.steps, recipe.steps);
    let mut d = doc("id,name\r\n1, ann \r\n");
    assert!(loaded.column_problems(&d).is_empty());
    d.run_recipe_step(&loaded.steps[0], NOW, 1, &Progress::new())
        .unwrap();
    assert_eq!(rows(&d)[1], ["1", "ANN"]);
    // A missing column: nothing changes at all.
    let mut other = doc("id,town\r\n1, x \r\n");
    assert!(!loaded.column_problems(&other).is_empty());
    assert!(other
        .run_recipe_step(&loaded.steps[0], NOW, 1, &Progress::new())
        .is_err());
    assert!(!other.is_modified());
}
