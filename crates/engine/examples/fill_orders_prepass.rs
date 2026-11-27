//! Times the step the app adds before an automatic fill: finding which of
//! the user's saved custom orders it follows (`Document::fill_orders_used`),
//! with the worst saved set (60 orders of 10,000 made-up values, none ticked
//! for one cell), on text that matches no order. Also the one-off build of
//! the lookup, which the app does in the background when the orders change.
//! `cargo run --release --example fill_orders_prepass -- [runs]`

use natendo_engine::fill::{FillOrder, OrderBook};
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
    let runs: usize = std::env::args()
        .nth(1)
        .map_or(15, |s| s.parse().expect("runs"));
    let saved: Vec<FillOrder> = (0..60)
        .map(|j| FillOrder {
            name: format!("Order {}", word(j)),
            items: (0..10_000)
                .map(|i| format!("{} {}", word(j), word(i)))
                .collect(),
            one_cell: false,
        })
        .collect();
    let build = time(5, || OrderBook::new(saved.clone()).prepare());
    let copy = time(5, || drop(OrderBook::new(saved.clone())));
    println!(
        "one-off lookup build (in the background)\t{:.1} ms",
        build - copy
    );
    let book = OrderBook::new(saved);
    book.prepare();
    // 200 columns of surnames, made up; then empty rows to fill.
    let cols = 200;
    let mut text = String::new();
    for r in 0..6 {
        let row: Vec<String> = (0..cols)
            .map(|c| {
                if r == 0 {
                    format!("t{}", word(c))
                } else if r <= 2 {
                    format!("Surname{}{}", word(c), word(r))
                } else {
                    String::new()
                }
            })
            .collect();
        text.push_str(&row.join(","));
        text.push('\n');
    }
    let d = Document::from_bytes(text.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
    let p = Progress::new();
    let fill = |rows: u64, ncols: u32| FillSpec {
        row: 1,
        col: 0,
        rows,
        cols: ncols,
        dir: Dir::Down,
        count: 3,
        ..Default::default()
    };
    let cases = [
        ("1 cell", fill(1, 1)),
        ("1 x 8 drag down", fill(1, 8)),
        ("1 x 64 drag down", fill(1, 64)),
        ("1 x 200 drag down", fill(1, 200)),
        ("2 x 64 drag down", fill(2, 64)),
    ];
    for (name, s) in cases {
        let mut used = 0;
        let ms = time(runs, || {
            used = d.fill_orders_used(&s, &book, &p).unwrap().len()
        });
        println!("{name}\t{ms:.3} ms added\t({used} orders used)");
    }
}
