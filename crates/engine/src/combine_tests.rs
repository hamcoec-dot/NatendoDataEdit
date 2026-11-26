use super::*;
use crate::reshape::Table;
use crate::tools::{combine, combine_with_plan, Combine};
use proptest::prelude::*;

fn doc(text: &str) -> Document {
    Document::from_bytes(
        text.as_bytes().to_vec(),
        OpenOptions::default(),
        Some("csv"),
    )
    .unwrap()
}

/// The combine of main before the column map (ecded9e7), kept as the oracle.
fn old_combine(tables: &[(String, &Document)], spec: &Combine) -> Table {
    let by_title = spec.by_title && spec.header;
    let mut union = TitleUnion::default();
    let mut titles: Vec<String> = Vec::new();
    let mut maps: Vec<Vec<usize>> = Vec::new();
    for (_, d) in tables {
        let w = d.col_count() as usize;
        let mut row0 = if spec.header && d.row_count() > 0 {
            d.row_values(0).unwrap()
        } else {
            Vec::new()
        };
        row0.resize(w, String::new());
        if by_title {
            maps.push(union.add(&row0));
        } else {
            for (c, t) in row0.iter().enumerate().skip(titles.len()) {
                titles.push(if t.is_empty() {
                    format!("Column {}", c + 1)
                } else {
                    t.clone()
                });
            }
            maps.push((0..w).collect());
        }
    }
    if by_title {
        titles = union.into_titles();
    }
    let width = titles.len() + usize::from(spec.source);
    let mut out = Vec::new();
    let mut line: Vec<String> = Vec::new();
    let mut rows = 0u64;
    if spec.header {
        if spec.source {
            line.push(crate::tools::source_title(&titles));
        }
        line.extend(titles.iter().cloned());
        crate::reshape::push_line(&mut out, &line);
        rows += 1;
    }
    for ((name, d), map) in tables.iter().zip(&maps) {
        for r in d.data_rows(spec.header) {
            let v = d.row_values(r).unwrap();
            line.clear();
            line.resize(width, String::new());
            let off = usize::from(spec.source);
            if spec.source {
                line[0] = name.clone();
            }
            for (k, &to) in map.iter().enumerate() {
                line[off + to] = v.get(k).cloned().unwrap_or_default();
            }
            crate::reshape::push_line(&mut out, &line);
            rows += 1;
        }
    }
    Table {
        bytes: out,
        rows,
        cols: width as u32,
    }
}

fn csv(rows: &[Vec<String>]) -> String {
    let mut s = String::new();
    for r in rows {
        let mut line = String::new();
        for (i, c) in r.iter().enumerate() {
            if i > 0 {
                line.push(',');
            }
            crate::scan::write_field(c, crate::Dialect::CSV, &mut line);
        }
        s += &line;
        s += "\r\n";
    }
    s
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 300, ..ProptestConfig::default() })]
    /// The automatic plan gives today's bytes, for any title rows (repeats, blanks, case and spaces).
    #[test]
    fn the_automatic_plan_gives_what_combine_gave(
        files in prop::collection::vec(
            prop::collection::vec(prop::sample::select(vec!["id", "ID", " id", "Note", "note", "", "x", "Cust_ID"]), 1..6),
            1..5,
        ),
        header in any::<bool>(),
        by_title in any::<bool>(),
        source in any::<bool>(),
    ) {
        let docs: Vec<Document> = files
            .iter()
            .enumerate()
            .map(|(f, titles)| {
                let head: Vec<String> = titles.iter().map(|t| t.to_string()).collect();
                let data: Vec<String> = (0..titles.len()).map(|c| format!("f{f}c{c}")).collect();
                doc(&csv(&[head, data.clone(), data]))
            })
            .collect();
        let tables: Vec<(String, &Document)> = docs.iter().enumerate().map(|(i, d)| (format!("f{i}.csv"), d)).collect();
        let spec = Combine { header, by_title, source };
        let new = combine(&tables, &spec, &Progress::new()).unwrap();
        let old = old_combine(&tables, &spec);
        prop_assert_eq!(new.bytes, old.bytes);
        prop_assert_eq!(new.cols, old.cols);
    }
}

