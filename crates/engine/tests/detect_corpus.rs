//! Accuracy of layout detection over hand-made awkward files
//! (`tests/detect-corpus`, written by its `make_corpus.py`) and over
//! generated tables in many layouts.
//!
//! A guess counts as right when it reads the file into exactly the same
//! rows and cells, with the same lines set aside above the table, as the
//! expected layout. (Two dialects that split a file identically, such as
//! "double quote" and "no quotes" on a file without quotes, are equally
//! right.)

use std::path::Path;

use natendo_engine::detect::LOW_CONFIDENCE;
use natendo_engine::{Dialect, Document, OpenOptions};

fn read_all(d: &Document) -> (u32, String, Vec<Vec<String>>) {
    let rows = (0..d.row_count())
        .map(|r| d.row_values(r).unwrap())
        .collect();
    (d.preamble_lines(), d.preamble_text(), rows)
}

/// `None` when detection read the file as expected, else a description.
fn check(name: &str, bytes: &[u8], want: Dialect, skip: u32) -> Option<String> {
    let ext = Path::new(name).extension().and_then(|e| e.to_str());
    let got = Document::from_bytes(bytes.to_vec(), OpenOptions::default(), ext).unwrap();
    let expected = OpenOptions {
        dialect: Some(want),
        skip_lines: Some(skip),
        ..Default::default()
    };
    let exp = Document::from_bytes(bytes.to_vec(), expected, ext).unwrap();
    if read_all(&got) == read_all(&exp) {
        return None;
    }
    let f = got.format();
    Some(format!(
        "{name}: got sep {:?} quote {:?} escape {:?} merge {} skip {}; want sep {:?} quote {:?} skip {}",
        f.dialect.delimiter_str(),
        f.dialect.quote().map(char::from),
        f.dialect.escape().map(char::from),
        f.dialect.merges(),
        got.preamble_lines(),
        want.delimiter_str(),
        want.quote().map(char::from),
        skip,
    ))
}

fn sep_bytes(name: &str) -> (Vec<u8>, bool) {
    match name {
        "comma" => (b",".to_vec(), false),
        "tab" => (b"\t".to_vec(), false),
        "semicolon" => (b";".to_vec(), false),
        "pipe" => (b"|".to_vec(), false),
        "caret" => (b"^".to_vec(), false),
        "colon" => (b":".to_vec(), false),
        "space" => (b" ".to_vec(), false),
        "spaces" => (b" ".to_vec(), true),
        "us" => (vec![0x1F], false),
        "soh" => (vec![0x01], false),
        other => (other.as_bytes().to_vec(), false),
    }
}

#[test]
fn hand_made_corpus() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/detect-corpus");
    let manifest = std::fs::read_to_string(dir.join("expected.txt")).unwrap();
    let mut failures = Vec::new();
    let mut total = 0;
    for line in manifest
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let mut parts = line.split(' ');
        let name = parts.next().unwrap();
        let field = |k: &str| {
            line.split(' ')
                .find_map(|p| p.strip_prefix(&format!("{k}=")).map(str::to_owned))
                .unwrap()
        };
        let sure = field("sure");
        let bytes = std::fs::read(dir.join(name)).unwrap();
        let ext = Path::new(name).extension().and_then(|e| e.to_str());
        let opened = Document::from_bytes(bytes.clone(), OpenOptions::default(), ext).unwrap();
        let confidence = opened.detection().unwrap().confidence;
        total += 1;
        match sure.as_str() {
            "no" if confidence >= LOW_CONFIDENCE => {
                failures.push(format!("{name}: confidence {confidence:.2}, should warn"))
            }
            "yes" if confidence < LOW_CONFIDENCE => failures.push(format!(
                "{name}: confidence {confidence:.2}, should be sure"
            )),
            _ => {}
        }
        if field("sep") == "any" {
            continue;
        }
        let (sep, merge) = sep_bytes(&field("sep"));
        let quote = match field("quote").as_str() {
            "none" => None,
            q => Some(q.as_bytes()[0]),
        };
        let escape = (field("escape") == "backslash").then_some(b'\\');
        let skip: u32 = field("skip").parse().unwrap();
        let want = Dialect::custom(&sep, quote, escape, merge).unwrap();
        if let Some(f) = check(name, &bytes, want, skip) {
            failures.push(f);
        }
    }
    println!(
        "hand-made corpus: {}/{} right",
        total - failures.len(),
        total
    );
    assert!(total >= 48);
    assert!(
        failures.is_empty(),
        "wrong guesses:\n{}",
        failures.join("\n")
    );
}

/// Small deterministic random numbers (no extra dependency).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len())]
    }
}

