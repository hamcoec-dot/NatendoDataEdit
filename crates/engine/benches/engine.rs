//! Regression benchmarks on a 1M-row in-memory fixture.
//! Run: `cargo bench -p natendo-engine`. For the 10M-row budget check see
//! `cargo run --release -p natendo-engine --example perf`.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use natendo_engine::index::RowIndex;
use natendo_engine::{Dialect, Document, OpenOptions};

fn fixture(rows: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(rows * 120);
    out.extend_from_slice(b"id,name,amount,note,city,code,flag,date\r\n");
    for i in 0..rows {
        out.extend_from_slice(
            format!(
                "{i},name {},{}.{:02},\"note, with comma {i}\",City {},C{:05},{},2026-{:02}-{:02}\r\n",
                i % 9973,
                i % 100_000,
                i % 100,
                i % 311,
                i % 99_991,
                i % 2 == 0,
                i % 12 + 1,
                i % 28 + 1
            )
            .as_bytes(),
        );
    }
    out
}

fn benches(c: &mut Criterion) {
    let data = fixture(1_000_000);
    let mut g = c.benchmark_group("engine");
    g.sample_size(10);
    g.throughput(Throughput::Bytes(data.len() as u64));
    g.bench_function("index 1M rows", |b| {
        b.iter(|| RowIndex::build(&data, Dialect::CSV, 0).unwrap().rows())
    });
    g.finish();

    let doc = Document::from_bytes(data, OpenOptions::default(), Some("csv")).unwrap();
    let mut row = 0u64;
    c.bench_function("viewport 60x8", |b| {
        b.iter(|| {
            row = (row + 7919 * 61) % (doc.row_count() - 60);
            doc.viewport(row..row + 60, 0..8, 4096).unwrap()
        })
    });
    c.bench_function("write 1M rows unedited", |b| {
        let mut out = Vec::with_capacity(doc.row_count() as usize * 120);
        b.iter(|| {
            out.clear();
            doc.write_to(&mut out, doc.format(), false).unwrap()
        })
    });
}

criterion_group!(engine, benches);
criterion_main!(engine);
