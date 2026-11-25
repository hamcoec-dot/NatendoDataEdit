//! Times for Change encoding (B2), for the speed gate:
//!
//! ```text
//! cargo run --release -p natendo-engine --example encoding_bench -- [rows] [repeats]
//! ```
//! The raw-byte scan over a mixed file (target at least 200 MB/s), Apply in place (target under 200 ms), the scrub
//! preview of the first rows (target under 50 ms), each the best of `repeats`.

use std::io::{BufWriter, Write};
use std::time::Instant;

use natendo_engine::encoding::Encoding;
use natendo_engine::{Document, OpenOptions, Progress};

fn best(name: &str, repeats: usize, mut f: impl FnMut() -> u64) {
    let mut times = Vec::new();
    let mut n = 0;
    for _ in 0..repeats {
        let t = Instant::now();
        n = f();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let b = times.iter().cloned().fold(f64::MAX, f64::min);
    let all: Vec<String> = times.iter().map(|t| format!("{t:.1}")).collect();
    println!("{name}: best {b:.1} ms (result {n}) [{}]", all.join(", "));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rows: u64 = args
        .first()
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);
    let repeats: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(5);
    let dir = std::env::temp_dir().join("nde-encoding-bench");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("enc-{rows}.csv"));
    if !path.exists() {
        let mut w = BufWriter::with_capacity(1 << 20, std::fs::File::create(&path).unwrap());
        writeln!(w, "id,name,city,notes,amount\r").unwrap();
        for i in 0..rows {
            // Mostly ASCII; every 20th row valid UTF-8 with accents; every 50th a Windows-1252 byte; every 90th double-encoded.
            match i % 90 {
                0 => write!(w, "{i},Ren\u{e9}e,Li\u{e8}ge,note {i},{}\r\n", i % 1000).unwrap(),
                1 => {
                    w.write_all(format!("{i},Jos").as_bytes()).unwrap();
                    w.write_all(b"\xe9").unwrap();
                    w.write_all(format!(",Lyon,note {i},{}\r\n", i % 1000).as_bytes())
                        .unwrap();
                }
                2 => write!(w, "{i},Ren\u{c3}\u{a9}e,Nice,note {i},{}\r\n", i % 1000).unwrap(),
                _ => write!(
                    w,
                    "{i},name {},City {},some longer note text for row {i},{}.5\r\n",
                    i % 9973,
                    i % 311,
                    i % 1000
                )
                .unwrap(),
            }
        }
        w.flush().unwrap();
    }
    let bytes = std::fs::metadata(&path).unwrap().len();
    println!("file: {:.0} MB, {rows} rows", bytes as f64 / 1e6);
    let opts = OpenOptions {
        encoding: Some(Encoding::Utf8),
        ..Default::default()
    };
    let mut d = Document::open_indexed(&path, opts, None).unwrap();
    let p = Progress::new();
    let t = Instant::now();
    let mut mb_s = 0.0f64;
    for _ in 0..repeats {
        let t0 = Instant::now();
        let s = d.scan_encodings(true, &p).unwrap();
        mb_s = mb_s.max(bytes as f64 / 1e6 / t0.elapsed().as_secs_f64());
        assert!(s.invalid > 0 && s.mixed);
    }
    let _ = t;
    println!("scan: best {mb_s:.0} MB/s over the whole file");
    let latin = Encoding::from_label("windows-1252").unwrap();
    best(
        "Apply in place (UTF-8 to Windows-1252 and back)",
        repeats,
        || {
            d.set_encoding(latin).unwrap();
            d.set_encoding(Encoding::Utf8).unwrap();
            d.undo();
            d.undo();
            0
        },
    );
    let head: Vec<u8> = std::fs::read(&path)
        .unwrap()
        .into_iter()
        .take(512 * 1024)
        .collect();
    best(
        "scrub preview (8 rows of the first 512 KB)",
        repeats,
        || d.preview_decoded(&head, latin, 8).unwrap().len() as u64,
    );
}
