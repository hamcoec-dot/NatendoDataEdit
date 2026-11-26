// The date mode of the gap audit (included at the end of `gaps_tests.rs`).

fn dated(step: DateStep) -> GapsSpec {
    GapsSpec {
        dates: Some(step),
        ..spec()
    }
}

fn run_dates(values: &[&str], step: DateStep) -> GapReport {
    run_with(&column(values), &dated(step))
}

fn typed_as(ty: ColumnType, values: &[&str]) -> Document {
    let mut d = column(values);
    d.set_column_types(ColumnTypes::default().with(0, ty));
    d
}

#[test]
fn days_over_a_leap_day_and_a_month_end() {
    let r = run_dates(
        &["2028-02-27", "2028-03-01", "2028-03-31", "2028-03-02"],
        DateStep::Day,
    );
    assert_eq!(
        lines(&r),
        [
            "2028-02-28 to 2028-02-29 (2 missing)",
            "2028-03-03 to 2028-03-30 (28 missing)"
        ]
    );
    assert_eq!(r.drops, 1);
    assert_eq!(r.series[0].first, "2028-02-27");
    // Before 1970 too.
    let r = run_dates(&["1969-12-30", "1970-01-02"], DateStep::Day);
    assert_eq!(lines(&r), ["1969-12-31 to 1970-01-01 (2 missing)"]);
}

#[test]
fn a_time_of_day_counts_on_its_day() {
    let r = run_dates(
        &["2026-03-02 08:00", "2026-03-02 17:30", "2026-03-04"],
        DateStep::Day,
    );
    assert_eq!((r.duplicate_values, r.duplicate_extra), (1, 1));
    assert_eq!(r.duplicates[0].value, "2026-03-02");
    assert_eq!(lines(&r), ["2026-03-03 (1 missing)"]);
}

#[test]
fn business_days_skip_weekends() {
    // Thursday 26 November 2026, the weekend, then Tuesday 1 December.
    let r = run_dates(
        &["2026-11-26", "2026-11-28", "2026-12-01", "2026-11-29"],
        DateStep::Business,
    );
    assert_eq!(lines(&r), ["2026-11-27 to 2026-11-30 (2 missing)"]);
    let d = r.dates.as_ref().unwrap();
    assert_eq!(
        (d.weekend, d.weekend_rows.clone()),
        (2, vec![Some(3), Some(5)])
    );
    let table = r.table(&[]).unwrap();
    assert!(String::from_utf8(table.bytes)
        .unwrap()
        .contains("On a weekend,2"));
    let mut s = dated(DateStep::Business);
    s.start = "2026-11-28".into();
    let d = column(&["2026-12-01"]);
    assert!(refused(&d, &s).contains("2026-11-28 is a Saturday"));
    s.start = String::new();
    s.end = "2026-11-29".into();
    assert!(refused(&d, &s).contains("is a Sunday"));
}

#[test]
fn months_count_several_rows_as_repeats() {
    let r = run_dates(
        &["2025-11-03", "2025-11-20", "2026-02-01", "Jan 5, 2026"],
        DateStep::Month,
    );
    assert_eq!(lines(&r), ["2025-12 (1 missing)"]);
    assert_eq!(r.duplicates[0].value, "2025-11");
    let mut s = dated(DateStep::Month);
    s.start = "2025-09".into();
    s.end = "2026-03-15".into();
    let d = column(&["2025-11-03", "2025-12-01"]);
    assert_eq!(
        lines(&run_with(&d, &s)),
        [
            "2025-09 to 2025-10 (2 missing)",
            "2026-01 to 2026-03 (3 missing)"
        ]
    );
}

#[test]
fn weeks_and_every_n_days_list_dates_off_the_step() {
    let r = run_dates(
        &["2026-03-02", "2026-03-09", "2026-03-11", "2026-03-23"],
        DateStep::Week,
    );
    assert_eq!(lines(&r), ["2026-03-16 (1 missing)"]);
    let d = r.dates.as_ref().unwrap();
    assert_eq!((d.off_step, d.off_step_rows.clone()), (1, vec![Some(4)]));
    // A typed start off the dates sets the step; a typed end off the step
    // stops at the step before.
    let mut s = dated(DateStep::Days { n: 10 });
    s.start = "2026-01-01".into();
    s.end = "2026-02-05".into();
    let d = column(&["2026-01-11", "2026-01-12"]);
    let r = run_with(&d, &s);
    assert_eq!(
        lines(&r),
        [
            "2026-01-01 (1 missing)",
            "2026-01-21 to 2026-01-31 (2 missing)"
        ]
    );
    assert_eq!(r.dates.unwrap().off_step, 1);
    assert!(refused(&d, &dated(DateStep::Days { n: 0 })).contains("1 to 3660"));
}

