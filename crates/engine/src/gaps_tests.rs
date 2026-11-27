use super::*;
use crate::filter::{ColumnFilter, Rule};
use crate::ops::CHUNK;
use crate::sort::SortSpec;
use crate::OpenOptions;

fn doc(text: &str) -> Document {
    Document::from_bytes(
        text.as_bytes().to_vec(),
        OpenOptions::default(),
        Some("csv"),
    )
    .unwrap()
}

/// A one-column table titled `N`, with a second column so it reads as CSV.
fn column(values: &[&str]) -> Document {
    let mut s = String::from("N,x\r\n");
    for v in values {
        s.push_str(&format!("\"{}\",1\r\n", v.replace('"', "\"\"")));
    }
    doc(&s)
}

fn spec() -> GapsSpec {
    GapsSpec {
        keep_first: true,
        ..Default::default()
    }
}

fn run_with(d: &Document, s: &GapsSpec) -> GapReport {
    d.gap_audit(s, &Progress::new()).unwrap()
}

fn run(values: &[&str]) -> GapReport {
    run_with(&column(values), &spec())
}

fn lines(r: &GapReport) -> Vec<String> {
    r.ranges.iter().map(RangeLine::text).collect()
}

fn refused(d: &Document, s: &GapsSpec) -> String {
    d.gap_audit(s, &Progress::new()).unwrap_err().to_string()
}

#[test]
fn a_gap_in_the_middle_and_none_at_the_ends() {
    let r = run(&["1001", "1002", "1006", "1007", "1009"]);
    assert_eq!(lines(&r), ["1003 to 1005 (3 missing)", "1008 (1 missing)"]);
    assert_eq!((r.rows, r.distinct, r.missing.as_str()), (5, 5, "4"));
    assert_eq!(r.series[0].first, "1001");
    assert_eq!(r.series[0].last, "1009");
}

#[test]
fn padding_is_kept_and_widths_roll_over() {
    let r = run(&["INV-00998", "INV-01001"]);
    assert_eq!(lines(&r), ["INV-00999 to INV-01000 (2 missing)"]);
    let r = run(&["0998", "0999", "1000", "1001"]);
    assert_eq!((r.range_total, r.other_shape), (0, 0));
    let r = run(&["A-999", "A-1000"]);
    assert_eq!((r.range_total, r.other_shape, r.rows), (0, 0, 2));
}

#[test]
fn two_widths_of_one_prefix_are_two_shapes() {
    let r = run(&["INV-001", "INV-002", "INV-003", "INV-01", "INV-05"]);
    assert_eq!((r.rows, r.other_shape, r.range_total), (3, 2, 0));
    assert_eq!(r.pattern_total, 2);
    assert_eq!(r.other_rows, [Some(5), Some(6)]);
    let mut s = spec();
    s.pattern = Some(Pattern {
        prefix: "INV-".into(),
        suffix: String::new(),
        width: 2,
    });
    let d = column(&["INV-001", "INV-002", "INV-003", "INV-01", "INV-05"]);
    assert_eq!(lines(&run_with(&d, &s)), ["INV-02 to INV-04 (3 missing)"]);
}

#[test]
fn a_short_unpadded_number_beside_padded_ones_is_another_shape() {
    let r = run(&["INV-001", "INV-003", "INV-5"]);
    assert_eq!((r.rows, r.other_shape), (2, 1));
    assert_eq!(lines(&r), ["INV-002 (1 missing)"]);
}

#[test]
fn the_digit_run_that_changes_most_is_picked() {
    let r = run(&["INV-00123/2024", "INV-00125/2024", "INV-00126/2024"]);
    assert_eq!(r.run, 1);
    assert_eq!(lines(&r), ["INV-00124/2024 (1 missing)"]);
    let mut s = spec();
    s.run = Some(0);
    let d = column(&["INV-00123/2024", "INV-00123/2026"]);
    assert_eq!(lines(&run_with(&d, &s)), ["INV-00123/2025 (1 missing)"]);
}

#[test]
fn a_changing_prefix_is_other_shapes_unless_each_shape_is_audited() {
    let values = [
        "A-17-0001",
        "A-17-0003",
        "A-17-0004",
        "A-18-0001",
        "A-18-0004",
    ];
    let r = run(&values);
    assert_eq!((r.rows, r.other_shape), (3, 2));
    assert_eq!(lines(&r), ["A-17-0002 (1 missing)"]);
    let mut s = spec();
    s.each_shape = true;
    let r = run_with(&column(&values), &s);
    assert_eq!((r.rows, r.other_shape, r.series_total), (5, 0, 2));
    assert_eq!(
        lines(&r),
        [
            "A-17-####: A-17-0002 (1 missing)",
            "A-18-####: A-18-0002 to A-18-0003 (2 missing)"
        ]
    );
}

