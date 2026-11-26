//! Times reading a `.dbf` as the import does:
//! `cargo run --release --example dbf_bench -- <file> [cancel-after-ms]`

use natendo_engine::dbf::Sidecars;
use natendo_engine::import::{import_report, ImportOptions};
use natendo_engine::Progress;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cancel: Option<u64> = args.get(1).map(|a| a.parse().expect("ms"));
    let progress = Progress::new();
    let t = Instant::now();
    let data = std::fs::read(&args[0]).expect("read");
    let opts = ImportOptions {
        beside: Some(std::sync::Arc::new(Sidecars::beside(Path::new(&args[0])))),
        ..Default::default()
    };
    std::thread::scope(|s| {
        if let Some(ms) = cancel {
            let progress = &progress;
            s.spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(ms));
                progress.cancel.store(true, Ordering::Relaxed);
            });
        }
        let r = import_report(&data, Some("dbf"), &opts, None, &progress)
            .and_then(|(t, _)| t.into_document());
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        match r {
            Ok(d) => println!("{ms:.1} ms, {} rows", d.row_count()),
            Err(e) => println!("{ms:.1} ms, {e}"),
        }
    });
}
