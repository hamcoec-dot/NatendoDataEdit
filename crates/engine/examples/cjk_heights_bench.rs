//! Speed gates of gap #7 (CJK in PDF, row heights read from workbooks):
//!
//! ```text
//! cargo run --release -p natendo-engine --example cjk_heights_bench -- [pdf-latin|pdf-cjk|xlsx-open|xlsx-heights] [rows] [repeats] [folder]
//! ```
//! `pdf-latin` / `pdf-cjk`: a PDF of `rows` made-up rows (6 columns; Latin names, or Chinese, Japanese
//! and Korean ones), written to memory. `xlsx-open`: open (read every cell of) a workbook of `rows`
//! rows made by our own export, with no row heights; `xlsx-heights`: the same with 100,000 rows given
//! a height. Prints the best and every time in milliseconds, and the PDF or workbook size. Uses only
//! what main has too, so the same file times main.

use std::time::Instant;

use natendo_engine::export::{ExportFormat, ExportOptions};
use natendo_engine::import::{self, ImportFormat, ImportOptions};
use natendo_engine::{Document, OpenOptions, Progress};

fn rows_text(rows: u64, cjk: bool) -> String {
    let mut s = String::from("id,name,city,amount,date,note\r\n");
    let names = [
        "王小明",
        "やまだ たろう",
        "김민수",
        "李华",
        "さとう はなこ",
        "박지성",
    ];
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in 0..rows {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let name = if cjk {
            format!("{}{}", names[(x % 6) as usize], x % 97)
        } else {
            format!("Person {}", x % 9973)
        };
        s.push_str(&format!(
            "{i},{name},City {},{}.{:02},2026-{:02}-{:02},note {}\r\n",
            x % 311,
            x % 100_000,
            x % 100,
            x % 12 + 1,
            x % 28 + 1,
            x % 1000
        ));
    }
    s
}

fn doc(text: String) -> Document {
    Document::from_bytes(text.into_bytes(), OpenOptions::default(), Some("csv")).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map_or("pdf-latin", String::as_str).to_string();
    let rows: u64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(100_000);
    let repeats: usize = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(5);
    let folder = args
        .get(4)
        .cloned()
        .unwrap_or_else(|| std::env::temp_dir().display().to_string());
    let mut times = Vec::new();
    let mut size = 0usize;
    match mode.as_str() {
        "pdf-latin" | "pdf-cjk" => {
            let d = doc(rows_text(rows, mode == "pdf-cjk"));
            let opts = ExportOptions {
                format: ExportFormat::Pdf,
                ..Default::default()
            };
            for _ in 0..repeats {
                let mut out = Vec::with_capacity(64 << 20);
                let t = Instant::now();
                d.export(&mut out, &opts, None, &Progress::new()).unwrap();
                times.push(t.elapsed().as_secs_f64() * 1e3);
                size = out.len();
            }
        }
        "xlsx-open" | "xlsx-heights" => {
            let path = std::path::Path::new(&folder).join(format!("nde-{mode}-{rows}.xlsx"));
            if !path.exists() {
                let d = doc(rows_text(rows, false));
                let heights: Vec<(u64, u32)> = if mode == "xlsx-heights" {
                    (0..100_000u64.min(rows))
                        .map(|r| (r * (rows / 100_000).max(1), 20 + (r % 50) as u32))
                        .collect()
                } else {
                    Vec::new()
                };
                let mut out = Vec::new();
                d.export(
                    &mut out,
                    &ExportOptions {
                        format: ExportFormat::Xlsx,
                        row_heights: heights,
                        ..Default::default()
                    },
                    None,
                    &Progress::new(),
                )
                .unwrap();
                std::fs::write(&path, out).unwrap();
            }
            let data = std::fs::read(&path).unwrap();
            size = data.len();
            let opts = ImportOptions {
                format: Some(ImportFormat::Spreadsheet),
                ..Default::default()
            };
            for _ in 0..repeats {
                let t = Instant::now();
                let (table, _) =
                    import::import_report(&data, Some("xlsx"), &opts, None, &Progress::new())
                        .unwrap();
                times.push(t.elapsed().as_secs_f64() * 1e3);
                std::hint::black_box(table);
            }
        }
        m => panic!("unknown mode {m}"),
    }
    let best = times.iter().cloned().fold(f64::MAX, f64::min);
    let all: Vec<String> = times.iter().map(|t| format!("{t:.1}")).collect();
    println!(
        "{mode} rows={rows} best={best:.1} ms bytes={size} all=[{}]",
        all.join(", ")
    );
}
