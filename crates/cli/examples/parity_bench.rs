//! The engine calls the CLI parity commands wrap, timed in one process (open
//! included), for the speed gate: `parity_bench <command> <file> [<other>]`
//! prints milliseconds. Made-up data only.

use std::path::Path;
use std::time::Instant;

use natendo_engine::diff::DiffSpec;
use natendo_engine::editing::LookAlikeSpec;
use natendo_engine::privacy::{ColumnAction, SafeCopy, Secret};
use natendo_engine::{Document, OpenOptions, Progress};

fn open(p: &str) -> Document {
    Document::open_indexed(Path::new(p), OpenOptions::default(), None).unwrap()
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, file) = (a[0].as_str(), a[1].as_str());
    let out = std::env::temp_dir().join(format!("nd-parity-bench-{}", std::process::id()));
    let p = Progress::new();
    let t = Instant::now();
    match cmd {
        "xray" => {
            open(file).xray_report(&[], true, &p).unwrap();
        }
        "private" => {
            open(file)
                .find_private(
                    true,
                    Default::default(),
                    natendo_engine::clean::now_utc(),
                    &p,
                )
                .unwrap();
        }
        "scan" => {
            open(file).scan_encodings(true, &p).unwrap();
        }
        "lookalikes" => {
            open(file)
                .lookalike_groups(
                    &LookAlikeSpec {
                        col: 3,
                        keep_first: true,
                        method: Default::default(),
                        extra: Vec::new(),
                    },
                    &p,
                )
                .unwrap();
        }
        "compare" => {
            let spec = DiffSpec {
                keys: vec![0],
                keep_first: true,
                match_case: false,
                trim: true,
            };
            open(file).diff_files(&open(&a[2]), &spec, &p).unwrap();
        }
        "share-safe" => {
            let mut scramble: natendo_engine::clean::Cleaner =
                serde_json::from_str(r#"{"steps":[{"tool":"scramble"}]}"#).unwrap();
            if let natendo_engine::clean::Step::Scramble { phrase, .. } = &mut scramble.steps[0] {
                *phrase = Secret::new("correct horse battery staple");
            }
            let mask: natendo_engine::clean::Cleaner =
                serde_json::from_str(r#"{"steps":[{"tool":"mask","keep_last":2}]}"#).unwrap();
            let spec = SafeCopy {
                columns: vec![
                    ColumnAction::Keep,
                    ColumnAction::Clean { cleaner: scramble },
                    ColumnAction::Drop,
                    ColumnAction::Keep,
                    ColumnAction::Clean { cleaner: mask },
                ],
                header: true,
                read: Default::default(),
                guard: true,
            };
            open(file)
                .save_safe_copy(&out, &spec, natendo_engine::clean::now_utc(), &p)
                .unwrap();
        }
        "encrypt" => {
            open(file)
                .save_encrypted(&out, &Secret::new("a long test password"), &p)
                .unwrap();
        }
        _ => panic!("unknown command {cmd}"),
    }
    println!("{}", t.elapsed().as_millis());
    let _ = std::fs::remove_file(&out);
}
