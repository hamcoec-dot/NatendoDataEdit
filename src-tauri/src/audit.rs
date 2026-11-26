//! The hidden-rows audit: every operation that writes is run on a table with
//! rows hidden by a filter and by hand, and what happens to those rows is
//! compared cell for cell. The expected result of each is listed here, so a
//! change in behaviour fails the test, and a new data operation does not
//! compile until it is named (`name_of`).

use super::*;
use natendo_engine::clean::{CaseTo, Cleaner, NamePart, Step};
use natendo_engine::editing::{ExtractWhat, GroupFunc};
use natendo_engine::filter::ColumnFilter;
use natendo_engine::find::Scope;
use natendo_engine::ops::{FillSpec, PasteOp, PasteSpec};
use natendo_engine::shift::{RectFill, Shift, ShiftCells};
use natendo_engine::sort::{SortKey, SortSpec};
use std::collections::{BTreeMap, BTreeSet};

const ROWS: usize = 24;

/// The name of a data operation. No catch-all arm: a new one fails to compile
/// until it is classified below.
fn name_of(op: &DataOp) -> &'static str {
    match op {
        DataOp::Paste { .. } => "paste",
        DataOp::Fill { .. } => "fill",
        DataOp::ShowDuplicates { .. } => "show_duplicates",
        DataOp::RemoveDuplicates { .. } => "remove_duplicates",
        DataOp::RemoveBlankRows { .. } => "remove_blank_rows",
        DataOp::RemoveBlankCols { .. } => "remove_blank_cols",
        DataOp::SplitColumn { .. } => "split_column",
        DataOp::MergeColumns { .. } => "merge_columns",
        DataOp::SplitRows { .. } => "split_rows",
        DataOp::MergeRows { .. } => "merge_rows",
        DataOp::SortColumns { .. } => "sort_columns",
        DataOp::Join { .. } => "join",
        DataOp::Append { .. } => "append",
        DataOp::Clean { .. } => "clean",
        DataOp::SplitName { .. } => "split_name",
        DataOp::FillExample { .. } => "fill_example",
        DataOp::Compute { .. } => "compute",
        DataOp::ShowProblems { .. } => "show_problems",
        DataOp::ShowRagged { .. } => "show_ragged",
        DataOp::ShowShape { .. } => "show_shape",
        DataOp::ShowGapRows { .. } => "show_gap_rows",
        DataOp::SortProblems { .. } => "sort_problems",
        DataOp::CheckColumn { .. } => "check_column",
        DataOp::FillBlanks { .. } => "fill_blanks",
        DataOp::CountRepeats { .. } => "count_repeats",
        DataOp::BlankRepeats { .. } => "blank_repeats",
        DataOp::Breaks { .. } => "breaks",
        DataOp::DeleteHidden => "delete_hidden",
        DataOp::MapValues { .. } => "map_values",
        DataOp::GroupCalc { .. } => "group_calc",
        DataOp::Arrange { .. } => "arrange",
        DataOp::Extract { .. } => "extract",
        DataOp::FillRect { .. } => "fill_rect",
        DataOp::FormulaFilter { .. } => "formula_filter",
        DataOp::CompareShow { .. } => "compare_show",
        DataOp::CompareColumn { .. } => "compare_column",
        DataOp::FillSpecial { .. } => "fill_special",
        DataOp::ShiftCells { .. } => "shift_cells",
    }
}

/// id, name, grp, num, note, ini (one typed example) and void (all empty),
/// with two wholly blank rows, one shown and one hidden by hand. Rows repeat so that duplicate, blank and
/// grouping tools find work among shown and hidden rows alike.
fn body() -> Vec<u8> {
    let mut s = String::from("id,name,grp,num,note,ini,void\r\n");
    for i in 1..=ROWS {
        let grp = ["A", "B", "C"][i % 3];
        let name = format!("ann lee{}", i % 4);
        let num = if i % 5 == 0 {
            String::new()
        } else {
            (i % 7).to_string()
        };
        let note = if i % 2 == 0 { "x;y" } else { "" };
        let ini = if i == 1 { "ann" } else { "" };
        s.push_str(&format!("{i},{name},{grp},{num},{note},{ini},\r\n"));
        if i == 5 || i == 12 {
            s.push_str(",,,,,,\r\n");
        }
    }
    s.into_bytes()
}