#[test]
fn lookalike_numbers_are_never_misread() {
    // Two digit runs: the run that differs most, and a warning.
    let r = run(&["1,001", "1,002", "1,004"]);
    assert!(r.looks_like_amounts);
    assert_eq!(lines(&r), ["1,003 (1 missing)"]);
    let r = run(&["12.5", "13.5"]);
    assert!(r.looks_like_amounts);
    let r = run(&["-3", "-5"]);
    assert_eq!(r.pattern.as_ref().unwrap().prefix, "-");
    assert_eq!(lines(&r), ["-4 (1 missing)"]);
    assert!(!r.looks_like_amounts);
    // Full-width digits are not digits.
    let r = run(&["１２３", "5", "7"]);
    assert_eq!((r.rows, r.other_shape), (2, 1));
}

#[test]
fn a_huge_range_is_one_line_and_too_big_numbers_are_other_shape() {
    let r = run(&["1", "10000000000"]);
    assert_eq!(lines(&r), ["2 to 9999999999 (9999999998 missing)"]);
    assert_eq!(r.copy_text().lines().count(), 1);
    let t = r.table(&[]).unwrap();
    assert_eq!(t.cols, 3);
    let rows = t.rows;
    let d = t.into_document().unwrap();
    assert_eq!(d.row_count(), rows);
    let last = d.row_values(rows - 1).unwrap();
    assert_eq!(last, ["2", "9999999999", "9999999998"]);
    assert_eq!(d.row_values(rows - 2).unwrap(), ["From", "To", "Missing"]);
    let r = run(&["1", "2", "99999999999999999999", "9223372036854775808"]);
    assert_eq!((r.rows, r.other_shape), (2, 2));
}

#[test]
fn listed_ranges_are_capped_with_exact_totals() {
    let values: Vec<String> = (0..MAX_RANGES + 5).map(|i| (i * 2).to_string()).collect();
    let v: Vec<&str> = values.iter().map(String::as_str).collect();
    let r = run(&v);
    assert_eq!(r.ranges.len(), MAX_RANGES);
    assert_eq!(r.range_total, MAX_RANGES as u64 + 4);
    assert_eq!(r.missing, (MAX_RANGES + 4).to_string());
    assert!(r.copy_text().ends_with("4 more ranges not copied\n"));
}

#[test]
fn duplicates_blanks_and_small_columns() {
    let r = run(&["5", "5", "", "5", "7", "7"]);
    assert_eq!((r.duplicate_values, r.duplicate_extra, r.blank), (2, 3, 1));
    assert_eq!(r.duplicates[0].rows, [Some(2), Some(3), Some(5)]);
    let r = run(&[]);
    assert_eq!((r.rows, r.range_total, r.missing.as_str()), (0, 0, "0"));
    let r = run(&["9"]);
    assert!(r.series[0].too_few);
    let r = run(&["abc", "def"]);
    assert_eq!((r.rows, r.other_shape), (0, 2));
}

#[test]
fn drops_are_counted_in_file_order_even_after_a_sort() {
    let mut d = column(&["1", "2", "3", "10", "4", "5"]);
    let before = run_with(&d, &spec());
    assert_eq!(before.drops, 1);
    assert_eq!(before.drop_rows, [Some(6)]);
    let plan = d
        .plan_sort(
            &SortSpec {
                keys: vec![crate::sort::SortKey {
                    col: 0,
                    descending: true,
                    kind: Default::default(),
                    match_case: false,
                }],
                keep_first: true,
                ..Default::default()
            },
            &Progress::new(),
        )
        .unwrap();
    d.apply_sort(plan).unwrap();
    let after = run_with(&d, &spec());
    assert_eq!(after.drops, 1);
    assert_eq!(after.drop_rows, [Some(6)]);
}

#[test]
fn drops_are_counted_across_a_chunk_edge() {
    let mut values: Vec<String> = (0..CHUNK + 10).map(|i| (i + 100).to_string()).collect();
    // The first row of the second chunk is lower than the last of the first.
    values[CHUNK] = "1".into();
    let v: Vec<&str> = values.iter().map(String::as_str).collect();
    let r = run(&v);
    assert_eq!(r.drops, 1);
}

