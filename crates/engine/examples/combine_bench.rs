//! Times Combine on given files:
//! `cargo run --release --example combine_bench -- <title|place> <file>...`

use natendo_engine::tools::{combine, Combine};
use natendo_engine::{Document, OpenOptions, Progress};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mode, files) = args.split_first().expect("mode and files");
    let docs: Vec<Document> = files
        .iter()
        .map(|f| {
            Document::open_indexed(std::path::Path::new(f), OpenOptions::default(), None)
                .expect("open")
        })
        .collect();
    let tables: Vec<(String, &Document)> = files.iter().cloned().zip(&docs).collect();
    let spec = Combine {
        header: true,
        by_title: mode == "title",
        source: true,
    };
    let t = Instant::now();
    let out = combine(&tables, &spec, &Progress::new()).expect("combine");
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    let rows = out.into_document().expect("table").row_count();
    println!("{ms:.1} ms, {rows} rows");
}
