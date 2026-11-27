//! Times the step the app adds before a fill: finding which of the user's
//! saved custom orders it follows (`Document::fill_orders_used`), with the
//! worst saved set (60 orders of 10,000 made-up values, none ticked for one
//! cell). Fills that use no order must add almost nothing.
//! `cargo run --release --example fill_orders_prepass -- [runs]`

use natendo_engine::fill::FillOrder;
use natendo_engine::ops::{Dir, FillSpec};
use natendo_engine::{Document, OpenOptions, Progress};
use std::time::Instant;

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn time(runs: usize, mut f: impl FnMut()) -> f64 {
    median(
        (0..runs)
            .map(|_| {
                let s = Instant::now();
                f();
                s.elapsed().as_secs_f64() * 1000.0
            })
            .collect(),
    )
}

/// A made-up word for a number (letters only: a digit makes numbered text).
fn word(mut n: usize) -> String {
    let mut s = String::new();
    loop {
        s.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
        if n == 0 {
            return s;
        }
    }
}

fn main() {
    let runs: usize = std::env::args().nth(1).map_or(15, |s| s.parse().expect("runs"));
    let saved: Vec<FillOrder> = (0..60)
        .map(|j| FillOrder {
            name: format!("Order {}", word(j)),
            items: (0..10_000).map(|i| format!("{} {}", word(j), word(i))).collect(),
            one_cell: false,
        })
        .collect();
    let p = Progress::new();
    let d = Document::from_bytes(
        b"v,w\n1,Smith\n2,Jones\n,\n,\n,\n".to_vec(),
        OpenOptions::default(),
        Some("csv"),
    )
    .unwrap();
    let fill = |col: u32, rows: u64, count: u64| FillSpec {
        row: 1,
        col,
        rows,
        cols: 1,
        dir: Dir::Down,
        count,
        ..Default::default()
    };
    let cases = [
        ("2-cell number drag", fill(0, 2, 3)),
        ("1M-row number fill (block of 2 cells)", fill(0, 2, 1_000_000)),
        ("one-cell text drag matching nothing", fill(1, 1, 3)),
        ("2-cell text drag matching nothing", fill(1, 2, 3)),
    ];
    for (name, s) in cases {
        let mut used = 0;
        let ms = time(runs, || used = d.fill_orders_used(&s, &saved, &p).unwrap().len());
        println!("{name}\t{ms:.3} ms added\t({used} orders used)");
    }
}
