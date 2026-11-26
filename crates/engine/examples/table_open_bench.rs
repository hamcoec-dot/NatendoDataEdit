//! Times opening a CSV file, or reading a table of a database:
//! `cargo run --release --example table_open_bench -- csv <file>`
//! `cargo run --release --example table_open_bench -- sqlite <file> <table>`

use natendo_engine::{Document, OpenOptions, Progress};
use std::path::Path;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = Path::new(&args[1]);
    let t = Instant::now();
    let rows = match args[0].as_str() {
        "csv" => Document::open_indexed(path, OpenOptions::default(), None)
            .expect("open")
            .row_count(),
        _ => natendo_engine::sqlite::read_table(path, &args[2], &Progress::new())
            .expect("read")
            .row_count(),
    };
    println!("{:.1} ms, {rows} rows", t.elapsed().as_secs_f64() * 1000.0);
}
