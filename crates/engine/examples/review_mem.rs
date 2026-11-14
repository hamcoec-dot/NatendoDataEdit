//! Reviewer's memory probe: charged bytes vs real peak memory.
use natendo_engine::document::Progress;
use natendo_engine::pivot::*;
use natendo_engine::{Document, OpenOptions};

fn status(k: &str) -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap();
    s.lines()
        .find(|l| l.starts_with(k))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<u64>()
        .unwrap()
        * 1024
}
fn reset_peak() {
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}
fn mb(x: u64) -> String {
    format!("{:.0} MB", x as f64 / 1e6)
}

fn main() {
    let which = std::env::args().nth(1).unwrap_or_default();
    let rows: usize = std::env::args()
        .nth(2)
        .map(|s| s.parse().unwrap())
        .unwrap_or(2_000_000);
    let mut s = String::with_capacity(rows * 40);
    s.push_str("id,grp,cat,n,t\n");
    let mut x = 88172645463325252u64;
    for i in 0..rows {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.push_str(&format!(
            "{i},{},{},{}.{},{}\n",
            x % 7,
            (x >> 8) % 5,
            (x >> 16) % 100000,
            (x >> 40) % 100,
            x % 1_000_003
        ));
    }
    let d = Document::from_bytes(s.into_bytes(), OpenOptions::default(), Some("csv")).unwrap();
    let all: Vec<u32> = (0..d.col_count()).collect();
    let titles = d.titles(&all, true);
    let f = |col: u32| PivotField {
        col,
        ..Default::default()
    };
    let v = |col: u32, agg: Agg| ValueField {
        col,
        agg,
        ..Default::default()
    };
    let spec = match which.as_str() {
        // many leaves, many light values
        "leaves" => PivotSpec {
            rows: vec![f(0)],
            cols: vec![f(2)],
            values: [
                Agg::Sum,
                Agg::Count,
                Agg::Average,
                Agg::Min,
                Agg::Max,
                Agg::StdDev,
                Agg::CountRows,
                Agg::Product,
            ]
            .iter()
            .map(|&a| v(3, a))
            .collect(),
            ..Default::default()
        },
        // heavy: median + distinct count on few groups
        "heavy" => PivotSpec {
            rows: vec![f(1)],
            cols: vec![f(2)],
            values: vec![v(3, Agg::Median), v(4, Agg::CountDistinct)],
            ..Default::default()
        },
        // heavy with many leaves
        "heavyleaves" => PivotSpec {
            rows: vec![f(1), f(0)],
            cols: vec![f(2)],
            values: vec![v(3, Agg::Median), v(4, Agg::CountDistinct), v(3, Agg::Sum)],
            ..Default::default()
        },
        _ => panic!("leaves|heavy|heavyleaves"),
    };
    let base = status("VmRSS:");
    reset_peak();
    let t = std::time::Instant::now();
    let cube = d.pivot_cube(&spec, None, &Progress::new()).unwrap();
    let peak_cube = status("VmHWM:");
    let after_cube = status("VmRSS:");
    println!(
        "{which} rows={rows} cube: {:.2}s charged {} | real held {} | peak during build {}",
        t.elapsed().as_secs_f64(),
        mb(cube.bytes as u64),
        mb(after_cube - base),
        mb(peak_cube - base)
    );
    reset_peak();
    let t = std::time::Instant::now();
    let g = cube
        .layout(
            &spec,
            &titles,
            Some(GridLimit {
                rows: 2000,
                cols: 400,
            }),
        )
        .unwrap();
    let peak_l = status("VmHWM:");
    let after_l = status("VmRSS:");
    println!(
        "  layout(preview): {:.2}s rows {} | extra held after (memo) {} | peak extra {}",
        t.elapsed().as_secs_f64(),
        g.total_rows,
        mb(after_l.saturating_sub(after_cube)),
        mb(peak_l.saturating_sub(after_cube))
    );
    // second layout = memo reuse
    let t = std::time::Instant::now();
    let _ = cube
        .layout(
            &spec,
            &titles,
            Some(GridLimit {
                rows: 2000,
                cols: 400,
            }),
        )
        .unwrap();
    println!("  re-layout: {:.2}s", t.elapsed().as_secs_f64());
}