fn plan_of(rows: &[&[&str]]) -> (CombinePlan, Vec<FileHead>) {
    let title_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| r.iter().map(|t| t.to_string()).collect())
        .collect();
    let heads = title_rows
        .iter()
        .enumerate()
        .map(|(f, t)| FileHead {
            name: format!("f{f}.csv"),
            titles: t.clone(),
            cols: t.len() as u32,
            rows_est: 1,
            whole: true,
            preview: vec![t.iter().map(|x| format!("{x} value")).collect()],
        })
        .collect();
    (auto_plan(&title_rows, true), heads)
}

fn names(plan: &CombinePlan, s: &[Suggestion]) -> Vec<(String, String)> {
    s.iter()
        .map(|s| {
            (
                plan.titles[s.keep as usize].clone(),
                plan.titles[s.merge as usize].clone(),
            )
        })
        .collect()
}

#[test]
fn tidied_and_similar_titles_are_suggested_not_merged() {
    let (plan, heads) = plan_of(&[
        &["Cust_ID", "Name"],
        &["CustID", "Name"],
        &["cust id", "Name"],
    ]);
    // Today's Combine keeps them apart, and so does the automatic plan.
    assert_eq!(plan.titles, ["Cust_ID", "Name", "CustID", "cust id"]);
    let s = suggest(&plan, &heads);
    assert!(s.iter().all(|s| s.why == Why::Tidied), "{s:?}");
    assert_eq!(
        names(&plan, &s),
        [
            ("Cust_ID".into(), "CustID".into()),
            ("Cust_ID".into(), "cust id".into()),
            ("CustID".into(), "cust id".into())
        ]
    );
    assert_eq!(s[0].keep_samples, ["Cust_ID value"]);
    assert_eq!(s[0].merge_samples, ["CustID value"]);
    // Similar, in about the same place.
    let (plan, heads) = plan_of(&[&["CustomerID", "Name"], &["Cust_ID", "Name"]]);
    let _ = jw_check("customerid", "custid");
    let s = suggest(&plan, &heads);
    assert_eq!(
        names(&plan, &s),
        [("CustomerID".into(), "Cust_ID".into())],
        "{s:?}"
    );
    assert!(matches!(s[0].why, Why::Similar { .. }));
}

fn jw_check(a: &str, b: &str) -> f32 {
    crate::fuzzy::jaro_winkler(a, b)
}

#[test]
fn titles_whose_numbers_differ_are_never_suggested() {
    for (a, b) in [
        ("Address Line 1", "Address Line 2"),
        ("Precinct 1", "Precinct 2"),
        ("Phone 2", "Phone 3"),
        ("2023 Votes", "2024 Votes"),
        // F1: other words as well as other numbers.
        ("Votes 2023", "Vote 2024"),
        ("Precinct 1", "Precincts 2"),
        ("Phone 2", "Phones 3"),
        ("Ward 01", "Wards 02"),
    ] {
        let (plan, heads) = plan_of(&[&[a, "x"], &[b, "x"]]);
        assert!(suggest(&plan, &heads).is_empty(), "{a} / {b}");
    }
    // The same numbers, or numbers on one side only, are still suggested when the words are alike.
    for (a, b) in [("CustomerID", "Cust_ID"), ("Address 1", "Addr 1")] {
        let (plan, heads) = plan_of(&[&[a, "x"], &[b, "x"]]);
        let s = suggest(&plan, &heads);
        assert_eq!(
            names(&plan, &s),
            [(a.to_string(), b.to_string())],
            "{a} / {b}: {s:?}"
        );
    }
}

#[test]
fn a_pair_one_file_has_both_of_is_never_suggested() {
    let (plan, heads) = plan_of(&[&["Cust_ID", "CustID"], &["Cust_ID"]]);
    assert!(suggest(&plan, &heads).is_empty());
    let mut p = plan.clone();
    assert!(p.merge(0, 1).is_err());
}

