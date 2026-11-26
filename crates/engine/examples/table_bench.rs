//! Times table checks against the totals pass they are built on:
//! `cargo run --release --example table_bench -- <file> <col> [runs]` (`col` a number
//! column, counted from 0; the file's first row holds titles).
//! Lines: the column's totals (what the totals row reads), a formula over that column's
//! figures (one totals pass), the same with a "no blanks" check and a row count beside it,
//! and a distinct count (the same work as the status bar's distinct figure).

use std::time::Instant;

use natendo_engine::tablecheck::{Compare, TableCheck, TableCheckKind, Tolerance};
use natendo_engine::validate::CheckSpec;
use natendo_engine::value::ReadOptions;
use natendo_engine::{Document, OpenOptions, Progress};

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn spec(kinds: Vec<TableCheckKind>) -> CheckSpec {
    CheckSpec {
        keep_first: true,
        table_checks: kinds
            .into_iter()
            .map(|kind| TableCheck { name: None, kind })
            .collect(),
        ..Default::default()
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let col: u32 = args[1].parse().expect("col");
    let runs: usize = args.get(2).map_or(9, |s| s.parse().expect("runs"));
    let d = Document::open_indexed(std::path::Path::new(&args[0]), OpenOptions::default(), None)
        .expect("open");
    let p = Progress::new();
    let title = d.expr_names(true)[col as usize].clone();
    let expr = format!(
        "AND(SUM([{t}]) >= MIN([{t}]) * COUNT([{t}]), AVERAGE([{t}]) <= MAX([{t}]))",
        t = title
    );
    let formula = spec(vec![TableCheckKind::Aggregate {
        expr: expr.clone(),
        tolerance: Tolerance::default(),
    }]);
    let mixed = spec(vec![
        TableCheckKind::Aggregate {
            expr,
            tolerance: Tolerance::default(),
        },
        TableCheckKind::NoBlanks { col },
        TableCheckKind::RowCount {
            compare: Compare::AtLeast,
            value: 1,
        },
    ]);
    let distinct = spec(vec![TableCheckKind::Distinct {
        col,
        compare: Compare::AtLeast,
        value: 1,
    }]);
    let (mut tot, mut one, mut mix, mut dis, mut sel) = (vec![], vec![], vec![], vec![], vec![]);
    for _ in 0..runs {
        let t = Instant::now();
        d.column_totals(&[col], true, ReadOptions::default(), &p)
            .expect("totals");
        tot.push(ms(t));
        let t = Instant::now();
        let r = d.run_table_checks(&formula, &p).expect("formula");
        one.push(ms(t));
        assert_eq!(r[0].ok, Some(true), "{r:?}");
        let t = Instant::now();
        d.run_table_checks(&mixed, &p).expect("mixed");
        mix.push(ms(t));
        let t = Instant::now();
        d.run_table_checks(&distinct, &p).expect("distinct");
        dis.push(ms(t));
        let t = Instant::now();
        d.selection_distinct(1, d.row_count() - 1, col, 1, &p)
            .expect("distinct figure");
        sel.push(ms(t));
    }
    let (tot, one, mix, dis, sel) = (
        median(tot),
        median(one),
        median(mix),
        median(dis),
        median(sel),
    );
    println!(
        "{} rows, medians of {runs}: column totals {tot:.1} ms; formula {one:.1} ms ({:.2}x totals); formula + no blanks + row count {mix:.1} ms ({:.2}x); distinct check {dis:.1} ms vs distinct figure {sel:.1} ms ({:.2}x)",
        d.row_count() - 1,
        one / tot,
        mix / tot,
        dis / sel
    );
}
