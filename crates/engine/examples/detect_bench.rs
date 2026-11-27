//! Separator guess speed: the median time of `detect()` over 200 runs on
//! each of a set of made-up inputs. Prints `name median_us` lines and a
//! total. Usage: detect_bench [runs]

use natendo_engine::detect::{detect, Hints};
use std::time::Instant;

fn inputs() -> Vec<(&'static str, Vec<u8>)> {
    let mut big = b"id,first,last,ward,precinct,city,zip,joined\n".to_vec();
    for i in 0..20_000u32 {
        big.extend_from_slice(
            format!(
                "{i},Ann{},Quorrin{},W{},P{:03},Fendle,{:05},2024-0{}-1{}\n",
                i % 97,
                i % 89,
                i % 4 + 1,
                i % 120,
                37000 + i % 400,
                i % 9 + 1,
                i % 9
            )
            .as_bytes(),
        );
    }
    let mut wide = (0..60)
        .map(|c| format!("c{c}"))
        .collect::<Vec<_>>()
        .join("\t")
        + "\n";
    for r in 0..2_000 {
        wide += &((0..60)
            .map(|c| format!("{}", r * 60 + c))
            .collect::<Vec<_>>()
            .join("\t")
            + "\n");
    }
    let mut messy = String::from("key:/value||note;;x\n");
    for r in 0..3_000 {
        messy += &format!("k{r}:/v{r}||a,b;;c...d--e//f\n");
    }
    let mut template = String::from("x,y,z\n");
    template += &",,\n".repeat(5_000);
    let mut tabs = String::from("a\tb\tc\n");
    tabs += &"\t\t\n".repeat(5_000);
    let mut dcomma = String::from("id,,name,,ward\n");
    for i in 0..5_000 {
        dcomma += &format!("{i},,Ann Quorrin {i},,W{}\n,,,,\n", i % 4 + 1);
    }
    vec![
        ("csv 20k rows", big),
        ("tab 60 cols", wide.into_bytes()),
        ("messy punctuation", messy.into_bytes()),
        ("comma template", template.into_bytes()),
        ("tab template", tabs.into_bytes()),
        ("double comma, empty rows", dcomma.into_bytes()),
    ]
}

fn main() {
    let runs: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(200);
    let mut total = 0f64;
    for (name, data) in inputs() {
        let mut t: Vec<f64> = (0..runs)
            .map(|_| {
                let s = Instant::now();
                let d = detect(&data, Hints::default());
                std::hint::black_box(d);
                s.elapsed().as_secs_f64() * 1e6
            })
            .collect();
        t.sort_by(f64::total_cmp);
        let m = t[runs / 2];
        total += m;
        println!("{name}\t{m:.1}");
    }
    println!("total\t{total:.1}");
}
