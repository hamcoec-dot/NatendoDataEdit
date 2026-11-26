//! Times hashing a file: `cargo run --release --example hash_bench -- <file>`

use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("a file");
    let mut ms = Vec::new();
    let mut hash = String::new();
    for _ in 0..5 {
        let t = Instant::now();
        hash = natendo_engine::tools::file_sha256(
            std::path::Path::new(&path),
            &natendo_engine::Progress::new(),
        )
        .expect("hash");
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    ms.sort_by(f64::total_cmp);
    println!("{:.0} ms (median of 5) {hash}", ms[2]);
}