fn wait(docs: &Docs, id: u32) {
    for _ in 0..500 {
        if docs.info(id).unwrap().indexed {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("indexing did not finish");
}

/// The whole table as it stands (every row shown first).
fn table(docs: &Docs, id: u32) -> Vec<Vec<String>> {
    let _ = docs.clear_filter(id);
    let _ = docs.show_rows(id, ShowRows::All);
    let info = docs.info(id).unwrap();
    (0..info.rows)
        .map(|r| {
            (0..info.cols)
                .map(|c| docs.cell_text(id, r, c).unwrap().0)
                .collect()
        })
        .collect()
}

struct Stage {
    docs: Docs,
    id: u32,
    before: Vec<Vec<String>>,
    /// The ids (first column) of the rows hidden by the filter or by hand.
    hidden: BTreeSet<String>,
}

fn stage(name: &str) -> Stage {
    let dir = std::env::temp_dir().join(format!("nde-audit-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("audit.csv");
    std::fs::write(&p, body()).unwrap();
    let docs = Docs::default();
    let id = docs.open_path(&p, false).unwrap().id;
    wait(&docs, id);
    let before = table(&docs, id);
    // The filter hides group C; then two shown rows are hidden by hand.
    docs.filter(
        id,
        FilterSpec {
            columns: vec![ColumnFilter {
                col: 2,
                rule: Rule::Values {
                    values: vec!["A".into(), "B".into()],
                    blanks: true,
                },
            }],
            keep_first: true,
            read: Default::default(),
        },
    )
    .unwrap();
    docs.hide_rows(id, 3, 1).unwrap();
    docs.hide_rows(id, 7, 2).unwrap();
    // One of the two blank rows is hidden by hand; the other stays shown.
    let blanks: Vec<u64> = (1..docs.info(id).unwrap().rows)
        .filter(|&r| (0..7).all(|c| docs.cell_text(id, r, c).unwrap().0.is_empty()))
        .collect();
    if blanks.len() == 2 {
        docs.hide_rows(id, blanks[1], 1).unwrap();
    }
    let shown: BTreeSet<String> = (1..docs.info(id).unwrap().rows)
        .map(|r| docs.cell_text(id, r, 0).unwrap().0)
        .collect();
    let hidden: BTreeSet<String> = before[1..]
        .iter()
        .map(|r| r[0].clone())
        .filter(|i| !i.is_empty() && !shown.contains(i))
        .collect();
    assert!(hidden.len() >= 10, "{hidden:?}");
    assert_eq!(blanks.len(), 2, "a shown and a hidden blank row");
    Stage {
        docs,
        id,
        before,
        hidden,
    }
}

/// What a run did to the hidden rows, in words ("untouched" when nothing).
fn effect(s: &Stage) -> String {
    let after = table(&s.docs, s.id);
    let titles = |t: &[Vec<String>]| t[0].clone();
    let (was, now) = (titles(&s.before), titles(&after));
    let id_col = now.iter().position(|t| t == "id").expect("the id column");
    let mut rows: BTreeMap<&str, &Vec<String>> = BTreeMap::new();
    for r in after.iter().skip(1) {
        rows.entry(r[id_col].as_str()).or_insert(r);
    }
    // Places counted among the rows that were there before and still are, so
    // rows removed or added elsewhere do not read as a move.
    let known: BTreeSet<&str> = s.before[1..].iter().map(|r| r[0].as_str()).collect();
    let mut seen = BTreeSet::new();
    let order_now: Vec<&str> = after[1..]
        .iter()
        .map(|r| r[id_col].as_str())
        .filter(|i| !i.is_empty() && known.contains(i) && seen.insert(*i))
        .collect();
    let order_was: Vec<&str> = s.before[1..]
        .iter()
        .map(|r| r[0].as_str())
        .filter(|i| seen.contains(i))
        .collect();
    let (mut gone, mut changed, mut moved, mut filled) = (false, false, false, false);
    for r in &s.before[1..] {
        if !s.hidden.contains(&r[0]) {
            continue;
        }
        let Some(new) = rows.get(r[0].as_str()) else {
            gone = true;
            continue;
        };
        moved |=
            order_now.iter().position(|i| *i == r[0]) != order_was.iter().position(|i| *i == r[0]);
        for (c, title) in was.iter().enumerate() {
            // A column the operation took away is a column operation, said
            // apart from a change to the row's values.
            if let Some(k) = now.iter().position(|t| t == title) {
                changed |= new[k] != r[c];
            }
        }
        filled |= now
            .iter()
            .enumerate()
            .any(|(k, t)| !was.contains(t) && !new[k].is_empty());
    }
    // Wholly blank rows have no id: counted by the columns that were there
    // before and still are.
    let blank = |t: &[Vec<String>]| {
        let cols: Vec<usize> = (0..t[0].len())
            .filter(|&k| was.contains(&t[0][k]) && now.contains(&t[0][k]))
            .collect();
        t[1..]
            .iter()
            .filter(|r| cols.iter().all(|&k| r[k].is_empty()))
            .count()
    };
    let mut out = Vec::new();
    for (yes, word) in [
        (
            blank(&after) + 1 < blank(&s.before),
            "removed the hidden blank row",
        ),
        (gone, "removed"),
        (changed, "changed"),
        (moved, "moved"),
        (filled, "filled in a new column"),
    ] {
        if yes {
            out.push(word);
        }
    }
    if after == s.before {
        // A case that did nothing at all proves nothing.
        "did nothing".into()
    } else if out.is_empty() {
        "untouched".into()
    } else {
        out.join(", ")
    }
}

fn clean_spec(scope: Scope) -> CleanSpec {
    CleanSpec {
        cleaner: Cleaner {
            version: 1,
            name: String::new(),
            steps: vec![Step::Case { to: CaseTo::Upper }],
        },
        scope,
        skip_first: true,
        read: Default::default(),
    }
}

/// Rows with an empty `num` fail.
fn checks() -> CheckSpec {
    use natendo_engine::validate::{Check, CheckKind};
    CheckSpec {
        checks: vec![Check {
            col: Some(3),
            kind: CheckKind::Required,
            name: None,
        }],
        keep_first: true,
        now: 20_000.0,
        ..Default::default()
    }
}

fn data_ops() -> Vec<DataOp> {
    use natendo_engine::ops::{Dir, MergeColumns, SplitColumn};
    let compute = |expr: &str, target| ComputeSpec {
        expr: expr.into(),
        title: "made".into(),
        target,
        at: None,
        keep_first: true,
        read: Default::default(),
        now: 20_000.0,
        tables: Default::default(),
    };
    vec![
        DataOp::Paste {
            spec: PasteSpec {
                row: 1,
                col: 1,
                op: PasteOp::None,
                ..Default::default()
            },
            text: "p\r\np\r\np\r\np\r\np\r\np\r\np\r\np\r\n".into(),
        },
        DataOp::Fill {
            spec: FillSpec {
                row: 1,
                col: 1,
                rows: 1,
                cols: 1,
                dir: Dir::Down,
                count: 8,
                ..Default::default()
            },
        },
        DataOp::RemoveDuplicates {
            spec: Duplicates {
                cols: vec![1, 2],
                keep_first: true,
                ..Default::default()
            },
        },
        DataOp::RemoveBlankRows {
            spec: Blanks {
                keep_first: true,
                ..Default::default()
            },
        },
        DataOp::SplitColumn {
            spec: SplitColumn {
                col: 4,
                separator: ";".into(),
                keep_original: true,
                keep_first: true,
                ..Default::default()
            },
        },
        DataOp::MergeColumns {
            spec: MergeColumns {
                col: 1,
                count: 2,
                separator: "+".into(),
                ..Default::default()
            },
        },
        DataOp::SplitRows {
            spec: SplitRows {
                col: 4,
                separator: ";".into(),
                keep_first: true,
                ..Default::default()
            },
        },
        DataOp::MergeRows {
            spec: MergeRows {
                keys: vec![1, 2],
                separator: "|".into(),
                keep_first: true,
                ..Default::default()
            },
        },
        DataOp::SortColumns {
            col: 1,
            count: 3,
            descending: true,
        },
        DataOp::Clean {
            spec: clean_spec(Scope::Columns { cols: vec![1] }),
            now: 20_000.0,
        },
        DataOp::SplitName {
            spec: SplitName {
                col: 1,
                parts: vec![NamePart::First, NamePart::Last],
                keep_original: true,
                keep_first: true,
            },
        },
        DataOp::FillExample {
            spec: ExampleSpec {
                col: 5,
                skip_first: true,
            },
        },
        DataOp::Compute {
            spec: compute("\"k\"", None),
        },
        DataOp::Compute {
            spec: compute("\"k\"", Some(1)),
        },
        DataOp::FillBlanks {
            spec: natendo_engine::tools::FillBlanks {
                cols: vec![3],
                from: natendo_engine::tools::FillFrom::Value { text: "f".into() },
                keep_first: true,
                ..Default::default()
            },
        },
        DataOp::CountRepeats {
            spec: natendo_engine::tools::Repeats {
                cols: vec![1],
                keep_first: true,
                ..Default::default()
            },
        },
        DataOp::BlankRepeats {
            spec: Duplicates {
                cols: vec![2],
                keep_first: true,
                ..Default::default()
            },
        },
        DataOp::Breaks {
            spec: natendo_engine::tools::Breaks {
                col: Some(2),
                rows: 1,
                keep_first: true,
                ..Default::default()
            },
        },
        DataOp::DeleteHidden,
        DataOp::MapValues {
            spec: MapValues {
                cols: vec![2],
                pairs: vec![("A".into(), "a".into()), ("C".into(), "c".into())],
                keep_first: true,
                ..Default::default()
            },
            from: None,
        },
        DataOp::GroupCalc {
            spec: GroupCalc {
                keys: vec![2],
                func: GroupFunc::Count,
                title: "made".into(),
                keep_first: true,
                ..Default::default()
            },
        },
        DataOp::Arrange {
            spec: Arrange {
                order: vec![0, 2, 1],
                keep_first: true,
                ..Default::default()
            },
        },
        DataOp::Extract {
            spec: Extract {
                col: 1,
                what: ExtractWhat::LastWord,
                title: "made".into(),
                keep_first: true,
                ..Default::default()
            },
        },
        DataOp::FillRect {
            spec: RectFill {
                row: 1,
                rows: 8,
                col: 1,
                cols: 1,
                value: "r".into(),
                ..Default::default()
            },
        },
        DataOp::RemoveBlankCols {
            spec: Blanks {
                keep_first: true,
                ..Default::default()
            },
        },
        DataOp::SortProblems { spec: checks() },
        DataOp::CheckColumn {
            spec: checks(),
            title: "made".into(),
        },
        DataOp::CompareColumn {
            spec: natendo_engine::colcompare::CompareCols {
                a: 1,
                b: 2,
                keep_first: true,
                title: "made".into(),
                ..Default::default()
            },
        },
        DataOp::FillSpecial {
            spec: natendo_engine::special::SpecialSpec {
                kind: natendo_engine::special::SpecialKind::Blanks,
                row: 1,
                rows: 100,
                cols: vec![3],
                read: Default::default(),
                repeats: Vec::new(),
            },
            value: "f".into(),
        },
        DataOp::ShiftCells {
            spec: ShiftCells {
                row: 2,
                rows: 2,
                col: 1,
                cols: 1,
                shift: Shift::Up,
            },
        },
    ]
}

#[test]
fn what_each_writing_operation_does_to_hidden_rows() {
    let mut got: Vec<(String, String)> = Vec::new();
    let mut run = |name: &str, f: &dyn Fn(&Stage) -> CmdResult<()>| {
        let s = stage(&format!("{name}-{}", got.len()));
        let said = match f(&s) {
            Ok(()) => effect(&s),
            Err(e) => format!("refused: {}", e.message),
        };
        got.push((name.to_string(), said));
    };
    for (k, op) in data_ops().into_iter().enumerate() {
        let name = format!("{}#{k}", name_of(&op));
        let op = std::cell::RefCell::new(Some(op));
        run(&name, &|s| {
            let op = op.borrow_mut().take().unwrap();
            s.docs.data_op(s.id, op).map(|_| ())
        });
    }
    run("sort", &|s| {
        let spec = SortSpec {
            scope: Default::default(),
            keys: vec![SortKey {
                col: 1,
                descending: true,
                ..serde_json::from_str(r#"{"col":1}"#).unwrap()
            }],
            keep_first: true,
            read: Default::default(),
            lists: Vec::new(),
        };
        s.docs.sort(s.id, spec).map(drop)
    });
    run("replace_all", &|s| {
        let spec = serde_json::from_value(serde_json::json!({
            "pattern": { "text": "ann", "kind": "plain", "match_case": false, "whole_cell": false },
            "scope": { "kind": "all" },
            "skip_first": true,
        }))
        .unwrap();
        s.docs.replace_all(s.id, spec, "bo".into()).map(drop)
    });
    run("set_cells", &|s| {
        s.docs
            .set_cells(s.id, (1..8).map(|r| (r, 1, "s".to_string())).collect())
            .map(drop)
    });
    run("fill_areas", &|s| {
        let area = natendo_engine::areas::Area {
            row: 1,
            rows: 8,
            col: 1,
            cols: 1,
        };
        s.docs
            .fill_areas(s.id, vec![area], "a".into(), &[])
            .map(drop)
    });
    run("move_cells", &|s| {
        s.docs.move_cells(s.id, 1, 3, 1, 1, 6, 1, false).map(drop)
    });
    run("insert_rows", &|s| s.docs.insert_rows(s.id, 3, 2).map(drop));
    run("delete_rows", &|s| s.docs.delete_rows(s.id, 2, 5).map(drop));
    run("move_rows", &|s| s.docs.move_rows(s.id, 2, 2, 8).map(drop));
    run("insert_cols", &|s| s.docs.insert_cols(s.id, 1, 1).map(drop));
    run("delete_cols", &|s| s.docs.delete_cols(s.id, 4, 1).map(drop));
    run("move_cols", &|s| s.docs.move_cols(s.id, 1, 1, 4).map(drop));
    // Everything leaves the hidden rows alone, except these.
    let touches: BTreeMap<&str, &str> = [
        // A new column is filled for hidden rows too (C3 changes compute).
        ("split_column#4", "filled in a new column"),
        ("merge_columns#5", "filled in a new column"),
        ("split_name#10", "filled in a new column"),
        ("compute#12", "filled in a new column"),
        ("extract#22", "filled in a new column"),
        // Says what it does: it deletes the hidden rows.
        ("delete_hidden#18", "removed"),
        // Sorts move hidden rows among all the rows.
        ("sort_problems#25", "moved"),
        ("sort", "moved"),
        // Refused while rows are hidden.
        ("shift_cells#29", "refused"),
        ("move_rows", "refused"),
    ]
    .into_iter()
    .collect();
    let wrong: Vec<String> = got
        .iter()
        .filter(|(name, said)| {
            let want = touches.get(name.as_str()).copied().unwrap_or("untouched");
            !(said == want || (want == "refused" && said.starts_with("refused:")))
        })
        .map(|(name, said)| format!("{name}: {said}"))
        .collect();
    assert!(
        wrong.is_empty(),
        "hidden rows treated otherwise than listed: {wrong:#?}"
    );
    assert!(got.len() >= touches.len());
}