#[test]
fn typed_start_and_end() {
    let d = column(&["INV-00099", "INV-00103", "INV-00104", "INV-00110"]);
    for start in ["100", "INV-00100"] {
        let mut s = spec();
        s.start = start.into();
        s.end = "INV-00106".into();
        let r = run_with(&d, &s);
        assert_eq!(
            lines(&r),
            [
                "INV-00100 to INV-00102 (3 missing)",
                "INV-00105 to INV-00106 (2 missing)"
            ]
        );
        assert_eq!(r.outside, 2);
    }
    let mut s = spec();
    s.start = "PO-100".into();
    assert!(refused(&d, &s).contains("INV-#####"));
    s.start = "200".into();
    s.end = "100".into();
    assert!(refused(&d, &s).contains("greater than the end"));
    // Nothing inside the typed range.
    s.start = "200".into();
    s.end = "201".into();
    assert_eq!(
        lines(&run_with(&d, &s)),
        ["INV-00200 to INV-00201 (2 missing)"]
    );
    // One row is enough with a typed end.
    let mut s = spec();
    s.end = "12".into();
    assert_eq!(
        lines(&run_with(&column(&["10"]), &s)),
        ["11 to 12 (2 missing)"]
    );
}

#[test]
fn series_are_audited_each_on_their_own() {
    let d = doc("Ward,N\r\n10,1\r\n2,5\r\n10,3\r\n2,6\r\n2,8\r\n9,1\r\n");
    let mut s = spec();
    s.col = 1;
    s.series = vec![0];
    let r = run_with(&d, &s);
    assert_eq!(lines(&r), ["2: 7 (1 missing)", "10: 2 (1 missing)"]);
    assert_eq!(r.series_total, 3);
    assert!(r.series[1].too_few);
    assert_eq!(r.drops, 0);
    assert_eq!(r.table(&["Ward".into()]).unwrap().cols, 4);
    s.series = vec![0, 0, 0];
    assert!(refused(&d, &s).contains("at most 2"));
}

#[test]
fn only_shown_rows_are_read_and_a_changed_table_is_refused() {
    let mut d = doc("N,k\r\n1,y\r\n2,n\r\n3,y\r\n5,y\r\n");
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
    let plan = d.plan_filter(&filter, &Progress::new()).unwrap();
    d.apply_filter(plan).unwrap();
    let r = run_with(&d, &spec());
    assert_eq!(lines(&r), ["2 (1 missing)", "4 (1 missing)"]);
    d.set_cell(1, 0, "9".into()).unwrap();
    match d.plan_show_gaps(&r) {
        Err(e) => assert!(e.to_string().contains("run the audit again")),
        Ok(_) => panic!("a stale audit was used"),
    }
}

#[test]
fn rows_next_to_each_gap_include_every_repeat() {
    let mut d = column(&["1", "2", "2", "4", "4", "5", "6"]);
    let r = run_with(&d, &spec());
    let plan = d.plan_show_gaps(&r).unwrap();
    d.apply_rows(plan).unwrap();
    let shown: Vec<String> = (0..d.row_count())
        .map(|i| d.row_values(i).unwrap()[0].clone())
        .collect();
    assert_eq!(shown, ["N", "2", "2", "4", "4"]);
}

#[test]
fn rows_added_by_hand_join_the_sequence_but_not_the_order_check() {
    let mut d = column(&["1", "2", "5"]);
    d.insert_rows(1, vec![vec!["3".into(), "1".into()]])
        .unwrap();
    let r = run_with(&d, &spec());
    assert_eq!((r.added, r.drops, r.rows), (1, 0, 4));
    assert_eq!(lines(&r), ["4 (1 missing)"]);
}

#[test]
fn cancel_stops_the_audit() {
    let p = Progress::new();
    p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(matches!(
        column(&["1", "3"]).gap_audit(&spec(), &p),
        Err(Error::Cancelled)
    ));
}

#[test]
fn too_many_shapes_are_capped_and_said() {
    // A different suffix on nearly every row, with two common rows spread
    // through chunks that run out of room.
    let n = MAX_SHAPES + CHUNK * 2;
    let values: Vec<String> = (0..n)
        .map(|i| {
            if i % 1000 == 0 {
                format!("INV-{}", i / 1000 + 1)
            } else {
                format!("INV-{i}-x{i}z")
            }
        })
        .collect();
    let v: Vec<&str> = values.iter().map(String::as_str).collect();
    let d = column(&v);
    let mut s = spec();
    // The run just after "INV-": the suffix is different on every row.
    s.run = Some(1);
    let r = run_with(&d, &s);
    assert!(r.too_many_shapes);
    assert_eq!(r.pattern_total, MAX_SHAPES as u64);
    assert_eq!(r.rows + r.other_shape, n as u64);
    // The rows with only one run are not of this run at all.
    s.run = Some(0);
    let r = run_with(&d, &s);
    assert!(r.too_many_shapes);
    assert_eq!(r.pattern.as_ref().unwrap().prefix, "INV-");
    // The common shape is kept in every chunk, also those read again.
    assert_eq!(r.rows, n.div_ceil(1000) as u64);
    assert_eq!(r.range_total, 0);
    assert_eq!(r.rows + r.other_shape, n as u64);
    s.each_shape = true;
    assert!(refused(&d, &s).contains("too many shapes to audit each"));
    let few = run(&["1", "2"]);
    assert!(!few.too_many_shapes);
}

