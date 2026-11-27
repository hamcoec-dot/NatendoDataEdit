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
        DataOp::ExplodeColumns { .. } => "explode_columns",
        DataOp::ExplodeRows { .. } => "explode_rows",
        DataOp::ShowExplodeBad { .. } => "show_explode_bad",
        DataOp::MergeRows { .. } => "merge_rows",
        DataOp::SortColumns { .. } => "sort_columns",
        DataOp::Join { .. } => "join",
        DataOp::Append { .. } => "append",
        DataOp::Clean { .. } => "clean",
        DataOp::CleanMatrix { .. } => "clean_matrix",
        DataOp::SplitName { .. } => "split_name",
        DataOp::FillExample { .. } => "fill_example",
        DataOp::Compute { .. } => "compute",
        DataOp::ShowProblems { .. } => "show_problems",
        DataOp::ShowRagged { .. } => "show_ragged",
        DataOp::ShowRelationExceptions { .. } => "show_relation_exceptions",
        DataOp::ShowShape { .. } => "show_shape",
        DataOp::ShowGapRows { .. } => "show_gap_rows",
        DataOp::ShowContractRows { .. } => "show_contract_rows",
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
    let dir = crate::testdir::path(&format!("nde-audit-{name}"));
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
            combine: Default::default(),
            invert: false,
            columns: vec![ColumnFilter {
                enabled: true,
                negate: false,
                name: None,
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
        // With as many rows as before, a hidden row must be exactly where it
        // was; otherwise it is placed among the rows that are still there.
        moved |= if after.len() == s.before.len() {
            after.iter().position(|x| x[id_col] == r[0])
                != s.before.iter().position(|x| x[0] == r[0])
        } else {
            order_now.iter().position(|i| *i == r[0]) != order_was.iter().position(|i| *i == r[0])
        };
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

/// Another tab beside the audited one.
fn second_tab(s: &Stage, name: &str, body: &str) -> u32 {
    let dir = crate::testdir::path(&format!("nde-audit-other-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("{name}.csv"));
    std::fs::write(&p, body).unwrap();
    let id = s.docs.open_path(&p, false).unwrap().id;
    wait(&s.docs, id);
    id
}

/// Find people matches of the audited tab's names in another tab: the pairs
/// as the screen would accept them.
fn find_pairs(s: &Stage, name: &str) -> Vec<(u32, u64, f32)> {
    let other = second_tab(
        s,
        name,
        "who,city\r\nann lee0,Oslo\r\nann lee1,Rome\r\nann lee2,Lyon\r\nann lee3,Kiel\r\n",
    );
    let spec = serde_json::from_value(serde_json::json!({
        "fields": [{ "col": 1, "other_col": 0 }],
        "keep_first": true, "other_header": true, "min_score": 95.0,
    }))
    .unwrap();
    let found = s.docs.match_find(s.id, Some(other), &spec, &[1]).unwrap();
    found
        .listed
        .iter()
        .map(|p| (p.id, p.other_row, p.score))
        .collect()
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
        hidden: true,
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
            typed: Vec::new(),
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
        DataOp::SortProblems {
            spec: checks(),
            scope: Default::default(),
        },
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
        // The note column holds "x;y" in every other row.
        DataOp::ExplodeColumns {
            spec: natendo_engine::explode::Explode {
                col: 4,
                kind: natendo_engine::explode::ExplodeKind::List,
                how: natendo_engine::explode::ExplodeHow::Flags,
                separator: ";".into(),
                ..Default::default()
            },
        },
        DataOp::ExplodeRows {
            spec: natendo_engine::explode::Explode {
                col: 4,
                kind: natendo_engine::explode::ExplodeKind::List,
                how: natendo_engine::explode::ExplodeHow::Rows,
                separator: ";".into(),
                original_col: true,
                ..Default::default()
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
    run("sort_shown", &|s| {
        let spec = SortSpec {
            keys: vec![serde_json::from_str(r#"{"col":1,"descending":true}"#).unwrap()],
            keep_first: true,
            scope: natendo_engine::sort::SortScope::Shown,
            ..Default::default()
        };
        s.docs.sort(s.id, spec).map(drop)
    });
    run("sort_problems_shown", &|s| {
        let op = DataOp::SortProblems {
            spec: checks(),
            scope: natendo_engine::sort::SortScope::Shown,
        };
        s.docs.data_op(s.id, op).map(drop)
    });
    run("compute_shown", &|s| {
        let spec = ComputeSpec {
            expr: "\"k\"".into(),
            title: "made".into(),
            keep_first: true,
            hidden: false,
            now: 20_000.0,
            ..Default::default()
        };
        s.docs.data_op(s.id, DataOp::Compute { spec }).map(drop)
    });
    run("clean_matrix", &|s| {
        let spec = natendo_engine::clean::CleanMatrix {
            entries: vec![natendo_engine::clean::MatrixEntry {
                col: 1,
                title: "name".into(),
                steps: vec![Step::Case { to: CaseTo::Upper }],
                from_file: String::new(),
            }],
            skip_first: true,
            read: Default::default(),
        };
        let op = DataOp::CleanMatrix {
            spec,
            now: 20_000.0,
            revision: None,
        };
        s.docs.data_op(s.id, op).map(drop)
    });
    run("join", &|s| {
        let other = second_tab(s, "join", "key,extra\r\nA,1\r\nB,2\r\nC,3\r\nD,4\r\n");
        let op = DataOp::Join {
            other: Some(other),
            list: None,
            list_id: None,
            spec: serde_json::from_value(serde_json::json!({
                "keys": [2], "other_keys": [0], "columns": [1],
                "keep_first": true, "other_header": true, "add_unmatched": true,
            }))
            .unwrap(),
        };
        s.docs.data_op(s.id, op).map(drop)
    });
    run("append", &|s| {
        let other = second_tab(s, "append", "id,name\r\n99,zed\r\n");
        let op = DataOp::Append {
            other,
            spec: serde_json::from_value(serde_json::json!({
                "by_name": true, "keep_first": true, "other_header": true,
            }))
            .unwrap(),
        };
        s.docs.data_op(s.id, op).map(drop)
    });
    run("batch_replace", &|s| {
        let spec = natendo_engine::find::BatchReplace {
            pairs: vec![("ann".into(), "bo".into())],
            whole_cell: false,
            match_case: false,
            scope: Scope::All,
            skip_first: true,
            skip_cols: Vec::new(),
        };
        s.docs.batch_preview(s.id, spec.clone())?;
        s.docs.batch_confirm(s.id, spec).map(drop)
    });
    run("match_accept", &|s| {
        let pairs = find_pairs(s, "match");
        assert!(!pairs.is_empty());
        s.docs.match_accept(s.id, &pairs).map(drop)
    });
    run("street_lookup", &|s| {
        let other = second_tab(
            s,
            "street",
            "street,low,high,ward\r\nann lee1,0,10,W1\r\nann lee2,0,10,W2\r\n",
        );
        let cols = natendo_engine::streets::IndexCols {
            street: 0,
            low: 1,
            high: 2,
            side: None,
            area: None,
            bring: vec![3],
            header: true,
        };
        let spec = natendo_engine::streets::StreetSpec {
            address: 1,
            number: Some(3),
            keep_first: true,
            ..Default::default()
        };
        s.docs.street_lookup(s.id, other, &cols, &spec).map(drop)
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
    run("set_aside", &|s| {
        s.docs
            .set_aside(s.id, 0, 1, natendo_engine::document::report::Place::Head)
            .map(drop)
    });
    run("put_back_notes", &|s| s.docs.put_back_notes(s.id).map(drop));
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
        ("explode_columns#30", "filled in a new column"),
        // Says what it does: it deletes the hidden rows.
        ("delete_hidden#18", "removed"),
        // Sorts move hidden rows among all the rows.
        ("sort_problems#25", "moved"),
        ("sort", "moved"),
        // Refused while rows are hidden.
        ("shift_cells#29", "refused"),
        ("move_rows", "refused"),
        // Report notes are the first and last rows of the file: refused
        // while some rows are hidden (C4).
        ("set_aside", "refused"),
        ("put_back_notes", "refused"),
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

/// A recipe that filters and then sorts the shown rows does the same on a
/// fresh copy: the rows its filter hides keep their places.
#[test]
fn a_recipe_with_a_filter_then_a_shown_sort_keeps_hidden_rows_in_place() {
    let s = stage("recipe-a");
    s.docs.show_rows(s.id, ShowRows::All).unwrap();
    let spec = SortSpec {
        keys: vec![serde_json::from_str(r#"{"col":0,"descending":true}"#).unwrap()],
        keep_first: true,
        scope: natendo_engine::sort::SortScope::Shown,
        ..Default::default()
    };
    s.docs.sort(s.id, spec).unwrap();
    let book = s.docs.recipe_of(s.id, true, "r").unwrap();
    assert!(
        book.steps
            .iter()
            .any(|st| st.describe().contains("shown rows")),
        "the step says which rows it sorts"
    );
    let want = table(&s.docs, s.id);
    let t = stage("recipe-b");
    let _ = t.docs.clear_filter(t.id);
    let _ = t.docs.show_rows(t.id, ShowRows::All);
    t.docs.run_recipe(t.id, &book, &[], 20_000.0, true).unwrap();
    let got = table(&t.docs, t.id);
    assert_eq!(got, want);
    // Group C, filtered out when the sort ran, is where it was.
    for (at, row) in t.before.iter().enumerate() {
        if row[2] == "C" {
            assert_eq!(got[at][0], row[0]);
        }
    }
}

/// Renaming and tidying titles writes the title row only, rows hidden or not.
#[test]
fn title_operations_change_no_data_row() {
    let s = stage("titles");
    s.docs.set_first_row_cell(s.id, 1, "who".into()).unwrap();
    s.docs
        .set_column_names(s.id, vec![(2, "group".into())])
        .unwrap();
    let spec = serde_json::from_value(serde_json::json!({ "case": "upper" })).unwrap();
    s.docs.tidy_titles(s.id, spec).unwrap();
    let after = table(&s.docs, s.id);
    assert_ne!(after[0], s.before[0]);
    assert_eq!(after[1..], s.before[1..]);
}

/// Matches found, then a row among them hidden, then accepted: the hidden row
/// is never written without a word (the accept is refused).
#[test]
fn a_row_hidden_between_find_and_accept_is_not_written_silently() {
    for how in ["hand", "filter"] {
        let s = stage(&format!("late-{how}"));
        let pairs = find_pairs(&s, &format!("late-{how}"));
        assert!(!pairs.is_empty());
        let before = table(&s.docs, s.id);
        // `table` shows every row: find again on that view, then hide.
        let pairs = find_pairs(&s, &format!("late-again-{how}"));
        if how == "hand" {
            s.docs.hide_rows(s.id, 1, 3).unwrap();
        } else {
            s.docs
                .filter(
                    s.id,
                    FilterSpec {
                        combine: Default::default(),
                        invert: false,
                        columns: vec![ColumnFilter {
                            enabled: true,
                            negate: false,
                            name: None,
                            col: 2,
                            rule: Rule::Values {
                                values: vec!["A".into()],
                                blanks: false,
                            },
                        }],
                        keep_first: true,
                        read: Default::default(),
                    },
                )
                .unwrap();
        }
        let refused = s.docs.match_accept(s.id, &pairs).unwrap_err();
        assert_eq!(refused.kind, "changed", "{}", refused.message);
        assert_eq!(table(&s.docs, s.id), before, "{how}");
    }
}