#[test]
fn merging_and_checking_a_plan() {
    let (mut plan, _) = plan_of(&[&["Cust_ID", "Name"], &["Name", "CustID", "Extra"]]);
    assert_eq!(plan.titles, ["Cust_ID", "Name", "CustID", "Extra"]);
    plan.merge(0, 2).unwrap();
    assert_eq!(plan.titles, ["Cust_ID", "Name", "Extra"]);
    assert_eq!(plan.files[1], [Some(1), Some(0), Some(2)]);
    plan.check(&[2, 3]).unwrap();
    // Two columns of one file on one output: refused.
    let mut bad = plan.clone();
    bad.files[1] = vec![Some(0), Some(0), None];
    assert!(bad
        .check(&[2, 3])
        .unwrap_err()
        .to_string()
        .contains("overwrite"));
    // Out of range, wrong widths, wrong file count.
    let mut bad = plan.clone();
    bad.files[0][0] = Some(9);
    assert!(bad.check(&[2, 3]).is_err());
    assert!(plan.check(&[2, 4]).is_err());
    assert!(plan.check(&[2]).is_err());
    // Leaving a column out, then compacting.
    let mut p = plan.clone();
    p.files[1][2] = None;
    p.compact();
    assert_eq!(p.titles, ["Cust_ID", "Name"]);
}

#[test]
fn a_plan_with_left_out_and_moved_columns_combines() {
    let a = doc("id,name\r\n1,Ann\r\n");
    let b = doc("name,ID,notes\r\nBo,2,x\r\n");
    let tables = vec![("a.csv".to_string(), &a), ("b.csv".to_string(), &b)];
    let plan = CombinePlan {
        titles: vec!["Key".into(), "Name".into()],
        files: vec![vec![Some(0), Some(1)], vec![Some(1), Some(0), None]],
    };
    let spec = Combine {
        header: true,
        by_title: true,
        source: true,
    };
    let t = combine_with_plan(&tables, &plan, &spec, &Progress::new()).unwrap();
    assert_eq!(
        String::from_utf8(t.bytes).unwrap(),
        "Source file,Key,Name\r\na.csv,1,Ann\r\nb.csv,2,Bo\r\n"
    );
}

#[test]
fn a_saved_mapping_round_trips_and_fits_next_months_files() {
    let rows: Vec<Vec<String>> = vec![
        vec![
            "Cust_ID".into(),
            "Name".into(),
            "Note".into(),
            "Note".into(),
        ],
        vec!["Name".into(), "CustID".into()],
    ];
    let mut plan = auto_plan(&rows, true);
    let ci = plan.titles.iter().position(|t| t == "CustID").unwrap() as u32;
    plan.merge(0, ci).unwrap();
    let m = SavedMapping::of_plan(&plan, &rows, Extras::Add, false);
    assert_eq!(m.columns[0].from, ["CustID"]);
    let back = SavedMapping::from_json(&m.to_json()).unwrap();
    assert_eq!(back, m);
    assert!(back.tool.starts_with("Natendo Data Edit "));
    // Next month: columns moved, a repeated title, a new column, one missing.
    let next: Vec<Vec<String>> = vec![vec![
        "note".into(),
        "CustID".into(),
        "Note".into(),
        "Phone".into(),
    ]];
    let fit = back.apply(&next);
    assert!(fit.problems.is_empty(), "{:?}", fit.problems);
    assert_eq!(
        fit.plan.titles,
        ["Cust_ID", "Name", "Note", "Note (2)", "Phone"]
    );
    assert_eq!(fit.plan.files[0], [Some(2), Some(0), Some(3), Some(4)]);
    assert_eq!(fit.missing, [(0, 1)]);
    assert_eq!(fit.extras, [(0, 3)]);
    // Leave extras out.
    let mut lo = back.clone();
    lo.extras = Extras::LeaveOut;
    assert_eq!(lo.apply(&next).plan.files[0][3], None);
}

#[test]
fn a_saved_mapping_never_guesses() {
    let m = SavedMapping {
        kind: MAPPING_KIND.into(),
        version: 1,
        tool: "t".into(),
        header: true,
        file_name_column: false,
        extras: Extras::Add,
        columns: vec![
            SavedColumn {
                title: "ID".into(),
                from: vec!["Cust_ID".into(), "Key".into()],
            },
            SavedColumn {
                title: "Code".into(),
                from: vec!["key".into()],
            },
        ],
    };
    // Two columns of one file match one output.
    let fit = m.apply(&[vec!["Cust_ID".into(), "ID".into()]]);
    assert_eq!(fit.problems.len(), 1, "{:?}", fit.problems);
    assert!(fit.problems[0].what.contains("both go to \"ID\""));
    assert_eq!(fit.plan.files[0], [None, None], "neither is chosen");
    // One column matches two outputs.
    let fit = m.apply(&[vec!["Key".into()]]);
    assert!(fit.problems[0].what.contains("matches 2 columns"));
}