const WORDS: &[&str] = &[
    "amber", "basalt", "cedar", "delta", "ember", "fjord", "garnet", "harbor", "indigo", "juniper",
    "kestrel", "lagoon", "meadow", "nimbus", "orchid", "pebble", "quartz", "russet", "sierra",
    "thistle", "umber", "violet", "willow", "yarrow", "zephyr",
];

/// A cell of the given column kind.
fn cell(rng: &mut Rng, kind: usize, decimal_comma: bool) -> String {
    match kind {
        0 => (rng.below(100_000)).to_string(),
        1 => {
            let v = format!("{}.{:02}", rng.below(5000), rng.below(100));
            if decimal_comma {
                v.replace('.', ",")
            } else {
                v
            }
        }
        2 => format!(
            "20{:02}-{:02}-{:02}",
            rng.below(30),
            1 + rng.below(12),
            1 + rng.below(28)
        ),
        3 => rng.pick(WORDS).to_string(),
        4 => format!("{} {}", rng.pick(WORDS), rng.pick(WORDS)),
        5 => format!("{}, {}", rng.pick(WORDS), rng.pick(WORDS)),
        _ => if rng.below(2) == 0 { "yes" } else { "no" }.to_string(),
    }
}

#[test]
fn generated_tables() {
    let seps: &[(&[u8], &str)] = &[
        (b",", "comma"),
        (b"\t", "tab"),
        (b";", "semicolon"),
        (b"|", "pipe"),
        (b"^", "caret"),
        (b"\x1f", "unit separator"),
        (b"||", "double pipe"),
        (b"~|~", "tilde pipe"),
        (b" ", "lined-up spaces"),
    ];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut failures = Vec::new();
    let mut confident_wrong = 0;
    let runs = 600;
    for run in 0..runs {
        let &(sep, _) = rng.pick(seps);
        let merge = sep == b" ";
        let quote_all = !merge && rng.below(4) == 0;
        // Mostly double quotes, sometimes single ones.
        let q = if rng.below(7) == 0 { b'\'' } else { b'"' };
        let header = rng.below(10) < 7;
        let crlf = rng.below(2) == 0;
        let cols = 2 + rng.below(8);
        let rows = 3 + rng.below(40);
        let decimal_comma = sep == b";" && rng.below(2) == 0;
        let kinds: Vec<usize> = (0..cols).map(|_| rng.below(7)).collect();
        let d = Dialect::custom(sep, Some(q), None, merge).unwrap();
        let qc = q as char;
        let pad = 2 + rng.below(4);
        let brk = if crlf { "\r\n" } else { "\n" };
        let mut text = String::new();
        // (Not above lined-up columns: a title of as many words as there
        // are columns reads as a row there; a known limit.)
        let title = !merge && rng.below(5) == 0;
        if title {
            text.push_str(&format!("Report on {}{brk}{brk}", rng.pick(WORDS)));
        }
        let write_row = |text: &mut String, cells: Vec<String>| {
            for (i, v) in cells.iter().enumerate() {
                let needs = v.bytes().any(|b| sep.contains(&b) || b == q);
                let cell = if quote_all || needs {
                    format!("{qc}{}{qc}", v.replace(qc, &format!("{qc}{qc}")))
                } else {
                    v.clone()
                };
                if merge {
                    // Lined up: each column padded to a fixed width.
                    text.push_str(&format!("{cell:<w$}", w = 16 + pad));
                } else {
                    if i > 0 {
                        text.push_str(d.delimiter_str());
                    }
                    text.push_str(&cell);
                }
            }
            text.push_str(brk);
        };
        if header {
            write_row(&mut text, (0..cols).map(|c| format!("col_{c}")).collect());
        }
        for _ in 0..rows {
            let cells = kinds
                .iter()
                .map(|&k| cell(&mut rng, k, decimal_comma))
                .collect();
            write_row(&mut text, cells);
        }
        let skip = if title { 2 } else { 0 };
        if let Some(f) = check("gen.txt", text.as_bytes(), d, skip) {
            let opened =
                Document::from_bytes(text.clone().into_bytes(), OpenOptions::default(), None)
                    .unwrap();
            if opened.detection().unwrap().confidence >= LOW_CONFIDENCE {
                confident_wrong += 1;
            }
            failures.push(format!(
                "run {run}: {f}\n{}",
                text.lines().take(4).collect::<Vec<_>>().join("\n")
            ));
        }
    }
    let right = runs - failures.len();
    println!("generated tables: {right}/{runs} right");
    for f in failures.iter().take(10) {
        println!("{f}\n");
    }
    // At least 99%, and a wrong reading never without the warning.
    assert!(right * 100 >= runs * 99, "{right}/{runs} right");
    assert_eq!(confident_wrong, 0, "wrong readings shown as sure");
}