#[test]
fn ambiguous_dates_are_never_guessed() {
    let values = [
        "2026-03-01",
        "03/02/2026",
        "03/13/2026",
        "2026-03-05",
        "05/05/2026",
    ];
    let r = run_dates(&values, DateStep::Day);
    let d = r.dates.clone().unwrap();
    assert_eq!((d.ambiguous, d.ambiguous_rows), (1, vec![Some(3)]));
    // 03/13 can only be month first; 05/05 is the same either way.
    assert_eq!(r.rows, 4);
    assert!(lines(&r)[0].starts_with("2026-03-02 to 2026-03-04"));
    // With the column set to an order, they are read in it; a date only the
    // other order reads is another shape.
    let doc = typed_as(
        ColumnType::Date(Some(DateOrder::Dmy)),
        &["01/03/2026", "03/03/2026", "13/03/2026", "03/14/2026"],
    );
    let r = run_with(&doc, &dated(DateStep::Day));
    assert_eq!(
        (r.rows, r.other_shape, r.dates.clone().unwrap().ambiguous),
        (3, 1, 0)
    );
    assert_eq!(r.other_rows, [Some(5)]);
    assert_eq!(r.series[0].first, "2026-03-01");
    assert_eq!(r.series[0].last, "2026-03-13");
}

#[test]
fn two_digit_years_are_counted_with_their_rows() {
    let r = run_dates(&["12/31/29", "01/15/30", "2029-12-30"], DateStep::Month);
    let d = r.dates.clone().unwrap();
    assert_eq!((d.two_digit, d.two_digit_rows), (2, vec![Some(2), Some(3)]));
    assert_eq!(r.series[0].first, "1930-01");
}

#[test]
fn eight_digit_numbers_follow_the_date_radar() {
    // A registration number column: never dates, whatever they look like.
    let d = doc("Reg No,x\r\n20260101,1\r\n20260105,1\r\n2026-01-03,1\r\n");
    let r = run_with(&d, &dated(DateStep::Day));
    assert_eq!((r.rows, r.other_shape), (1, 2));
    // Elsewhere, read when most of the column is dates.
    let d = doc("Day,x\r\n20260101,1\r\n20260105,1\r\n2026-01-03,1\r\n");
    let r = run_with(&d, &dated(DateStep::Day));
    assert_eq!((r.rows, r.other_shape), (3, 0));
    // Text and Id columns are not read as dates.
    let d = typed_as(ColumnType::Id, &["2026-01-01"]);
    assert!(refused(&d, &dated(DateStep::Day)).contains("Id"));
}

#[test]
fn typed_dates_and_series() {
    let d = column(&["2026-01-05"]);
    let mut s = dated(DateStep::Day);
    for bad in ["01/05/2026", "2026-1-5", "2026-02-30", "2026-01", "soon"] {
        s.start = bad.into();
        assert!(
            refused(&d, &s).contains("type the start as a date"),
            "{bad}"
        );
    }
    s.start = "2026-01-07".into();
    s.end = "2026-01-06".into();
    assert!(refused(&d, &s).contains("greater than the end"));
    let d = doc("Box,Day\r\nb,2026-01-03\r\na,2026-01-01\r\nb,2026-01-01\r\na,2026-01-02\r\n");
    let mut s = dated(DateStep::Day);
    s.col = 1;
    s.series = vec![0];
    let r = run_with(&d, &s);
    assert_eq!(lines(&r), ["b: 2026-01-02 (1 missing)"]);
    assert_eq!((r.series_total, r.drops), (2, 1));
}

/// Day and N-day steps against a plain model.
#[test]
fn date_steps_agree_with_a_model() {
    let mut seed = 0xDA3E_39CB_94B9_5BDBu64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    let iso = |d: i64| {
        let (y, m, day) = civil_from_days(d);
        format!("{y:04}-{m:02}-{day:02}")
    };
    let day_of = |s: &str| {
        let p: Vec<i64> = s.split('-').map(|x| x.parse().unwrap()).collect();
        days_from_civil(p[0], p[1] as u32, p[2] as u32)
    };
    for round in 0..400 {
        let step = next(9) as i64 + 1;
        let base = days_from_civil(1968, 6, 1) + next(30_000) as i64;
        let days: Vec<i64> = (0..next(15) + 2).map(|_| base + next(80) as i64).collect();
        let text: Vec<String> = days.iter().map(|&d| iso(d)).collect();
        let v: Vec<&str> = text.iter().map(String::as_str).collect();
        let kind = if step == 1 {
            DateStep::Day
        } else {
            DateStep::Days { n: step as u32 }
        };
        let r = run_dates(&v, kind);
        let anchor = *days.iter().min().unwrap();
        let on: Vec<i64> = days
            .iter()
            .copied()
            .filter(|d| (d - anchor) % step == 0)
            .collect();
        let set: std::collections::BTreeSet<i64> = on.iter().copied().collect();
        let (a, b) = (*set.first().unwrap(), *set.last().unwrap());
        let want: Vec<String> = if on.len() < 2 {
            Vec::new()
        } else {
            (0..=(b - a) / step)
                .map(|i| a + i * step)
                .filter(|d| !set.contains(d))
                .map(iso)
                .collect()
        };
        let got: Vec<String> = r
            .ranges
            .iter()
            .flat_map(|g| {
                let (from, to) = (day_of(&g.from), day_of(&g.to));
                (0..=(to - from) / step).map(move |i| from + i * step)
            })
            .map(iso)
            .collect();
        assert_eq!(got, want, "round {round}: step {step} {text:?}");
        assert_eq!(r.dates.unwrap().off_step as usize, days.len() - on.len());
    }
}