#[test]
fn a_typed_range_needs_a_number_somewhere() {
    let d = column(&["abc", ""]);
    let mut s = spec();
    s.start = "1".into();
    assert!(refused(&d, &s).contains("no cell holds a number"));
    s.each_shape = true;
    assert!(refused(&d, &s).contains("no cell holds a number"));
}

/// A typed start and end against a plain model: the missing numbers and the
/// rows outside the range.
#[test]
fn typed_ranges_agree_with_a_model() {
    let mut seed = 0x853C_49E6_748F_EA9Bu64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    for round in 0..1_500 {
        let n = next(12) + 1;
        let span = next(40) + 1;
        let values: Vec<u64> = (0..n).map(|_| next(span) + 5).collect();
        let text: Vec<String> = values.iter().map(u64::to_string).collect();
        let v: Vec<&str> = text.iter().map(String::as_str).collect();
        let mut s = spec();
        let start = (next(3) > 0).then(|| next(span + 10));
        let end = (next(3) > 0).then(|| start.unwrap_or(0) + next(span + 10));
        s.start = start.map_or(String::new(), |x| x.to_string());
        s.end = end.map_or(String::new(), |x| x.to_string());
        let d = column(&v);
        let r = run_with(&d, &s);
        let present: std::collections::BTreeSet<u64> = values.iter().copied().collect();
        let outside = values
            .iter()
            .filter(|&&x| start.is_some_and(|a| x < a) || end.is_some_and(|b| x > b))
            .count() as u64;
        let lo = start.unwrap_or(*present.first().unwrap());
        let hi = end.unwrap_or(*present.last().unwrap());
        let checked = values.len() >= 2 || start.is_some() || end.is_some();
        let want: Vec<u64> = if checked {
            (lo..=hi).filter(|x| !present.contains(x)).collect()
        } else {
            Vec::new()
        };
        let got: Vec<u64> = r
            .ranges
            .iter()
            .flat_map(|g| g.from.parse::<u64>().unwrap()..=g.to.parse::<u64>().unwrap())
            .collect();
        let say = format!("round {round}: {values:?} from {start:?} to {end:?}");
        assert_eq!(got, want, "{say}");
        assert_eq!(r.missing, want.len().to_string(), "{say}");
        assert_eq!(r.outside, outside, "{say}");
    }
}

#[test]
fn saved_rows_added_by_hand_get_file_numbers() {
    let mut d = column(&["1", "3"]);
    d.insert_rows(1, vec![vec!["5".into(), "1".into()]])
        .unwrap();
    assert_eq!(run_with(&d, &spec()).added, 1);
    let path = std::env::temp_dir().join(format!("nde-gaps-{}.csv", std::process::id()));
    d.save(crate::save::SaveOptions {
        path: Some(path.clone()),
        ..Default::default()
    })
    .unwrap();
    let r = run_with(&d, &spec());
    std::fs::remove_file(path).ok();
    assert_eq!((r.added, r.drops, r.drop_rows), (0, 1, vec![Some(3)]));
}

/// Present numbers plus expanded ranges are exactly first to last, per series.
#[test]
fn present_and_missing_make_the_whole_range() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    for _ in 0..200 {
        let n = next(60) + 2;
        let span = next(200) + 1;
        let rows: Vec<(u64, u64)> = (0..n).map(|_| (next(3), next(span))).collect();
        let mut s = String::from("S,N\r\n");
        for (a, b) in &rows {
            s.push_str(&format!("{a},{b}\r\n"));
        }
        let mut sp = spec();
        sp.col = 1;
        sp.series = vec![0];
        sp.run = Some(0);
        let r = run_with(&doc(&s), &sp);
        for series in 0..3u64 {
            let present: std::collections::BTreeSet<u64> =
                rows.iter().filter(|x| x.0 == series).map(|x| x.1).collect();
            let mut all = present.clone();
            for line in r.ranges.iter().filter(|l| l.series == [series.to_string()]) {
                let (a, b): (u64, u64) = (line.from.parse().unwrap(), line.to.parse().unwrap());
                for v in a..=b {
                    assert!(all.insert(v), "{v} listed as missing but present");
                }
            }
            let want: Vec<u64> = match (present.first(), present.last()) {
                (Some(&a), Some(&b)) if rows.iter().filter(|x| x.0 == series).count() >= 2 => {
                    (a..=b).collect()
                }
                _ => present.iter().copied().collect(),
            };
            assert_eq!(all.into_iter().collect::<Vec<_>>(), want);
        }
    }
}

// ---- dates ----
use crate::coltype::{ColumnType, ColumnTypes};
use crate::fill::{civil_from_days, days_from_civil};
use crate::value::DateOrder;

include!("gaps_date_tests.rs");
