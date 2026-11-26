//! Value canonicalizer (C6) speed: on a 1,000,000-row column of about
//! 200,000 different street names, counting the values and grouping them by
//! sound (as main does), applying 10,000 merges with "change values from a
//! list", and (not `plain`) grouping by Abbreviations and listing the values a
//! 10,000-pair dictionary does not know.
//! `cargo run --release --example canon_bench -- <file.csv> [plain]`
//! (`plain`: only what main can do too.) The file is made (made-up data) when
//! it does not exist.
use natendo_engine::document::Progress;
use natendo_engine::editing::{LookAlikeSpec, MapValues, TallySpec};
use natendo_engine::{Document, OpenOptions};
use std::io::Write;
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

const NAMES: [&str; 8] = [
    "Main", "Oak", "Pine", "Maple", "Cedar", "Elm", "Lake", "Hill",
];

fn street(i: u32) -> String {
    let suffix = ["St", "Street", "Ave", "Avenue", "Rd"][(i / 200_000 % 5) as usize];
    format!("{} {} {suffix}", NAMES[(i % 8) as usize], i % 40_000)
}

fn make(path: &str) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    writeln!(f, "street,n").unwrap();
    for i in 0..1_000_000u32 {
        writeln!(f, "{},{}", street(i), i % 97).unwrap();
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("a CSV file");
    let plain = std::env::args().nth(2).is_some_and(|a| a == "plain");
    if !std::path::Path::new(&path).exists() {
        make(&path);
    }
    let d = Document::open_indexed(path.as_ref(), OpenOptions::default(), None).unwrap();
    let time = |name: &str, f: &dyn Fn() -> String| {
        let mut runs = Vec::new();
        let mut what = String::new();
        for _ in 0..5 {
            let t = Instant::now();
            what = f();
            runs.push(ms(t));
        }
        println!("{name}: median {:.1} ms ({what})", median(runs));
    };
    time("count values", &|| {
        let t = d
            .value_tally(
                &TallySpec {
                    col: 0,
                    keep_first: true,
                    limit: None,
                },
                &Progress::new(),
            )
            .unwrap();
        format!("{} values", t.values.len())
    });
    let spec: LookAlikeSpec =
        serde_json::from_str(r#"{"col":0,"keep_first":true,"method":"sounds"}"#).unwrap();
    time("group by sound", &|| {
        let g = d.lookalike_groups(&spec, &Progress::new()).unwrap();
        format!("{} groups", g.groups.len())
    });
    let pairs: Vec<(String, String)> = (0..10_000u32)
        .map(|k| {
            (
                street(k),
                format!("{} {} Street", NAMES[(k % 8) as usize], k % 40_000),
            )
        })
        .collect();
    let map = MapValues {
        cols: vec![0],
        pairs: pairs.clone(),
        match_case: false,
        trim: true,
        keep_first: true,
    };
    time("10,000 merges (change values from a list)", &|| {
        let p = d.plan_map_values(&map, &Progress::new()).unwrap();
        format!("{} cells", p.cells)
    });
    if !plain {
        let spec: LookAlikeSpec =
            serde_json::from_str(r#"{"col":0,"keep_first":true,"method":"abbreviations"}"#)
                .unwrap();
        time("group by abbreviations", &|| {
            let g = d.lookalike_groups(&spec, &Progress::new()).unwrap();
            format!("{} groups of {} values", g.groups.len(), g.values)
        });
        // The gate: 200,000 different values grouped by Abbreviations (under 3 s).
        let many: Vec<natendo_engine::editing::Tally> = (0..200_000u32)
            .map(|i| natendo_engine::editing::Tally {
                value: street(i),
                count: 1 + u64::from(i % 7),
            })
            .collect();
        time("group 200,000 different values by abbreviations", &|| {
            let g = natendo_engine::lookalike::group_lookalikes(
                &many,
                natendo_engine::lookalike::Method::Abbreviations,
                &Progress::new(),
            )
            .unwrap();
            format!("{} groups", g.groups.len())
        });
        let dict = natendo_engine::dictionary::Dictionary {
            name: "bench".into(),
            match_case: false,
            trim: true,
            pairs: pairs.into_iter().filter(|(a, b)| a != b).collect(),
        };
        let dict = dict.checked().unwrap().dict;
        time("values not in a 10,000-pair dictionary", &|| {
            let t = d
                .value_tally(
                    &TallySpec {
                        col: 0,
                        keep_first: true,
                        limit: None,
                    },
                    &Progress::new(),
                )
                .unwrap();
            format!("{} values", dict.unmatched(&t.values).len())
        });
    }
}
