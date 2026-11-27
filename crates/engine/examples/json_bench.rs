//! JSON save-back (C11) speed and memory: `gen N file`, then
//! `open file` (as the app opens JSON: shape kept), `save file out` (save back vs JSON Lines export),
//! `reads file out` (JSON Lines and CSV export and a sort of a tab opened from JSON, the C11 review gate).
//! Build the same file on main with `gen` and compare `reads` side by side (on main `reads` opens with
//! `import::import`, the only way it has).
//! `cargo run --release -p natendo-engine --example json_bench -- gen 1000000 m1.jsonl`
use natendo_engine::export::{ExportFormat, ExportOptions};
use natendo_engine::import;
use natendo_engine::sort::{SortKey, SortSpec};
use natendo_engine::Progress;
use std::io::Write;
use std::time::Instant;

fn open(path: &str) -> natendo_engine::Document {
    let data = std::fs::read(path).unwrap();
    let t0 = Instant::now();
    let (t, shape) = import::import_json_shaped(&data, &Progress::new()).unwrap();
    let bytes = shape.bytes();
    let mut d = t.into_document().unwrap();
    d.set_json_shape(Some(shape));
    eprintln!(
        "open {:.3} s, {} rows, shape {} bytes ({:.3}% of the file)",
        t0.elapsed().as_secs_f64(),
        d.total_rows(),
        bytes,
        bytes as f64 * 100.0 / data.len() as f64
    );
    d
}

fn best(runs: usize, mut f: impl FnMut() -> f64) -> f64 {
    let mut v: Vec<f64> = (0..runs).map(|_| f()).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a[1].as_str() {
        "gen" => {
            let n: u64 = a[2].parse().unwrap();
            let mut w = std::io::BufWriter::new(std::fs::File::create(&a[3]).unwrap());
            for i in 0..n {
                // 5% of rows are odd: a text amount, or a null / missing note.
                let amount = if i % 40 == 7 {
                    "\"n/a\"".to_string()
                } else {
                    format!("{}.{:02}", i % 1000, i % 100)
                };
                let note = match i % 40 {
                    3 => ",\"note\":null".to_string(),
                    5 => String::new(),
                    _ => format!(",\"note\":\"call {}\"", i % 17),
                };
                writeln!(
                    w,
                    "{{\"id\":{},\"name\":\"Person {}\",\"amount\":{},\"active\":{},\"tags\":[{},\"x\"]{}}}",
                    1_000_000_000_000u64 + i,
                    i,
                    amount,
                    i % 3 == 0,
                    i % 9,
                    note
                )
                .unwrap();
            }
        }
        "gen2" => {
            // Wider records: numbers (some with trailing zeros), nulls, nested lists of numbers.
            let n: u64 = a[2].parse().unwrap();
            let mut w = std::io::BufWriter::new(std::fs::File::create(&a[3]).unwrap());
            for i in 0..n {
                let maybe = |k: u64, v: String| {
                    if (i + k) % 7 == 0 {
                        "null".to_string()
                    } else {
                        v
                    }
                };
                writeln!(
                    w,
                    "{{\"id\":{},\"score\":{},\"ratio\":{},\"count\":{},\"city\":\"Town {}\",\"zip\":\"{:05}\",\"vals\":[{},{}.5,{}],\"flag\":{},\"when\":\"2024-{:02}-{:02}\",\"extra\":{}}}",
                    i,
                    maybe(1, format!("{}.{:02}", i % 500, i % 100)),
                    maybe(2, format!("0.{:04}", i % 10000)),
                    maybe(3, format!("{}", i % 97)),
                    i % 50,
                    i % 99999,
                    i % 10,
                    i % 7,
                    i % 1000,
                    i % 2 == 0,
                    1 + i % 12,
                    1 + i % 28,
                    maybe(4, format!("[{},{}]", i % 3, i % 5))
                )
                .unwrap();
            }
        }
        "open" => {
            let d = open(&a[2]);
            std::hint::black_box(&d);
        }
        "save" => {
            let mut d = open(&a[2]);
            let out = std::path::Path::new(&a[3]);
            let form = natendo_engine::jsonshape::JsonForm::from(&d.json_shape().unwrap().layout);
            let mut best_save = f64::MAX;
            let mut best_export = f64::MAX;
            for _ in 0..3 {
                let t0 = Instant::now();
                d.save_json(
                    out,
                    &form,
                    natendo_engine::jsonshape::Blanks::LeaveOut,
                    &Progress::new(),
                )
                .unwrap();
                best_save = best_save.min(t0.elapsed().as_secs_f64());
                let opts = ExportOptions {
                    format: ExportFormat::JsonLines,
                    header: true,
                    ..Default::default()
                };
                let t0 = Instant::now();
                d.export_file(&out.with_extension("export.jsonl"), &opts, &Progress::new())
                    .unwrap();
                best_export = best_export.min(t0.elapsed().as_secs_f64());
            }
            eprintln!(
                "save_json {best_save:.3} s, export JSON Lines {best_export:.3} s, ratio {:.3}",
                best_save / best_export
            );
            let same = std::fs::read(out).unwrap() == std::fs::read(&a[2]).unwrap();
            eprintln!("saved file byte-identical to the source: {same}");
        }
        "reads" => {
            let mut d = open(&a[2]);
            let out = std::path::Path::new(&a[3]);
            let opts = ExportOptions {
                format: ExportFormat::JsonLines,
                header: true,
                ..Default::default()
            };
            let ms = best(5, || {
                let t0 = Instant::now();
                d.export_file(out, &opts, &Progress::new()).unwrap();
                t0.elapsed().as_secs_f64() * 1000.0
            });
            eprintln!("export JSON Lines: median {ms:.1} ms");
            let ms = best(5, || {
                let t0 = Instant::now();
                let mut w = std::io::BufWriter::new(std::fs::File::create(out).unwrap());
                d.write_copy(&mut w).unwrap();
                w.flush().unwrap();
                t0.elapsed().as_secs_f64() * 1000.0
            });
            eprintln!("export CSV (write_copy): median {ms:.1} ms");
            let spec = SortSpec {
                keys: vec![SortKey {
                    col: 1,
                    descending: true,
                    kind: Default::default(),
                    match_case: false,
                }],
                keep_first: true,
                ..Default::default()
            };
            let ms = best(5, || {
                let t0 = Instant::now();
                let p = d.plan_sort(&spec, &Progress::new()).unwrap();
                std::hint::black_box(&p);
                t0.elapsed().as_secs_f64() * 1000.0
            });
            eprintln!("sort plan (name, descending): median {ms:.1} ms");
            let _ = &mut d;
        }
        _ => panic!("gen | open | save | reads"),
    }
}