#[test]
fn bad_mapping_files_are_refused() {
    let good = SavedMapping {
        kind: MAPPING_KIND.into(),
        version: 1,
        tool: "t".into(),
        header: true,
        file_name_column: false,
        extras: Extras::Add,
        columns: vec![SavedColumn {
            title: "A".into(),
            from: vec![],
        }],
    };
    assert!(SavedMapping::from_json(&good.to_json()).is_ok());
    assert!(SavedMapping::from_json(b"not json").is_err());
    assert!(SavedMapping::from_json(&vec![b' '; MAX_MAPPING_BYTES + 1]).is_err());
    let mut v: Value = serde_json::from_slice(&good.to_json()).unwrap();
    v["kind"] = "natendo-receipt".into();
    assert!(SavedMapping::from_json(&serde_json::to_vec(&v).unwrap()).is_err());
    let mut newer = good.clone();
    newer.version = 2;
    assert!(SavedMapping::from_json(&newer.to_json()).is_err());
    let mut wide = good.clone();
    wide.columns = (0..=MAX_OUTPUT_COLS)
        .map(|i| SavedColumn {
            title: format!("c{i}"),
            from: vec![],
        })
        .collect();
    assert!(SavedMapping::from_json(&wide.to_json()).is_err());
    let mut no_head = good.clone();
    no_head.header = false;
    assert!(SavedMapping::from_json(&no_head.to_json()).is_err());
}

#[test]
fn a_saved_mapping_holds_no_path_file_name_or_value() {
    let dir = std::env::temp_dir().join(format!("Precinct-Voters-Zyx-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("voters-jan-Qwv.csv");
    let b = dir.join("voters-feb-Qwv.csv");
    std::fs::write(&a, "Cust_ID,Name\r\n7731,Annabel Quist\r\n").unwrap();
    std::fs::write(&b, "CustID,Name\r\n8842,Bo Varn\r\n").unwrap();
    let heads: Vec<FileHead> = [&a, &b].iter().map(|p| head_of(p, true).unwrap()).collect();
    let rows: Vec<Vec<String>> = heads.iter().map(|h| h.titles.clone()).collect();
    let mut plan = auto_plan(&rows, true);
    plan.merge(0, 2).unwrap();
    let text = String::from_utf8(SavedMapping::of_plan(&plan, &rows, Extras::Add, true).to_json())
        .unwrap();
    for bad in [
        "Zyx", "Qwv", "voters", "7731", "Annabel", "8842", "Varn", ":\\", "/",
    ] {
        assert!(!text.contains(bad), "{bad} in {text}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_head_reads_the_start_only_and_still_fits_checks_the_whole_file() {
    let dir = std::env::temp_dir().join(format!("nd-combine-head-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("big.csv");
    let mut s = String::from("id,name\r\n");
    for i in 0..100_000 {
        if i == 90_000 {
            s += &format!("{i},wide,extra\r\n");
        } else {
            s += &format!("{i},n{i}\r\n");
        }
    }
    std::fs::write(&p, &s).unwrap();
    let h = head_of(&p, true).unwrap();
    assert!(!h.whole);
    assert_eq!(
        (h.cols, h.titles.clone()),
        (2, vec!["id".to_string(), "name".into()])
    );
    assert_eq!(h.preview.len(), PREVIEW_ROWS);
    assert!((90_000..110_000).contains(&h.rows_est), "{}", h.rows_est);
    let d = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
    let e = still_fits(&h, &d, true, &Progress::new())
        .unwrap_err()
        .to_string();
    assert!(e.contains("big.csv") && e.contains("wider"), "{e}");
    // Titles changed after the head was read.
    std::fs::write(&p, "id,full name\r\n1,a\r\n").unwrap();
    let d = Document::open_indexed(&p, OpenOptions::default(), None).unwrap();
    let h2 = FileHead { cols: 2, ..h };
    assert!(still_fits(&h2, &d, true, &Progress::new())
        .unwrap_err()
        .to_string()
        .contains("titles"));
    let _ = std::fs::remove_dir_all(dir);
}
