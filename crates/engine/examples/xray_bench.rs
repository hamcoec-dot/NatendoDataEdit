//! Times of the X-ray report and the Sanitize step (B1), for the speed gate:
//!
//! ```text
//! cargo run --release -p natendo-engine --example xray_bench -- [rows] [repeats] [folder]
//! ```
//! The report over 1M rows x 20 columns (target under 4 s), and Sanitize on one 1M-row column against the
//! existing `Spaces::Plain` step on the same column (target within 1.2x), each the best of `repeats`.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

use natendo_engine::clean::{CleanSpec, Cleaner, Spaces, Step};
use natendo_engine::find::Scope;
use natendo_engine::xray::SanitizeOptions;
use natendo_engine::{Document, OpenOptions, Progress};

const COLS: usize = 20;

fn generate(path: &std::path::Path, rows: u64) -> std::io::Result<()> {
    let mut w = BufWriter::with_capacity(1 << 20, File::create(path)?);
    let header: Vec<String> = (0..COLS).map(|c| format!("col{c}")).collect();
    writeln!(w, "{}\r", header.join(","))?;
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in 0..rows {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        // One row in 50 has a look-alike letter, one in 70 a curly quote, one in 90 an odd space.
        let name = match i % 450 {
            0 | 50 | 100 => format!("Sm\u{430}rt {}", x % 9973),
            70 | 140 => format!("O\u{2019}Brien {}", x % 9973),
            90 => format!("Main\u{a0}St {}", x % 9973),
            _ => format!("name {}", x % 9973),
        };
        write!(
            w,
            "{i},\"{name}\",{}.{:02},City {}",
            x % 100_000,
            x % 100,
            x % 311
        )?;
        for c in 4..COLS {
            write!(w, ",v{}", (x >> (c * 2)) % 1_000_000)?;
        }
        w.write_all(b"\r\n")?;
    }
    w.flush()
}

fn best(name: &str, repeats: usize, mut f: impl FnMut() -> u64) {
    let mut times = Vec::new();
    let mut n = 0;
    for _ in 0..repeats {
        let t = Instant::now();
        n = f();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let b = times.iter().cloned().fold(f64::MAX, f64::min);
    let all: Vec<String> = times.iter().map(|t| format!("{t:.0}")).collect();
    println!("{name}: best {b:.0} ms (result {n}) [{}]", all.join(", "));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rows: u64 = args
        .first()
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);
    let repeats: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(3);
    let dir = args.get(2).map_or_else(
        || std::env::temp_dir().join("nde-xray-bench"),
        std::path::PathBuf::from,
    );
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("xray-{rows}.csv"));
    if !path.exists() {
        generate(&path, rows).unwrap();
    }
    let open = || Document::open_indexed(&path, OpenOptions::default(), None).unwrap();
    let p = Progress::new();
    let d = open();
    best("report, every column", repeats, || {
        d.xray_report(&[], true, &p).unwrap().cells
    });
    let step = |s: Step| CleanSpec {
        cleaner: Cleaner {
            version: 1,
            name: String::new(),
            steps: vec![s],
        },
        scope: Scope::Columns { cols: vec![1] },
        skip_first: true,
        read: Default::default(),
    };
    let run = |s: Step| {
        let mut d = open();
        let plan = d.plan_clean(&step(s), 0.0, 1, 0, &Progress::new()).unwrap();
        d.apply_clean(plan).unwrap();
        d.row_count()
    };
    best("sanitize, one column", repeats, || {
        run(Step::Sanitize {
            options: SanitizeOptions::usual(),
        })
    });
    best("Spaces::Plain, one column", repeats, || {
        run(Step::Spaces { how: Spaces::Plain })
    });
}
