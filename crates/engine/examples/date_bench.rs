//! Reading dates, sorting by them and the date radar on 1,000,000 rows:
//! `cargo run --release --example date_bench -- <work folder>`. The part
//! before "radar" also runs on the build without the radar, to compare.
use natendo_engine::sort::{SortKey, SortKind, SortSpec};
use natendo_engine::value::{parse_date, ReadOptions};
use natendo_engine::{Document, OpenOptions, Progress};
use std::io::Write;
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("a work folder"));
    std::fs::create_dir_all(&dir).unwrap();
    let value = |r: u32| {
        let (y, m, d) = (1950 + r % 70, 1 + r % 12, 1 + r % 28);
        match r % 7 {
            0 => format!("{y}-{m:02}-{d:02}"),
            1 => format!("{m:02}/{d:02}/{y}"),
            2 => format!("{d}/{m}/{y}"),
            3 => format!("{m}/{d}/{} 14:05", y % 100),
            4 => format!("Jan {d} {y}"),
            5 => format!("{y}{m:02}{d:02}"),
            _ => "unknown".to_string(),
        }
    };
    let path = dir.join("dates-1m.csv");
    if !path.exists() {
        let mut w = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        writeln!(w, "when,n").unwrap();
        for r in 0..1_000_000u32 {
            writeln!(w, "{},{r}", value(r)).unwrap();
        }
    }
    let values: Vec<String> = (0..1_000_000).map(value).collect();
    let mut read = f64::MAX;
    let mut sorted = f64::MAX;
    for _ in 0..5 {
        let t = Instant::now();
        let n = values
            .iter()
            .filter(|v| parse_date(v, ReadOptions::default()).is_some())
            .count();
        read = read.min(ms(t));
        assert!(n > 500_000);
        let mut d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
        let spec = SortSpec {
            keys: vec![SortKey {
                col: 0,
                descending: false,
                kind: SortKind::Date,
                match_case: false,
            }],
            keep_first: true,
            ..Default::default()
        };
        let t = Instant::now();
        d.sort(&spec).unwrap();
        sorted = sorted.min(ms(t));
    }
    println!("best of 5: read 1M dates {read:.1} ms; sort 1M rows by date {sorted:.1} ms");
    // radar
    let d = Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
    let mut sample = f64::MAX;
    let mut full = f64::MAX;
    for _ in 0..5 {
        let t = Instant::now();
        d.date_scan(0, true, Some(200_000), &Progress::new())
            .unwrap();
        sample = sample.min(ms(t));
        let t = Instant::now();
        d.date_scan(0, true, None, &Progress::new()).unwrap();
        full = full.min(ms(t));
    }
    let t = Instant::now();
    let marks = d.date_marks(0, true, 500_000, 60).unwrap();
    let screen = ms(t);
    println!(
        "radar: first 200,000 rows {sample:.1} ms; all rows {full:.1} ms; one screen of marks {screen:.3} ms ({})",
        marks.len()
    );
}
