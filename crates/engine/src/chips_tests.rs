use super::*;
use crate::filter::Rule;
use crate::OpenOptions;
use proptest::prelude::*;

fn doc(text: &str) -> Document {
    Document::from_bytes(
        text.as_bytes().to_vec(),
        OpenOptions::default(),
        Some("csv"),
    )
    .unwrap()
}

fn values(col: u32, v: &[&str]) -> ColumnFilter {
    ColumnFilter::new(
        col,
        Rule::Values {
            values: v.iter().map(|s| s.to_string()).collect(),
            blanks: false,
        },
    )
}

fn spec(cols: Vec<ColumnFilter>) -> FilterSpec {
    FilterSpec {
        columns: cols,
        keep_first: true,
        ..Default::default()
    }
}

/// The first column of every shown data row.
fn shown(d: &Document) -> Vec<String> {
    (1..d.row_count()).map(|r| d.cell(r, 0).unwrap()).collect()
}

fn change(d: &mut Document, c: ChipChange) {
    d.change_filter(&c).unwrap();
}

const TABLE: &str = "id,party,town\n1,DEM,A\n2,REP,B\n3,DEM,B\n4,IND,A\n5,REP,A\n6,DEM,C\n";

#[test]
fn old_json_in_same_json_out() {
    let old = r#"{"columns":[{"col":1,"rule":{"type":"values","values":["DEM"],"blanks":false}}],"keep_first":true,"read":{"decimal_comma":false,"date_order":"mdy"}}"#;
    let s: FilterSpec = serde_json::from_str(old).unwrap();
    assert!(s.is_plain());
    assert_eq!(s.combine, Combine::And);
    assert!(s.columns[0].enabled && !s.columns[0].negate && s.columns[0].name.is_none());
    assert_eq!(serde_json::to_string(&s).unwrap(), old);
    // The new fields are written only when they differ.
    let mut t = s.clone();
    t.combine = Combine::Or;
    t.invert = true;
    t.columns[0].enabled = false;
    t.columns[0].negate = true;
    t.columns[0].name = Some("Dems".into());
    let j = serde_json::to_string(&t).unwrap();
    assert!(
        j.contains(r#""enabled":false"#)
            && j.contains(r#""combine":"or""#)
            && j.contains(r#""invert":true"#)
    );
    assert_eq!(serde_json::from_str::<FilterSpec>(&j).unwrap(), t);
}

#[test]
fn toggle_negate_match_any_and_invert_all() {
    let mut d = doc(TABLE);
    d.set_filter(&spec(vec![values(1, &["DEM"]), values(2, &["A"])]))
        .unwrap();
    assert_eq!(shown(&d), ["1"]);
    change(&mut d, ChipChange::Toggle { index: 1 });
    assert_eq!(shown(&d), ["1", "3", "6"]);
    change(&mut d, ChipChange::Toggle { index: 1 });
    change(&mut d, ChipChange::Negate { index: 0 });
    assert_eq!(shown(&d), ["4", "5"], "not DEM, and town A");
    change(&mut d, ChipChange::Negate { index: 0 });
    change(
        &mut d,
        ChipChange::Combine {
            combine: Combine::Or,
        },
    );
    assert_eq!(shown(&d), ["1", "3", "4", "5", "6"]);
    change(&mut d, ChipChange::Invert { on: true });
    assert_eq!(shown(&d), ["2"], "the rows that failed both");
    let c = d.filter_chips();
    assert_eq!(
        (c.shown, c.total, c.combine, c.invert),
        (1, 6, Combine::Or, true)
    );
    // All chips off: every row shows, and the bar says so.
    change(&mut d, ChipChange::Invert { on: false });
    change(&mut d, ChipChange::Toggle { index: 0 });
    change(&mut d, ChipChange::Toggle { index: 1 });
    assert_eq!(shown(&d).len(), 6);
    assert!(d.filter_chips().all_off);
    // Each change is one undo step, and undo puts back what showed.
    assert!(d.undo());
    assert_eq!(
        shown(&d),
        ["1", "4", "5"],
        "town A (the party chip still off)"
    );
    assert!(!d.filter_chips().all_off);
}

#[test]
fn switches_after_undo_match_a_fresh_filter() {
    let rules = || {
        vec![
            values(1, &["DEM"]),
            values(2, &["A"]),
            values(1, &["DEM", "REP"]),
        ]
    };
    let mut d = doc(TABLE);
    d.set_filter(&spec(rules())).unwrap();
    change(&mut d, ChipChange::Toggle { index: 0 });
    change(&mut d, ChipChange::Negate { index: 2 });
    assert!(d.undo());
    assert!(d.undo());
    change(&mut d, ChipChange::Toggle { index: 1 });
    change(
        &mut d,
        ChipChange::Combine {
            combine: Combine::Or,
        },
    );
    let got = shown(&d);
    let mut want = rules();
    want[1].enabled = false;
    let mut fresh = doc(TABLE);
    fresh
        .set_filter(&FilterSpec {
            combine: Combine::Or,
            ..spec(want)
        })
        .unwrap();
    assert_eq!(got, shown(&fresh));
}

#[test]
fn a_plain_filter_counts_each_chip() {
    let mut d = doc(TABLE);
    d.set_filter(&spec(vec![values(1, &["DEM"]), values(2, &["A"])]))
        .unwrap();
    // The bar never reads the table: a plain filter's rules were read together, so no count yet.
    let c = d.filter_chips();
    assert!(c.chips.iter().all(|x| x.rows.is_none()));
    assert_eq!(d.rule_cache_len(), 0);
    // Asked for (the bar's menu or tooltip): each rule's own count, kept for switching.
    let c = d.filter_chip_counts(&Progress::new()).unwrap();
    assert!(
        c.chips.iter().all(|x| x.rows.is_some()),
        "each rule's own count"
    );
    assert_eq!(d.rule_cache_len(), 2, "and kept for switching");
}

/// C8 review M1: after an edit the bar reads no row (the counts it had are gone until asked for).
#[test]
fn the_bar_reads_no_row_after_an_edit() {
    let mut d = doc(TABLE);
    d.set_filter(&spec(vec![values(1, &["DEM"]), values(2, &["A"])]))
        .unwrap();
    d.filter_chip_counts(&Progress::new()).unwrap();
    assert_eq!(d.rule_cache_len(), 2);
    d.set_cell(1, 2, "B".into()).unwrap();
    let c = d.filter_chips();
    assert!(
        c.chips.iter().all(|x| x.rows.is_none()),
        "no count without reading"
    );
    assert_eq!(d.rule_cache_len(), 0, "and no rule was read again");
    assert_eq!(c.shown + 1, d.row_count(), "the rows shown still known");
}

#[test]
fn no_row_matches_keeps_the_chips() {
    let mut d = doc(TABLE);
    d.set_filter(&spec(vec![values(1, &["DEM"]), values(1, &["REP"])]))
        .unwrap();
    assert!(shown(&d).is_empty());
    let c = d.filter_chips();
    assert_eq!((c.shown, c.chips.len()), (0, 2));
}

#[test]
fn rename_keeps_the_cache_an_edit_reads_again() {
    let mut d = doc(TABLE);
    d.set_filter(&spec(vec![values(1, &["DEM"]), values(2, &["A"])]))
        .unwrap();
    change(&mut d, ChipChange::Toggle { index: 1 });
    // The first build reads every rule at once; the switch reads the rule
    // still on by itself, and keeps it.
    let n = d.rule_cache_len();
    assert_eq!(n, 1);
    change(
        &mut d,
        ChipChange::Rename {
            index: 0,
            name: Some("Dems".into()),
        },
    );
    assert_eq!(d.rule_cache_len(), n);
    assert_eq!(d.filter_chips().chips[0].name.as_deref(), Some("Dems"));
    assert_eq!(d.filter_chips().chips[0].rows, Some(3));
    // An edit: row 2 (REP) becomes DEM. The next toggle reads the rule
    // again; the old result is not used.
    let before = d.rule_cache_len();
    d.clear_filter();
    d.set_cell(2, 1, "DEM".into()).unwrap();
    assert_eq!(d.rule_cache_len(), before, "kept until asked");
    d.set_filter(&spec(vec![values(1, &["DEM"]), values(2, &["A"])]))
        .unwrap();
    change(&mut d, ChipChange::Toggle { index: 1 });
    assert_eq!(shown(&d), ["1", "2", "3", "6"]);
}

#[test]
fn edit_then_toggle_reads_the_rule_again() {
    let mut d = doc(TABLE);
    d.set_filter(&spec(vec![values(1, &["DEM"]), values(2, &["A"])]))
        .unwrap();
    change(&mut d, ChipChange::Toggle { index: 1 });
    assert_eq!(shown(&d), ["1", "3", "6"]);
    // Shown row 1 is id 1 ("1, DEM, A"): it stops being DEM.
    d.set_cell(1, 1, "REP".into()).unwrap();
    change(&mut d, ChipChange::Toggle { index: 1 });
    change(&mut d, ChipChange::Toggle { index: 1 });
    assert_eq!(
        shown(&d),
        ["3", "6"],
        "the cached result from before the edit was not used"
    );
}

#[test]
fn the_cache_is_capped_by_bytes() {
    let mut c = RuleCache::default();
    c.at(1);
    let bits = |n: usize| Arc::new(vec![0u64; n]);
    c.put("a".into(), bits(100), 2000);
    c.put("b".into(), bits(100), 2000);
    assert_eq!(c.bytes(), 1600);
    c.get("a");
    c.put("c".into(), bits(100), 2000);
    assert!(c.get("b").is_none(), "the least recently used went");
    assert!(c.get("a").is_some() && c.get("c").is_some());
    assert!(c.bytes() <= 2000);
    c.put("huge".into(), bits(1000), 2000);
    assert!(c.get("huge").is_none(), "too big to keep");
    c.at(2);
    assert_eq!(
        (c.len(), c.bytes()),
        (0, 0),
        "a new data revision empties it"
    );
}

#[test]
fn tool_chips_never_show_rows_they_did_not_look_at() {
    let mut d = doc(TABLE);
    // A value chip: DEM rows (1, 3, 6) show; a tool then picks row 3 of them.
    d.set_filter(&spec(vec![values(1, &["DEM"])])).unwrap();
    let id3 = d.id_at(2).unwrap();
    let plan = d.plan_show_ids(&[id3], true);
    d.apply_rows(plan).unwrap();
    assert_eq!(shown(&d), ["3"]);
    let c = d.filter_chips();
    assert_eq!(
        (c.chips[1].rows, c.chips[1].looked, c.chips[1].not_looked),
        (Some(1), Some(3), Some(3))
    );
    // The value chip off, the tool chip inverted: only the two DEM rows it
    // looked at and did not pick; never the rows it never saw.
    change(&mut d, ChipChange::Toggle { index: 0 });
    change(&mut d, ChipChange::Negate { index: 1 });
    assert_eq!(shown(&d), ["1", "6"]);
    // Invert all on the tool chip alone: the row it picked.
    change(&mut d, ChipChange::Negate { index: 1 });
    change(&mut d, ChipChange::Invert { on: true });
    assert_eq!(shown(&d), ["1", "6"]);
    // A new value chip after it (OR) adds its rows; unlooked rows stay out
    // of the tool chip.
    change(&mut d, ChipChange::Invert { on: false });
    change(
        &mut d,
        ChipChange::Combine {
            combine: Combine::Or,
        },
    );
    change(&mut d, ChipChange::Toggle { index: 0 });
    assert_eq!(shown(&d), ["1", "3", "6"]);
    // After an edit the tool chip says it was made before it.
    assert!(!d.filter_chips().chips[1].stale);
    d.set_cell(1, 2, "Z".into()).unwrap();
    assert!(d.filter_chips().chips[1].stale);
}

#[test]
fn formula_errors_are_neither_pass_nor_fail() {
    use crate::expr::ComputeSpec;
    // "x" makes VALUE() an error.
    let mut d = doc("id,n\n1,5\n2,20\n3,x\n4,30\n");
    let f = ComputeSpec {
        expr: "VALUE([n]) > 10".into(),
        keep_first: true,
        ..Default::default()
    };
    let (plan, errors) = d.plan_formula_filter(&f, &Progress::new()).unwrap();
    assert_eq!(errors, 1);
    d.apply_rows(plan).unwrap();
    assert_eq!(shown(&d), ["2", "4"], "alone: as today");
    let c = d.filter_chips();
    assert_eq!(
        (c.chips[0].errors, c.chips[0].kind.as_str()),
        (1, "formula")
    );
    change(&mut d, ChipChange::Negate { index: 0 });
    assert_eq!(
        shown(&d),
        ["1"],
        "inverted: the FALSE row, not the error row"
    );
    change(&mut d, ChipChange::Negate { index: 0 });
    change(&mut d, ChipChange::Invert { on: true });
    assert_eq!(shown(&d), ["1"], "Invert all: the same");
    change(&mut d, ChipChange::Invert { on: false });
    // Beside a value chip with Match any: the error row shows only through
    // the other chip.
    d.set_filter(&spec(vec![values(0, &["3"])])).unwrap();
    change(
        &mut d,
        ChipChange::Combine {
            combine: Combine::Or,
        },
    );
    assert_eq!(shown(&d), ["2", "3", "4"]);
    change(&mut d, ChipChange::Toggle { index: 0 });
    assert_eq!(shown(&d), ["2", "4"]);
    // Show those rows: the error rows as a chip of their own.
    change(&mut d, ChipChange::ShowErrors { index: 1 });
    assert_eq!(shown(&d), ["3"]);
}

#[test]
fn twenty_tool_chips_at_most() {
    let mut d = doc(TABLE);
    for _ in 0..MAX_SET_CHIPS {
        let ids: Vec<u32> = (1..7).collect();
        let plan = d.plan_show_ids(&ids, true);
        d.apply_rows(plan).unwrap();
    }
    assert!(d.filter_chips().sets_full);
    let plan = d.plan_show_ids(&[1], true);
    let e = d.apply_rows(plan).err().unwrap().to_string();
    assert!(e.contains("at most 20"), "{e}");
}

#[test]
fn chips_survive_a_save() {
    let dir = std::env::temp_dir().join(format!("nde-chips-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.csv");
    std::fs::write(&path, TABLE).unwrap();
    let mut d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
    d.set_filter(&spec(vec![values(1, &["DEM"])])).unwrap();
    let id = d.id_at(2).unwrap();
    let plan = d.plan_show_ids(&[id], true);
    d.apply_rows(plan).unwrap();
    d.set_cell(1, 2, "Q".into()).unwrap();
    d.save(Default::default()).unwrap();
    assert_eq!(shown(&d), ["3"]);
    change(&mut d, ChipChange::Toggle { index: 1 });
    assert_eq!(shown(&d), ["1", "3", "6"]);
    change(&mut d, ChipChange::Toggle { index: 1 });
    change(&mut d, ChipChange::Toggle { index: 0 });
    change(&mut d, ChipChange::Negate { index: 1 });
    assert_eq!(shown(&d), ["1", "6"]);
    // Undo after the save goes back through the chip steps, chips and all.
    assert!(d.undo());
    assert!(d.undo());
    assert_eq!(shown(&d), ["3"], "both chips on again");
    assert_eq!(d.filter_chips().chips.len(), 2);
    assert!(d.undo());
    assert_eq!(shown(&d), ["1", "3", "6"], "the tool chip off again");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn undo_after_a_save_brings_back_the_chips() {
    let dir = std::env::temp_dir().join(format!("nde-chips-undo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.csv");
    std::fs::write(&path, TABLE).unwrap();
    let mut d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
    d.set_filter(&spec(vec![values(1, &["DEM"]), values(2, &["A"])]))
        .unwrap();
    let id = d.id_at(1).unwrap();
    let plan = d.plan_show_ids(&[id], true);
    d.apply_rows(plan).unwrap();
    let before = shown(&d);
    change(&mut d, ChipChange::Toggle { index: 2 });
    let off = shown(&d);
    d.save(Default::default()).unwrap();
    assert_eq!(shown(&d), off, "the save keeps what shows");
    assert!(d.undo());
    assert_eq!(
        shown(&d),
        before,
        "undo after the save switches the tool chip on again"
    );
    assert_eq!(d.filter_chips().chips.len(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Shows what the source showed on its own, ANDed with a value chip that
/// was on first (N3), for every "show these rows" tool.
fn beside(d: &mut Document, plan: crate::reshape::RowPlan) {
    let expected: Vec<u32> = d
        .ids()
        .filter(|&id| {
            let v = plan.view.as_ref().unwrap();
            v.passes(id) && d.view().is_none_or(|c| c.shows(id))
        })
        .collect();
    d.apply_rows(plan).unwrap();
    let got: Vec<u32> = d.ids().filter(|&id| d.view().unwrap().shows(id)).collect();
    assert_eq!(got, expected);
}

#[test]
fn every_tool_joins_beside_a_value_chip() {
    use crate::colcompare::CompareCols;
    use crate::expr::ComputeSpec;
    use crate::find::{FindSpec, Pattern, Scope};
    use crate::profile::ShapeFilter;
    use crate::reshape::Duplicates;
    let csv = "id,party,town,phone\n1,DEM,A,555-1234\n2,REP,B,x\n3,DEM,B,555-9999\n4,DEM,A,555-1234\n5,REP,A,7\n6,DEM,C,\n";
    let fresh = || {
        let mut d = doc(csv);
        d.set_filter(&spec(vec![values(1, &["DEM"])])).unwrap();
        d
    };
    let p = Progress::new();
    // Duplicates.
    let mut d = fresh();
    let plan = d
        .plan_show_duplicates(
            &Duplicates {
                cols: vec![2],
                keep_first: true,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    beside(&mut d, plan);
    // Problems (and the rows that passed).
    for passing in [false, true] {
        let mut d = fresh();
        let mut failing = vec![0u64; 1];
        failing[0] |= 1 << d.id_at(2).unwrap();
        let rep = crate::validate::CheckReport {
            rows: 1,
            failing,
            ..Default::default()
        };
        let plan = d.plan_show_problems(&rep, true, passing).unwrap();
        if passing {
            d.apply_rows(plan).unwrap();
            assert_eq!(shown(&d), ["1", "4", "6"], "the DEM rows that passed");
        } else {
            beside(&mut d, plan);
        }
    }
    // The pattern pill.
    let mut d = fresh();
    let plan = d
        .plan_shape_filter(
            &ShapeFilter {
                col: 3,
                keep_first: true,
                shape: "999-9999".into(),
                negate: false,
                include_blanks: false,
                revision: Some(d.data_revision()),
            },
            &p,
        )
        .unwrap();
    beside(&mut d, plan);
    // Two columns differ.
    let mut d = fresh();
    let plan = d
        .plan_compare_show(
            &CompareCols {
                a: 1,
                b: 2,
                keep_first: true,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    beside(&mut d, plan);
    // A formula.
    let mut d = fresh();
    let (plan, _) = d
        .plan_formula_filter(
            &ComputeSpec {
                expr: "[town] = \"A\"".into(),
                keep_first: true,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    beside(&mut d, plan);
    // Find matches.
    let mut d = fresh();
    let plan = d
        .plan_show_matching(
            &FindSpec {
                pattern: Pattern {
                    text: "555".into(),
                    ..Default::default()
                },
                scope: Scope::All,
                skip_first: true,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    beside(&mut d, plan);
    // Rows a Replace all changed (and the cells that did not parse).
    let mut d = fresh();
    let plan = d.plan_show_ids(&[1, 2, 5], true);
    beside(&mut d, plan);
    let mut d = fresh();
    let plan = d
        .plan_explode_bad(
            &crate::explode::Explode {
                col: 3,
                kind: crate::explode::ExplodeKind::Object,
                ..Default::default()
            },
            &p,
        )
        .unwrap();
    beside(&mut d, plan);
    // Ragged rows read every row (not a shown-rows tool).
    let mut d = doc("id,party\n1,DEM\n2,REP,extra\n3,DEM,more\n");
    d.set_filter(&spec(vec![values(1, &["DEM"])])).unwrap();
    let r = d.ragged_rows(true, &p).unwrap();
    let plan = d
        .plan_show_ragged(&r, true, crate::document::ragged::RaggedPick::All)
        .unwrap();
    d.apply_rows(plan).unwrap();
    assert_eq!(shown(&d), ["3"]);
    change(&mut d, ChipChange::Toggle { index: 0 });
    assert_eq!(shown(&d), ["2", "3"], "the ragged chip alone: as today");
}

// ---- the three-valued reference ----

#[derive(Debug, Clone)]
struct Model {
    /// Each rule: (column, value it picks, enabled, negate).
    rules: Vec<(u32, String, bool, bool)>,
    /// Each tool chip: (picked ids, looked-at ids, enabled, negate).
    sets: Vec<(Vec<u32>, Vec<u32>, bool, bool)>,
    combine: Combine,
    invert: bool,
}

/// Pass, fail or unknown for one chip and one row.
fn chip_says(model: &Model, d_vals: &[String], id: u32) -> Vec<Option<bool>> {
    let mut out = Vec::new();
    for (c, v, on, neg) in &model.rules {
        if *on {
            out.push(Some((&d_vals[*c as usize] == v) != *neg));
        }
    }
    for (set, looked, on, neg) in &model.sets {
        if *on {
            out.push(if looked.contains(&id) {
                Some(set.contains(&id) != *neg)
            } else {
                None
            });
        }
    }
    out
}

fn reference(model: &Model, rows: &[(u32, Vec<String>)]) -> Vec<u32> {
    let mut shown = Vec::new();
    for (id, vals) in rows {
        let says = chip_says(model, vals, *id);
        if says.is_empty() {
            shown.push(*id);
            continue;
        }
        let (pass, fail) = match model.combine {
            Combine::And => (
                says.iter().all(|s| *s == Some(true)),
                says.contains(&Some(false)),
            ),
            Combine::Or => (
                says.contains(&Some(true)),
                says.iter().all(|s| *s == Some(false)),
            ),
        };
        if if model.invert { fail } else { pass } {
            shown.push(*id);
        }
    }
    shown
}

proptest! {
    #[test]
    fn chips_equal_a_three_valued_reference(
        cells in prop::collection::vec(prop::collection::vec(0u8..3, 3), 1..30),
        rules in prop::collection::vec((0u32..3, 0u8..3, any::<bool>(), any::<bool>()), 1..5),
        sets in prop::collection::vec((prop::collection::vec(any::<bool>(), 30), any::<bool>(), any::<bool>()), 0..3),
        combine_or in any::<bool>(),
        invert in any::<bool>(),
        moves in prop::collection::vec((0u8..5, 0usize..8), 0..8),
    ) {
        let letter = |x: u8| ["a", "b", "c"][x as usize].to_string();
        let mut csv = String::from("x,y,z\n");
        for r in &cells {
            csv.push_str(&r.iter().map(|&x| letter(x)).collect::<Vec<_>>().join(","));
            csv.push('\n');
        }
        let mut d = doc(&csv);
        let rows: Vec<(u32, Vec<String>)> = (1..=cells.len() as u32)
            .map(|id| (id, cells[id as usize - 1].iter().map(|&x| letter(x)).collect()))
            .collect();
        let mut model = Model {
            rules: rules.iter().map(|&(c, v, on, neg)| (c, letter(v), on, neg)).collect(),
            sets: Vec::new(),
            combine: if combine_or { Combine::Or } else { Combine::And },
            invert,
        };
        // A plain filter of the first rule first, so tools look at fewer rows.
        let first = model.rules[0].clone();
        d.set_filter(&spec(vec![values(first.0, &[first.1.as_str()])])).unwrap();
        for (picks, on, neg) in &sets {
            let looked: Vec<u32> = rows.iter().map(|r| r.0).filter(|&id| d.view().unwrap().shows(id)).collect();
            let picked: Vec<u32> = looked.iter().copied().filter(|&id| picks[id as usize - 1]).collect();
            let before = d.tool_chip_count();
            let plan = d.plan_show_ids(&picked, true);
            d.apply_rows(plan).unwrap();
            if picked.is_empty() {
                // A tool that finds nothing adds no chip.
                prop_assert_eq!(d.tool_chip_count(), before);
                continue;
            }
            model.sets.push((picked, looked, *on, *neg));
        }
        // Now the rules, as the model has them (the tool chips stay).
        let s = FilterSpec {
            columns: model.rules.iter().map(|(c, v, on, neg)| ColumnFilter {
                enabled: *on,
                negate: *neg,
                ..values(*c, &[v.as_str()])
            }).collect(),
            keep_first: true,
            combine: model.combine,
            invert: model.invert,
            ..Default::default()
        };
        d.set_filter(&s).unwrap();
        for (k, (set, _, on, neg)) in model.sets.iter().enumerate() {
            let _ = set;
            let i = model.rules.len() + k;
            if !on {
                change(&mut d, ChipChange::Toggle { index: i });
            }
            if *neg {
                change(&mut d, ChipChange::Negate { index: i });
            }
        }
        let got = |d: &Document| -> Vec<u32> { rows.iter().map(|r| r.0).filter(|&id| d.view().is_none_or(|v| v.shows(id))).collect() };
        prop_assert_eq!(got(&d), reference(&model, &rows));
        // Random changes: each is the same as a fresh build of that state.
        for (op, at) in moves {
            let n = model.rules.len() + model.sets.len();
            if n == 0 {
                break;
            }
            let i = at % n;
            let c = match op {
                0 => ChipChange::Toggle { index: i },
                1 => ChipChange::Negate { index: i },
                2 => ChipChange::Combine { combine: if model.combine == Combine::And { Combine::Or } else { Combine::And } },
                3 => ChipChange::Invert { on: !model.invert },
                _ => ChipChange::Remove { index: i },
            };
            let rl = model.rules.len();
            match &c {
                ChipChange::Toggle { .. } if i < rl => model.rules[i].2 ^= true,
                ChipChange::Toggle { .. } => model.sets[i - rl].2 ^= true,
                ChipChange::Negate { .. } if i < rl => model.rules[i].3 ^= true,
                ChipChange::Negate { .. } => model.sets[i - rl].3 ^= true,
                ChipChange::Combine { combine } => model.combine = *combine,
                ChipChange::Invert { on } => model.invert = *on,
                _ if i < rl => { model.rules.remove(i); }
                _ => { model.sets.remove(i - rl); }
            }
            change(&mut d, c);
            if model.rules.is_empty() && model.sets.is_empty() {
                prop_assert!(d.view().is_none_or(|v| !v.filters()));
                break;
            }
            prop_assert_eq!(got(&d), reference(&model, &rows));
        }
    }
}

/// C8 review M2: a row put in after a tool chip shows whether the chip, or the whole filter, is inverted,
/// after a save that keeps the history just as without one; undo across the save too.
#[test]
fn rows_added_after_a_tool_chip_show_after_a_save() {
    for (k, invert_all) in [false, true].into_iter().enumerate() {
        for save in [false, true] {
            let dir = std::env::temp_dir()
                .join(format!("nde-chips-m2-{}-{k}-{save}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("t.csv");
            std::fs::write(&path, "id\n1\n2\n3\n4\n").unwrap();
            let mut d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
            let id = d.id_at(1).unwrap();
            let plan = d.plan_show_ids(&[id], true);
            d.apply_rows(plan).unwrap();
            assert_eq!(shown(&d), ["1"]);
            d.insert_rows(1, vec![vec!["9".into()]]).unwrap();
            assert_eq!(shown(&d), ["9", "1"], "a new row shows");
            if save {
                d.save(Default::default()).unwrap();
                assert_eq!(shown(&d), ["9", "1"], "the save keeps what shows");
            }
            let c = if invert_all {
                ChipChange::Invert { on: true }
            } else {
                ChipChange::Negate { index: 0 }
            };
            change(&mut d, c);
            assert_eq!(
                shown(&d),
                ["9", "2", "3", "4"],
                "invert_all {invert_all}, save {save}"
            );
            assert!(d.undo());
            assert_eq!(shown(&d), ["9", "1"], "undo the inversion (save {save})");
            assert!(d.undo());
            assert_eq!(
                shown(&d),
                ["1"],
                "undo the new row, across the save (save {save})"
            );
            assert!(d.redo());
            assert!(d.redo());
            assert_eq!(shown(&d), ["9", "2", "3", "4"], "redo (save {save})");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// C8 review note 1: a tool that finds nothing adds no chip and no undo step.
#[test]
fn a_tool_that_finds_nothing_adds_no_chip() {
    let mut d = doc(TABLE);
    d.set_filter(&spec(vec![values(1, &["DEM"])])).unwrap();
    let steps = d.history_pos();
    let plan = d.plan_show_ids(&[], true);
    d.apply_rows(plan).unwrap();
    assert_eq!(d.tool_chip_count(), 0);
    assert_eq!(d.history_pos(), steps, "no undo step");
    assert_eq!(shown(&d), ["1", "3", "6"], "the filter as it was");
}
