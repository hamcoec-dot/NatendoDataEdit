//! File contracts (C9). Made-up data only.

use super::*;
use crate::coltype::ColumnTypes;
use crate::tablecheck::{Compare, TableCheckKind};
use crate::OpenOptions;
use proptest::prelude::*;

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

fn titles(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn col(title: &str) -> ContractColumn {
    ContractColumn {
        title: title.into(),
        aliases: Vec::new(),
        ty: None,
        allowed: None,
        blank_rate: None,
    }
}

fn contract(cols: Vec<ContractColumn>) -> Contract {
    Contract {
        contract: VERSION,
        name: "Monthly voters".into(),
        columns: cols,
        checks: None,
        rows: None,
        recognise: Recognise::default(),
        blank_points: DEFAULT_BLANK_POINTS,
    }
}

fn opt() -> DriftOptions {
    DriftOptions {
        keep_first: true,
        ..Default::default()
    }
}

fn drift(d: &Document, c: &Contract) -> DriftReport {
    let p = pair(c, &d.rule_titles(true));
    d.drift(c, &p, &opt(), &Progress::new()).unwrap()
}

const JAN: &str = "id,name,party,phone\r\n1,Ann Moss,DEM,555-0100\r\n2,Bo Lane,REP,555-0101\r\n3,Cy Hart,IND,\r\n4,Di Fox,DEM,555-0103\r\n";

fn jan_contract() -> Contract {
    let d = doc(JAN);
    let draft = d
        .contract_draft(
            &DraftSpec {
                name: "Monthly voters".into(),
                keep_first: true,
                allowed_cols: vec![2],
                ..Default::default()
            },
            &Progress::new(),
        )
        .unwrap();
    draft.contract
}

#[test]
fn a_draft_keeps_titles_blank_shares_and_the_values_asked_for() {
    let mut d = doc(JAN);
    d.set_column_types(ColumnTypes::from_list(vec![ColumnType::Id]));
    let draft = d
        .contract_draft(
            &DraftSpec {
                name: "Monthly voters".into(),
                keep_first: true,
                allowed_cols: vec![0, 2],
                ..Default::default()
            },
            &Progress::new(),
        )
        .unwrap();
    let c = &draft.contract;
    assert_eq!(draft.rows, 4);
    let t: Vec<&str> = c.columns.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(t, ["id", "name", "party", "phone"]);
    assert_eq!(c.columns[0].ty, Some(ColumnType::Id));
    assert_eq!(c.columns[1].ty, None);
    assert_eq!(c.columns[3].blank_rate, Some(0.25));
    assert_eq!(
        c.columns[2].allowed.as_ref().unwrap().values,
        ["DEM", "IND", "REP"]
    );
    // Every id differs and it is an ID: it looks like personal data. Party does not.
    assert!(draft.values[0].looks_personal);
    // Party: 3 different values in 4 filled cells also looks personal on so few rows.
    assert!(draft.values[1].looks_personal);
    assert!(c.columns[1].allowed.is_none());
}

#[test]
fn a_column_with_too_many_values_is_refused_for_allowed_values() {
    let mut t = String::from("k\r\n");
    for i in 0..=MAX_ALLOWED {
        t += &format!("v{i}\r\n");
    }
    let d = doc(&t);
    let draft = d
        .contract_draft(
            &DraftSpec {
                name: "Codes".into(),
                keep_first: true,
                allowed_cols: vec![0],
                ..Default::default()
            },
            &Progress::new(),
        )
        .unwrap();
    assert!(draft.values[0]
        .refused
        .as_ref()
        .unwrap()
        .contains("more than 500"));
    assert!(draft.contract.columns[0].allowed.is_none());
}

#[test]
fn the_same_file_has_no_drift() {
    let c = jan_contract();
    let rep = drift(&doc(JAN), &c);
    assert!(!rep.drifted, "{rep:?}");
    assert_eq!(rep.rows, 4);
    assert_eq!(rep.columns.len(), 4);
}

#[test]
fn a_renamed_title_is_found_by_place_and_by_another_title() {
    let mut c = jan_contract();
    c.columns[3].blank_rate = None;
    let d = doc("id,full_name,party,phone\r\n1,Ann,DEM,1\r\n");
    let p = pair(&c, &d.rule_titles(true));
    assert_eq!(p.map, [Some(0), Some(1), Some(2), Some(3)]);
    assert_eq!(p.renamed, [1]);
    let rep = d.drift(&c, &p, &opt(), &Progress::new()).unwrap();
    assert_eq!(rep.renamed.len(), 1);
    assert_eq!(
        (rep.renamed[0].from.as_str(), rep.renamed[0].to.as_str()),
        ("full_name", "name")
    );
    assert!(!rep.renamed[0].by_alias);
    assert!(rep.drifted);
    // Accepted as another title: paired, and not drift.
    c.columns[1].aliases.push("Full_Name".into());
    let p = pair(&c, &d.rule_titles(true));
    assert_eq!(p.by_alias, [1]);
    assert!(p.renamed.is_empty());
    let rep = d.drift(&c, &p, &opt(), &Progress::new()).unwrap();
    assert!(rep.renamed[0].by_alias);
    assert!(!rep.drifted, "{rep:?}");
}

#[test]
fn two_unknown_columns_in_one_gap_are_never_guessed() {
    let c = contract(vec![col("a"), col("b"), col("c")]);
    let p = pair(&c, &titles(&["a", "x", "y", "c"]));
    assert_eq!(p.map, [Some(0), None, Some(3)]);
    assert_eq!(p.missing, [1]);
    assert_eq!(p.added, [1, 2]);
}

#[test]
fn a_dropped_and_an_added_column() {
    let c = jan_contract();
    // phone dropped; email added before party: two gaps, none with exactly one of each.
    let d = doc("id,name,email,party\r\n1,Ann,a@example.com,DEM\r\n");
    let rep = drift(&d, &c);
    assert_eq!(rep.missing, ["phone"]);
    assert_eq!(rep.added, ["email"]);
    assert!(rep.renamed.is_empty());
    assert!(rep.drifted);
}

#[test]
fn columns_in_another_order() {
    let c = jan_contract();
    let d = doc("name,id,party,phone\r\n1,Ann,DEM,1\r\n");
    let p = pair(&c, &d.rule_titles(true));
    assert_eq!(p.map, [Some(1), Some(0), Some(2), Some(3)]);
    assert_eq!(p.moved.len(), 1);
    let rep = d.drift(&c, &p, &opt(), &Progress::new()).unwrap();
    assert_eq!(rep.moved.len(), 1);
    assert!(rep.drifted);
}

#[test]
fn a_new_value_in_a_list_column() {
    let c = jan_contract();
    let d = doc("id,name,party,phone\r\n1,Ann,DEM,1\r\n2,Bo,GRN,2\r\n3,Cy,dem,3\r\n4,Di,GRN,4\r\n5,Ed, LIB ,5\r\n");
    let rep = drift(&d, &c);
    let party = &rep.columns[2];
    // Case matters (the draft keeps the values as they are); spaces do not.
    assert_eq!(party.new_values, 4);
    assert_eq!(party.first_new, ["GRN", "dem", "LIB"]);
    assert!(rep.drifted);
}

#[test]
fn blank_share_over_and_under_the_threshold() {
    let c = jan_contract(); // phone 25% blank
                            // 2 of 5 blank: 40%, up 15 points.
    let d = doc(
        "id,name,party,phone\r\n1,A,DEM,1\r\n2,B,DEM,\r\n3,C,DEM,\r\n4,D,DEM,4\r\n5,E,DEM,5\r\n",
    );
    let rep = drift(&d, &c);
    assert!(rep.columns[3].blank_jump);
    // 1 of 4 blank: the same share.
    let d = doc("id,name,party,phone\r\n1,A,DEM,1\r\n2,B,DEM,\r\n3,C,DEM,3\r\n4,D,DEM,4\r\n");
    assert!(!drift(&d, &c).columns[3].blank_jump);
    // A wider threshold lets 15 points pass.
    let mut c = c;
    c.blank_points = 20.0;
    let d = doc(
        "id,name,party,phone\r\n1,A,DEM,1\r\n2,B,DEM,\r\n3,C,DEM,\r\n4,D,DEM,4\r\n5,E,DEM,5\r\n",
    );
    assert!(!drift(&d, &c).columns[3].blank_jump);
}

#[test]
fn row_count_out_of_range() {
    let mut c = jan_contract();
    c.rows = Some(RowRange { min: 5, max: 10 });
    let rep = drift(&doc(JAN), &c);
    assert!(rep.rows_out);
    assert!(rep.drifted);
    c.rows = Some(RowRange { min: 4, max: 4 });
    assert!(!drift(&doc(JAN), &c).rows_out);
}

#[test]
fn cells_that_break_the_column_type() {
    let mut c = contract(vec![col("n"), col("d")]);
    c.columns[0].ty = Some(ColumnType::Number);
    c.columns[1].ty = Some(ColumnType::Date(None));
    let d = doc("n,d\r\n1,2026-01-02\r\nabc,2026-01-03\r\n3,soon\r\n,\r\n");
    let rep = drift(&d, &c);
    assert_eq!(rep.columns[0].type_misses, 1);
    assert_eq!(rep.columns[1].type_misses, 1);
    assert_eq!(rep.columns[0].blanks, 1);
}

#[test]
fn a_failing_rule_and_one_whose_column_is_gone() {
    let src = doc(JAN);
    let mut c = jan_contract();
    c.checks = Some(ChecksFile::for_checks(
        &src,
        &[
            Check::new(Some(3), CheckKind::Required),
            Check::new(Some(1), CheckKind::Required),
        ],
        &[TableCheck {
            name: None,
            kind: TableCheckKind::RowCount {
                compare: Compare::AtLeast,
                value: 10,
            },
        }],
        true,
    ));
    // name renamed (found by place), phone blank twice.
    let d = doc("id,full_name,party,phone\r\n1,Ann,DEM,\r\n2,,REP,\r\n3,Cy,IND,7\r\n");
    let rep = drift(&d, &c);
    assert_eq!(rep.rules.len(), 2);
    assert_eq!((rep.rules[0].check, rep.rules[0].failing), (0, 2));
    // The rule follows the renamed column.
    assert_eq!((rep.rules[1].check, rep.rules[1].failing), (1, 1));
    assert_eq!(rep.table.len(), 1);
    assert_eq!(rep.table[0].ok, Some(false));
    // phone gone: its rule did not run, with the reason.
    let d = doc("id,name,party\r\n1,Ann,DEM\r\n");
    let rep = drift(&d, &c);
    assert_eq!(rep.not_run.len(), 1);
    assert_eq!(rep.not_run[0].check, 0);
    assert!(
        rep.not_run[0].why.contains("phone"),
        "{}",
        rep.not_run[0].why
    );
}

#[test]
fn show_these_rows_works_them_out_again() {
    let c = jan_contract();
    let d = doc("id,name,party,phone\r\n1,Ann,DEM,1\r\n2,Bo,GRN,\r\n3,Cy,DEM,\r\n");
    let p = pair(&c, &d.rule_titles(true));
    let plan = d
        .plan_drift_rows(
            &c,
            &p,
            DriftRows::Blanks { col: 3 },
            &opt(),
            &Progress::new(),
        )
        .unwrap();
    assert_eq!(plan.rows, 2);
    assert_eq!(plan.chip.label, "Blanks in phone");
    let plan = d
        .plan_drift_rows(
            &c,
            &p,
            DriftRows::NewValues { col: 2 },
            &opt(),
            &Progress::new(),
        )
        .unwrap();
    assert_eq!(plan.rows, 1);
    let mut d = d;
    d.apply_rows(plan).unwrap();
    assert_eq!(d.row_count(), 2);
    assert_eq!(d.cell(1, 2).unwrap(), "GRN");
}

#[test]
fn recognising_by_pattern_titles_and_both() {
    let mut a = jan_contract();
    a.recognise.pattern = Some("voters_*.csv".into());
    let mut b = contract(vec![col("precinct"), col("ward")]);
    b.name = "Precincts".into();
    let list = [a.clone(), b.clone()];
    let t = titles(&["id", "name", "party", "phone"]);
    // Both: the pattern wins over titles alone.
    let m = recognise(&list, Some("VOTERS_2026-02.CSV"), Some(&t));
    assert_eq!(m[0].index, 0);
    assert!(m[0].by_pattern);
    // Titles only (3 of 4 is under 80%: no match; 4 of 4 is).
    assert!(recognise(
        &list,
        Some("x.csv"),
        Some(&titles(&["id", "name", "party"]))
    )
    .is_empty());
    assert_eq!(recognise(&list, Some("x.csv"), Some(&t))[0].index, 0);
    // Pattern only (no titles: the tab has no title row).
    assert_eq!(recognise(&list, Some("voters_3.csv"), None).len(), 1);
    // An unrelated file.
    assert!(recognise(&list, Some("budget.csv"), Some(&titles(&["year", "total"]))).is_empty());
    // Titles turned off.
    a.recognise.titles = false;
    assert!(recognise([&a], Some("x.csv"), Some(&t)).is_empty());
    // The best of two by titles.
    let mut c = contract(vec![
        col("id"),
        col("name"),
        col("party"),
        col("phone"),
        col("ward"),
    ]);
    c.name = "Wide".into();
    let m = recognise([&c, &b, &jan_contract()], None, Some(&t));
    // Wide has 4 of its 5 titles (80%): both match, the full match first.
    assert_eq!(m.len(), 2);
    assert_eq!(m[0].index, 2);
}

#[test]
fn patterns() {
    assert!(pattern_matches("voters_*.csv", "voters_jan.csv"));
    assert!(pattern_matches("voters_*.csv", "voters_.csv"));
    assert!(pattern_matches("v?ters*", "Voters 2026.txt"));
    assert!(!pattern_matches("voters_*.csv", "voters_jan.tsv"));
    assert!(pattern_matches("*a*b*", "xxaxxbxx"));
    assert!(!pattern_matches("*a*b", "xxaxxbxxc"));
}

#[test]
fn fix_what_i_can_is_one_undo_step_of_titles_and_order() {
    let mut c = jan_contract();
    c.columns[0].ty = Some(ColumnType::Id);
    // name renamed (one unknown column in its place) and phone moved before party.
    let text = "id,full_name,phone,party,email\r\n1,Ann,555,DEM,a@example.com\r\n";
    let mut d = doc(text);
    let p = pair(&c, &d.rule_titles(true));
    let fix = d.contract_fix(&c, &p, true).unwrap();
    assert!(fix.moved);
    assert_eq!(fix.renamed, [("full_name".to_string(), "name".to_string())]);
    assert_eq!(fix.types, [(0, ColumnType::Id)]);
    assert_eq!(fix.order, [0, 1, 3, 2, 4]);
    let row = |d: &Document, r: u64| -> Vec<String> {
        (0..d.col_count()).map(|k| d.cell(r, k).unwrap()).collect()
    };
    assert_eq!(row(&d, 0), ["id", "name", "party", "phone", "email"]);
    assert_eq!(row(&d, 1), ["1", "Ann", "DEM", "555", "a@example.com"]);
    assert!(!drift(&d, &c).renamed.iter().any(|r| !r.by_alias));
    assert!(d.undo());
    assert_eq!(row(&d, 0), ["id", "full_name", "phone", "party", "email"]);
    assert_eq!(row(&d, 1), ["1", "Ann", "555", "DEM", "a@example.com"]);
    assert!(!d.undo(), "one step only");
    assert!(d.redo());
    assert_eq!(row(&d, 0), ["id", "name", "party", "phone", "email"]);
}

#[test]
fn fix_with_nothing_to_do_adds_no_step() {
    let c = jan_contract();
    let mut d = doc(JAN);
    let p = pair(&c, &d.rule_titles(true));
    let fix = d.contract_fix(&c, &p, true).unwrap();
    assert!(!fix.moved && fix.renamed.is_empty());
    assert!(!d.undo());
}

#[test]
fn a_stale_pairing_is_refused() {
    let c = jan_contract();
    let mut d = doc(JAN);
    let p = pair(&c, &d.rule_titles(true));
    d.delete_cols(3, 1).unwrap();
    assert!(d.drift(&c, &p, &opt(), &Progress::new()).is_err());
    assert!(d.contract_fix(&c, &p, true).is_err());
}

#[test]
fn round_trip() {
    let mut c = jan_contract();
    c.rows = Some(RowRange { min: 3, max: 6 });
    c.recognise.pattern = Some("voters_*.csv".into());
    c.columns[1].aliases.push("full_name".into());
    c.checks = Some(ChecksFile::for_rules(
        &doc(JAN),
        &[Check::new(Some(3), CheckKind::Required)],
        true,
    ));
    let bytes = c.to_bytes().unwrap();
    assert_eq!(Contract::from_bytes(&bytes).unwrap(), c);
}

#[test]
fn bad_contracts_are_refused_on_load() {
    let c = jan_contract();
    let json = |c: &Contract| serde_json::to_vec(c).unwrap();
    let err = |b: &[u8]| match Contract::from_bytes(b) {
        Err(Error::Invalid(w)) => w,
        other => panic!("{other:?}"),
    };
    assert!(err(b"{not json").contains("cannot be read"));
    assert!(err(&vec![b' '; MAX_FILE_BYTES + 1]).contains("too large"));
    let mut v: serde_json::Value = serde_json::from_slice(&json(&c)).unwrap();
    v["contract"] = 2.into();
    assert!(err(&serde_json::to_vec(&v).unwrap()).contains("newer version"));
    v["contract"] = 1.into();
    v["surprise"] = true.into();
    assert!(err(&serde_json::to_vec(&v).unwrap()).contains("cannot be read"));
    let mut big = c.clone();
    big.columns[2].allowed.as_mut().unwrap().values =
        (0..=MAX_ALLOWED).map(|i| format!("v{i}")).collect();
    assert!(err(&json(&big)).contains("allowed values"));
    let mut many = c.clone();
    many.columns[0].aliases = (0..=MAX_ALIASES).map(|i| format!("a{i}")).collect();
    assert!(err(&json(&many)).contains("other titles"));
    let mut pat = c.clone();
    pat.recognise.pattern = Some("[ab]*.csv".into());
    assert!(err(&json(&pat)).contains("only * and ?"));
    let mut rows = c.clone();
    rows.rows = Some(RowRange { min: 9, max: 1 });
    assert!(err(&json(&rows)).contains("wrong way round"));
    let mut name = c;
    name.name = "CON".into();
    assert!(err(&json(&name)).contains("Windows keeps"));
}

#[test]
fn contract_names_that_cannot_be_file_names() {
    for bad in [
        "a/b", "a\\b", "a:b", "a*", "a?", "a\"b", "<a>", "a|b", "a\u{1}b",
    ] {
        assert!(check_name(bad).is_err(), "{bad:?}");
    }
    for bad in ["voters.", "voters ", "..", "", "   "] {
        assert!(check_name(bad).is_err(), "{bad:?}");
    }
    for bad in ["CON", "con", "Nul.txt", "COM1", "lpt9.csv", "AUX "] {
        assert!(check_name(bad).is_err(), "{bad:?}");
    }
    assert!(check_name(&"x".repeat(MAX_NAME_CHARS + 1)).is_err());
    assert!(check_name(&"x".repeat(MAX_NAME_CHARS)).is_ok());
    for good in ["Monthly voters", "COM10", "console", "v1.2 précinct"] {
        assert!(check_name(good).is_ok(), "{good:?}");
    }
    assert!(name_taken(["Monthly Voters"], "monthly voters"));
    assert!(!name_taken(["Monthly Voters"], "Weekly voters"));
    assert_eq!(file_name("Monthly voters"), "Monthly voters.ndcontract");
}

#[test]
fn drift_counts_hidden_rows_too() {
    let c = jan_contract();
    let mut d = doc("id,name,party,phone\r\n1,Ann,DEM,1\r\n2,Bo,GRN,2\r\n");
    let p = pair(&c, &d.rule_titles(true));
    let plan = d
        .plan_drift_rows(
            &c,
            &p,
            DriftRows::NewValues { col: 2 },
            &opt(),
            &Progress::new(),
        )
        .unwrap();
    d.apply_rows(plan).unwrap(); // only the title row and the GRN row show
    assert_eq!(d.row_count(), 2);
    // Ann's row is hidden and still counted.
    assert_eq!(drift(&d, &c).columns[3].blanks, 0);
    assert_eq!(drift(&d, &c).columns[2].new_values, 1);
    assert_eq!(drift(&d, &c).rows, 2);
}

#[test]
fn drift_stops_when_cancelled() {
    let c = jan_contract();
    let d = doc(JAN);
    let p = pair(&c, &d.rule_titles(true));
    let pr = Progress::new();
    pr.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(matches!(
        d.drift(&c, &p, &opt(), &pr),
        Err(Error::Cancelled)
    ));
}

fn shuffled(n: usize, seed: u64) -> Vec<usize> {
    // A small deterministic shuffle.
    let mut v: Vec<usize> = (0..n).collect();
    let mut s = seed | 1;
    for i in (1..n).rev() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        v.swap(i, (s % (i as u64 + 1)) as usize);
    }
    v
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]
    /// Drops, moves and renames to another title the contract accepts are found exactly.
    #[test]
    fn pairing_finds_drops_moves_and_aliases(
        n in 1usize..12,
        drop in prop::collection::vec(any::<bool>(), 12),
        alias in prop::collection::vec(any::<bool>(), 12),
        seed in any::<u64>(),
    ) {
        let mut c = contract((0..n).map(|k| col(&format!("c{k}"))).collect());
        for k in 0..n {
            c.columns[k].aliases.push(format!("other{k}"));
        }
        let kept: Vec<usize> = (0..n).filter(|&k| !drop[k]).collect();
        let order = shuffled(kept.len(), seed);
        let file: Vec<usize> = order.iter().map(|&i| kept[i]).collect();
        let t: Vec<String> = file
            .iter()
            .map(|&k| if alias[k] { format!("Other{k}") } else { format!("C{k}") })
            .collect();
        let p = pair(&c, &t);
        for k in 0..n {
            let want = file.iter().position(|&f| f == k).map(|j| j as u32);
            prop_assert_eq!(p.map[k], want);
        }
        let missing: Vec<u32> = (0..n as u32).filter(|&k| drop[k as usize]).collect();
        prop_assert_eq!(&p.missing, &missing);
        prop_assert!(p.added.is_empty());
        prop_assert!(p.renamed.is_empty());
        let aliased: Vec<u32> = kept.iter().filter(|&&k| alias[k]).map(|&k| k as u32).collect();
        prop_assert_eq!(&p.by_alias, &aliased);
        let rising = file.windows(2).all(|w| w[0] < w[1]);
        prop_assert_eq!(p.moved.is_empty(), rising);
        // What stays put is in order, and the longest such run.
        let stay: Vec<usize> = kept.iter().copied().filter(|k| !p.moved.contains(&(*k as u32))).collect();
        let at: Vec<u32> = stay.iter().map(|&k| p.map[k].unwrap()).collect();
        prop_assert!(at.windows(2).all(|w| w[0] < w[1]));
    }

    /// Added columns are found exactly, wherever they are put.
    #[test]
    fn pairing_finds_added_columns(
        n in 1usize..10,
        add in prop::collection::vec(0usize..11, 0..4),
    ) {
        let c = contract((0..n).map(|k| col(&format!("c{k}"))).collect());
        let mut t: Vec<String> = (0..n).map(|k| format!("c{k}")).collect();
        for (i, &at) in add.iter().enumerate() {
            t.insert(at.min(t.len()), format!("new{i}"));
        }
        let p = pair(&c, &t);
        let added: Vec<u32> = (0..t.len() as u32).filter(|&j| t[j as usize].starts_with("new")).collect();
        prop_assert_eq!(&p.added, &added);
        prop_assert!(p.missing.is_empty() && p.moved.is_empty() && p.renamed.is_empty());
    }
}
