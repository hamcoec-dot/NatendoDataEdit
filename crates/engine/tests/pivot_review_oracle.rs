//! Reviewer's brute-force oracle for pivot-v2 (not part of the branch).
use natendo_engine::document::Progress;
use natendo_engine::pivot::*;
use natendo_engine::{Document, OpenOptions};
use std::collections::{BTreeSet, HashMap, HashSet};

struct Rng(u64);
impl Rng {
    fn n(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

/// Raw cells per row: a, b, d, n, t
type Row = Vec<String>;

fn make(rows: usize, seed: u64) -> (Document, Vec<Row>) {
    let mut r = Rng(seed);
    let mut s = String::from("a,b,d,n,t\n");
    let mut data = Vec::new();
    for _ in 0..rows {
        let a = ["x", "X", " x", "y", "Y ", "z", "", "  ", "Zed", "zed"][r.n(10) as usize];
        let b = r.n(9).to_string();
        let b = if r.n(5) == 0 { format!(" {b}") } else { b };
        let d = ["north", "south", "east", "west", ""][r.n(5) as usize];
        let n = match r.n(12) {
            0 => String::new(),
            1 => "x".into(),
            2 => " 12.5 ".into(),
            _ => format!("{}.{:02}", r.n(2000) as i64 - 700, r.n(100)),
        };
        let t = ["Ann", "ann", "ANN ", "bob", "cy", "", "dee", "Bob"][r.n(8) as usize];
        s.push_str(&format!("{a},{b},{d},{n},{t}\n"));
        data.push(vec![a.into(), b, d.into(), n, t.into()]);
    }
    let doc = Document::from_bytes(s.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
    (doc, data)
}

fn key(t: &str) -> String {
    t.trim().to_lowercase()
}

fn num(t: &str) -> Option<f64> {
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok()
}

fn agg(agg: Agg, vals: &[&str], ci_distinct: bool) -> Option<f64> {
    let nums: Vec<f64> = vals.iter().filter_map(|v| num(v)).collect();
    let nb: Vec<&&str> = vals.iter().filter(|v| !v.trim().is_empty()).collect();
    match agg {
        Agg::Count => Some(nb.len() as f64),
        Agg::CountRows => Some(vals.len() as f64),
        Agg::CountNumbers => Some(nums.len() as f64),
        Agg::CountBlank => Some((vals.len() - nb.len()) as f64),
        Agg::CountDistinct => {
            let s: HashSet<String> = nb
                .iter()
                .map(|v| {
                    if ci_distinct {
                        key(v)
                    } else {
                        v.trim().to_string()
                    }
                })
                .collect();
            Some(s.len() as f64)
        }
        Agg::Sum => (!nums.is_empty()).then(|| nums.iter().sum()),
        Agg::Average => (!nums.is_empty()).then(|| nums.iter().sum::<f64>() / nums.len() as f64),
        Agg::Min => nums.iter().copied().reduce(f64::min),
        Agg::Max => nums.iter().copied().reduce(f64::max),
        Agg::Median => {
            if nums.is_empty() {
                return None;
            }
            let mut v = nums.clone();
            v.sort_by(f64::total_cmp);
            let m = v.len() / 2;
            Some(if v.len() % 2 == 1 {
                v[m]
            } else {
                (v[m - 1] + v[m]) / 2.0
            })
        }
        Agg::StdDev => {
            if nums.len() < 2 {
                return None;
            }
            let mean = nums.iter().sum::<f64>() / nums.len() as f64;
            let ss: f64 = nums.iter().map(|x| (x - mean).powi(2)).sum();
            Some((ss / (nums.len() - 1) as f64).sqrt())
        }
        _ => unreachable!(),
    }
}

fn close(a: Option<f64>, cell: &str) -> bool {
    match (a, cell.trim()) {
        (None, "") => true,
        (Some(x), s) if !s.is_empty() => {
            let y: f64 = s.replace(',', "").parse().unwrap();
            (x - y).abs() <= 1e-6 * x.abs().max(1.0)
        }
        _ => false,
    }
}

fn titles(d: &Document) -> Vec<String> {
    let all: Vec<u32> = (0..d.col_count()).collect();
    d.titles(&all, true)
}

fn f(col: u32) -> PivotField {
    PivotField {
        col,
        ..Default::default()
    }
}
fn v(col: u32, agg: Agg) -> ValueField {
    ValueField {
        col,
        agg,
        ..Default::default()
    }
}

/// Check every cell (items, subtotals, grand) of a grid against a recount.
fn check(d: &Document, data: &[Row], spec: &PivotSpec, ci_distinct: bool) -> (usize, Vec<String>) {
    let cube = d.pivot_cube(spec, None, &Progress::new()).unwrap();
    let g = cube.layout(spec, &titles(d), None).unwrap();
    // Rows that pass report filters and item picks.
    let pass = |row: &Row| -> bool {
        for fl in spec.filters.iter().chain(&spec.rows).chain(&spec.cols) {
            if let Some(p) = &fl.items {
                let k = key(&row[fl.col as usize]);
                if p.items.contains(&k) == p.exclude {
                    return false;
                }
            }
        }
        true
    };
    let rows: Vec<&Row> = data.iter().filter(|r| pass(r)).collect();
    // Group rows by every (row prefix, col prefix).
    let mut groups: HashMap<(Vec<String>, Vec<String>), Vec<usize>> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        let rk: Vec<String> = spec
            .rows
            .iter()
            .map(|fl| key(&r[fl.col as usize]))
            .collect();
        let ckk: Vec<String> = spec
            .cols
            .iter()
            .map(|fl| key(&r[fl.col as usize]))
            .collect();
        for a in 0..=rk.len() {
            for b in 0..=ckk.len() {
                groups
                    .entry((rk[..a].to_vec(), ckk[..b].to_vec()))
                    .or_default()
                    .push(i);
            }
        }
    }
    let empty = Vec::new();
    let mut bad = Vec::new();
    let mut n = 0;
    for gr in &g.rows {
        if gr.kind == Kind::Group && !spec.layout.subtotals_top {
            assert!(gr.cells.iter().all(|c| c.is_empty()));
            continue;
        }
        for (i, gc) in g.cols.iter().enumerate() {
            let ids = groups
                .get(&(gr.path.clone(), gc.path.clone()))
                .unwrap_or(&empty);
            let hit: Vec<&str> = ids
                .iter()
                .map(|&j| rows[j][spec.values[gc.value as usize].col as usize].as_str())
                .collect();
            let a = spec.values[gc.value as usize].agg;
            let want = if hit.is_empty() {
                None
            } else {
                agg(a, &hit, ci_distinct)
            };
            n += 1;
            if !close(want, &gr.cells[i]) {
                bad.push(format!(
                    "{:?} row {:?} col {:?} v{} {:?}: want {:?} got {:?}",
                    gr.kind, gr.path, gc.path, gc.value, a, want, gr.cells[i]
                ));
            }
        }
    }
    (n, bad)
}

#[test]
fn review_oracle_all_counts_and_sums() {
    let (d, data) = make(3 * 32768 + 1234, 0x1234_5678_9abc_def1);
    let aggs = [
        Agg::Count,
        Agg::CountRows,
        Agg::CountNumbers,
        Agg::CountBlank,
        Agg::Sum,
        Agg::Average,
        Agg::Min,
        Agg::Max,
        Agg::Median,
        Agg::StdDev,
    ];
    let layouts = [
        PivotLayout {
            subtotals_top: false,
            ..Default::default()
        },
        PivotLayout {
            subtotals_top: true,
            ..Default::default()
        },
    ];
    let mut total = 0;
    let mut all_bad = Vec::new();
    for (ri, rows) in [vec![0], vec![0, 1], vec![2, 0, 1], vec![]]
        .iter()
        .enumerate()
    {
        for cols in [vec![], vec![2], vec![1, 2]] {
            if rows.iter().any(|r| cols.contains(r)) {
                continue;
            }
            for lay in &layouts {
                let spec = PivotSpec {
                    rows: rows.iter().map(|&c| f(c)).collect(),
                    cols: cols.iter().map(|&c| f(c)).collect(),
                    values: aggs.iter().map(|&a| v(3, a)).collect(),
                    layout: lay.clone(),
                    ..Default::default()
                };
                let (n, bad) = check(&d, &data, &spec, false);
                total += n;
                if !bad.is_empty() {
                    all_bad.push(format!(
                        "spec rows {rows:?} cols {cols:?} top {} : {} bad, first {:?}",
                        lay.subtotals_top,
                        bad.len(),
                        &bad[..bad.len().min(3)]
                    ));
                }
                let _ = ri;
            }
        }
    }
    println!("checked {total} cells");
    assert!(all_bad.is_empty(), "{all_bad:#?}");
}

#[test]
fn review_oracle_filters_and_picks() {
    let (d, data) = make(2 * 32768 + 99, 0xdead_beef_cafe_f00d);
    let mut bad_all = Vec::new();
    let mut total = 0;
    let picks = [
        (2u32, false, vec!["north", "east", ""]),
        (0u32, true, vec!["x", ""]),
        (1u32, false, vec!["1", "3", "5", "7"]),
    ];
    for (pc, ex, items) in &picks {
        let mut spec = PivotSpec {
            rows: vec![f(0), f(1)],
            cols: vec![f(2)],
            values: vec![
                v(3, Agg::Sum),
                v(3, Agg::CountRows),
                v(4, Agg::Count),
                v(3, Agg::Median),
            ],
            filters: vec![],
            ..Default::default()
        };
        let pick = Some(ItemPick {
            exclude: *ex,
            items: items.iter().map(|s| s.to_string()).collect(),
        });
        // as a report filter on another column
        let mut s1 = spec.clone();
        let mut fl = f(*pc);
        fl.items = pick.clone();
        s1.filters.push(fl);
        let (n, b) = check(&d, &data, &s1, false);
        total += n;
        if !b.is_empty() {
            bad_all.push(format!(
                "report filter {pc}: {} bad {:?}",
                b.len(),
                &b[..3.min(b.len())]
            ));
        }
        // as a pick on the axis field itself
        for fld in spec.rows.iter_mut().chain(spec.cols.iter_mut()) {
            if fld.col == *pc {
                fld.items = pick.clone();
            }
        }
        let (n, b) = check(&d, &data, &spec, false);
        total += n;
        if !b.is_empty() {
            bad_all.push(format!(
                "axis pick {pc}: {} bad {:?}",
                b.len(),
                &b[..3.min(b.len())]
            ));
        }
    }
    println!("checked {total} cells");
    assert!(bad_all.is_empty(), "{bad_all:#?}");
}

#[test]
fn review_distinct_count_semantics() {
    let (d, data) = make(40_000, 77);
    let spec = PivotSpec {
        rows: vec![f(0)],
        cols: vec![f(2)],
        values: vec![v(4, Agg::CountDistinct)],
        ..Default::default()
    };
    let (_, bad_cs) = check(&d, &data, &spec, false);
    let (_, bad_ci) = check(&d, &data, &spec, true);
    println!(
        "distinct case-sensitive mismatches: {}, case-insensitive mismatches: {}",
        bad_cs.len(),
        bad_ci.len()
    );
    if let Some(x) = bad_ci.first() {
        println!("example vs case-insensitive: {x}");
    }
    // Expected after fix 3: distinct count follows match_case (off by default).
    assert!(bad_ci.is_empty(), "{:?}", &bad_ci[..3.min(bad_ci.len())]);
    let _ = bad_cs;
}

#[test]
fn review_top_n_and_value_filter() {
    let (d, data) = make(70_000, 4242);
    // Rows a, top 3 by sum; columns d.
    let mut fa = f(0);
    fa.top = Some(TopN {
        n: 3,
        value: 0,
        bottom: false,
    });
    let spec = PivotSpec {
        rows: vec![fa],
        cols: vec![f(2)],
        values: vec![v(3, Agg::Sum), v(3, Agg::CountRows)],
        ..Default::default()
    };
    let cube = d.pivot_cube(&spec, None, &Progress::new()).unwrap();
    let g = cube.layout(&spec, &titles(&d), None).unwrap();
    let mut sums: HashMap<String, f64> = HashMap::new();
    for r in &data {
        if let Some(x) = num(&r[3]) {
            *sums.entry(key(&r[0])).or_default() += x;
        } else {
            sums.entry(key(&r[0])).or_default();
        }
    }
    let mut ranked: Vec<(String, f64)> = sums.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let want: BTreeSet<String> = ranked.iter().take(3).map(|x| x.0.clone()).collect();
    let got: BTreeSet<String> = g
        .rows
        .iter()
        .filter(|r| r.kind == Kind::Item)
        .map(|r| r.path[0].clone())
        .collect();
    println!("top3 want {want:?} got {got:?}");
    assert_eq!(want, got);
    // grand total = only kept items
    let grand = g.rows.last().unwrap();
    let ci = g
        .cols
        .iter()
        .position(|c| c.kind == Kind::Grand && c.value == 1)
        .unwrap();
    let kept_rows = data.iter().filter(|r| want.contains(&key(&r[0]))).count();
    println!(
        "grand rows shown {} (kept rows {kept_rows}, all {})",
        grand.cells[ci],
        data.len()
    );
    assert_eq!(grand.cells[ci], kept_rows.to_string());

    // Drill-down of each item / grand cell matches the Rows count.
    for r in &g.rows {
        for (i, c) in g.cols.iter().enumerate() {
            if c.value != 1 || r.cells[i].is_empty() {
                continue;
            }
            let t = d
                .pivot_drill(&spec, &cube, &r.path, &c.path, &Progress::new())
                .unwrap();
            assert_eq!(
                (t.rows - 1).to_string(),
                r.cells[i],
                "drill {:?} {:?}",
                r.path,
                c.path
            );
        }
    }

    // Value filter: sum > 0 on b under a
    let mut fb = f(1);
    fb.value_filter = Some(ValueFilter {
        op: CmpOp::Greater,
        a: 0.0,
        b: 0.0,
        value: 0,
    });
    let spec2 = PivotSpec {
        rows: vec![f(0), fb],
        values: vec![v(3, Agg::Sum)],
        ..Default::default()
    };
    let cube2 = d.pivot_cube(&spec2, None, &Progress::new()).unwrap();
    let g2 = cube2.layout(&spec2, &titles(&d), None).unwrap();
    let mut s2: HashMap<(String, String), f64> = HashMap::new();
    for r in &data {
        let e = s2.entry((key(&r[0]), key(&r[1]))).or_default();
        if let Some(x) = num(&r[3]) {
            *e += x;
        }
    }
    for r in g2.rows.iter().filter(|r| r.kind == Kind::Item) {
        let k = (r.path[0].clone(), r.path[1].clone());
        assert!(s2[&k] > 0.0, "{k:?} {}", s2[&k]);
    }
    let shown = g2.rows.iter().filter(|r| r.kind == Kind::Item).count();
    let want_n = s2.values().filter(|&&x| x > 0.0).count();
    assert_eq!(shown, want_n);
}

#[test]
fn review_five_fields_boxed_keys() {
    // 5+ fields uses the boxed key path.
    let (d, data) = make(40_000, 99);
    let spec = PivotSpec {
        rows: vec![f(0), f(1), f(2)],
        cols: vec![f(4), f(3)],
        values: vec![v(3, Agg::CountRows)],
        layout: PivotLayout {
            subtotals_top: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let (n, bad) = check(&d, &data, &spec, false);
    println!("boxed: checked {n}");
    assert!(bad.is_empty(), "{:?}", &bad[..3.min(bad.len())]);
}

#[test]
fn review_saved_layout_hostile() {
    let (d, _) = make(100, 1);
    let t = titles(&d);
    for bad in [
        &br#"{"version":1,"name":"x","spec":{"rows":[{"col":4000000000}]},"columns":[]}"#[..],
        br#"{"version":1,"name":"x","spec":{"rows":[{"col":0,"group":{"kind":"bins","start":0,"size":0}}]},"columns":[[0,"a"]]}"#,
        br#"{"version":1,"name":"x","spec":{"rows":[{"col":3,"group":{"kind":"bins","start":-1e308,"size":1e-310}}],"values":[{"col":3,"agg":"count"}]},"columns":[[3,"n"]]}"#,
        br#"{"version":1,"name":"x","spec":{"rows":[{"col":0,"sort":{"by":"value","value":7}}],"values":[{"col":3}]},"columns":[[0,"a"],[3,"n"]]}"#,
        br#"{"version":1,"name":"../../etc","spec":{},"columns":[[0,"../a"]]}"#,
        br#"{"version":1,"name":"x","spec":{"values":[{"col":3,"agg":"sum","separator":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}],"rows":[{"col":0,"top":{"n":4294967295,"value":9}}]},"columns":[[3,"n"]]}"#,
        br#"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[["#,
    ] {
        match SavedPivot::from_json(bad) {
            Err(e) => println!("rejected at read: {e}"),
            Ok(s) => match s.fit(&t) {
                Err(e) => println!("rejected at fit: {e}"),
                Ok(spec) => match d.pivot_cube(&spec, None, &Progress::new()) {
                    Err(e) => println!("rejected at check: {e}"),
                    Ok(c) => {
                        let r = c.layout(&spec, &t, None);
                        println!("ACCEPTED: layout ok = {}", r.is_ok());
                    }
                },
            },
        }
    }
}

/// The app reuses a cached cube when only layout settings change, and then
/// calls `layout` without `spec.check`. What does layout do with bad
/// indices?
#[test]
fn review_layout_without_check() {
    let (d, _) = make(5000, 5);
    let t = titles(&d);
    let good = PivotSpec {
        rows: vec![f(0)],
        cols: vec![f(2)],
        values: vec![v(3, Agg::Sum), v(3, Agg::Median)],
        ..Default::default()
    };
    let cube = d.pivot_cube(&good, None, &Progress::new()).unwrap();
    let cases: Vec<(&str, PivotSpec)> = vec![
        ("sort by value 99", {
            let mut s = good.clone();
            s.rows[0].sort = FieldSort {
                by: SortBy::Value,
                value: 99,
                desc: false,
            };
            s
        }),
        ("col sort by value 99", {
            let mut s = good.clone();
            s.cols[0].sort = FieldSort {
                by: SortBy::Value,
                value: 99,
                desc: false,
            };
            s
        }),
        ("top n value 99", {
            let mut s = good.clone();
            s.rows[0].top = Some(TopN {
                n: 2,
                value: 99,
                bottom: false,
            });
            s
        }),
        ("value filter 99", {
            let mut s = good.clone();
            s.rows[0].value_filter = Some(ValueFilter {
                value: 99,
                ..Default::default()
            });
            s
        }),
        ("decimals 200", {
            let mut s = good.clone();
            s.values[0].format.decimals = Some(200);
            s
        }),
    ];
    let mut panics = Vec::new();
    for (name, s) in cases {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cube.layout(&s, &t, Some(GridLimit { rows: 50, cols: 50 }))
        }));
        match r {
            Err(_) => {
                println!("LAYOUT-NO-CHECK {name}: PANIC");
                panics.push(name);
            }
            Ok(Err(e)) => println!("LAYOUT-NO-CHECK {name}: error {e}"),
            Ok(Ok(_)) => println!("LAYOUT-NO-CHECK {name}: accepted"),
        }
    }
    // Expected after fix 1: bad indices are errors, never panics.
    assert!(panics.is_empty(), "layout panicked on: {panics:?}");
}

#[test]
fn review_drill_with_filters_and_subtotals() {
    let (d, _) = make(50_000, 31337);
    let mut fl = f(4);
    fl.items = Some(ItemPick {
        exclude: true,
        items: vec!["bob".into(), "".into()],
    });
    let mut fb = f(1);
    fb.items = Some(ItemPick {
        exclude: false,
        items: vec!["1".into(), "2".into(), "3".into()],
    });
    let spec = PivotSpec {
        rows: vec![f(0), fb],
        cols: vec![f(2)],
        values: vec![v(3, Agg::CountRows)],
        filters: vec![fl],
        ..Default::default()
    };
    let cube = d.pivot_cube(&spec, None, &Progress::new()).unwrap();
    let g = cube.layout(&spec, &titles(&d), None).unwrap();
    let mut n = 0;
    for r in &g.rows {
        for (i, c) in g.cols.iter().enumerate() {
            if r.cells[i].is_empty() {
                continue;
            }
            let t = d
                .pivot_drill(&spec, &cube, &r.path, &c.path, &Progress::new())
                .unwrap();
            assert_eq!(
                (t.rows - 1).to_string(),
                r.cells[i],
                "drill {:?} {:?} {:?}",
                r.kind,
                r.path,
                c.path
            );
            n += 1;
        }
    }
    println!("drill cells checked {n}");
}
