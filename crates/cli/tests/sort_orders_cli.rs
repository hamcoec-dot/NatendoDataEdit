//! `natendo-cli sort --by` with custom orders: typed (`order=A|B|C`, with
//! the direction first so values may hold `:`), from a file (`order=@file`)
//! and ready-made (`:weekdays`). Made-up data.

use std::path::PathBuf;
use std::process::Command;

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        let d = std::env::temp_dir().join(format!("nd-cli-orders-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Folder(d)
    }
    fn write(&self, name: &str, text: &str) -> String {
        std::fs::write(self.0.join(name), text).unwrap();
        self.path(name)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().to_string()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Exit code, the sorted file's first column (titles left out), and error text.
fn sort(d: &Folder, input: &str, by: &str) -> (i32, Vec<String>, String) {
    let out = d.path("out.csv");
    let _ = std::fs::remove_file(&out);
    let o = Command::new(env!("CARGO_BIN_EXE_natendo-cli"))
        .args(["sort", input, "-o", &out, "--by", by])
        .output()
        .unwrap();
    let col = std::fs::read_to_string(&out)
        .map(|t| {
            t.lines()
                .skip(1)
                .map(|l| l.split(',').next().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default();
    (
        o.status.code().unwrap(),
        col,
        String::from_utf8_lossy(&o.stderr).to_string(),
    )
}

fn input(d: &Folder) -> String {
    d.write(
        "in.csv",
        "level,day,slot\r\nHigh,Wed,11:00\r\nzz,Mon,09:30\r\nLow,Fri,10:00\r\nMedium,Tue,12:15\r\nlow,Sun,09:30\r\n",
    )
}

#[test]
fn a_typed_order_ascending_and_descending_with_other_values_last() {
    let d = Folder::new("typed");
    let a = input(&d);
    let (code, col, err) = sort(&d, &a, "level:order=Low|Medium|High");
    assert_eq!(code, 0, "{err}");
    assert_eq!(col, ["Low", "low", "Medium", "High", "zz"]);
    let (code, col, _) = sort(&d, &a, "level:desc:order=Low|Medium|High");
    assert_eq!(code, 0);
    // Descending turns the whole order round: values not on it come first.
    assert_eq!(col, ["zz", "High", "Medium", "Low", "low"]);
}

#[test]
fn values_may_hold_a_colon_when_the_direction_comes_first() {
    let d = Folder::new("times");
    let a = d.write(
        "t.csv",
        "slot,who\r\n11:00,a\r\n09:30,b\r\n10:00,c\r\n12:15,d\r\n",
    );
    let (code, col, err) = sort(&d, &a, "slot:desc:order=10:00|11:00|09:30");
    assert_eq!(code, 0, "{err}");
    // Descending along the order (the value not on it first, as in the app).
    assert_eq!(col, ["12:15", "09:30", "11:00", "10:00"]);
    let (_, col, _) = sort(&d, &a, "slot:order=10:00|11:00|09:30");
    assert_eq!(col, ["10:00", "11:00", "09:30", "12:15"]);
}

#[test]
fn ready_made_orders_by_name() {
    let d = Folder::new("builtin");
    let a = input(&d);
    let (code, _, err) = sort(&d, &a, "day:weekdays-short");
    assert_eq!(code, 0, "{err}");
    let out = std::fs::read_to_string(d.path("out.csv")).unwrap();
    let days: Vec<&str> = out
        .lines()
        .skip(1)
        .map(|l| l.split(',').nth(1).unwrap())
        .collect();
    assert_eq!(days, ["Mon", "Tue", "Wed", "Fri", "Sun"]);
    let (code, col, _) = sort(&d, &a, "level:desc:low-medium-high");
    assert_eq!(code, 0);
    assert_eq!(col[..2], ["zz", "High"]);
}

#[test]
fn an_order_from_a_file_and_escaped_values() {
    let d = Folder::new("file");
    let a = d.write("p.csv", "pri,n\r\nB|x,1\r\nA,2\r\nC\\,y,3\r\n");
    let f = d.write("order.txt", "C\\,y\r\n\r\nB|x\r\nA\r\n");
    let (code, _, err) = sort(&d, &a, &format!("pri:order=@{f}"));
    assert_eq!(code, 0, "{err}");
    let out = std::fs::read_to_string(d.path("out.csv")).unwrap();
    let first: Vec<&str> = out
        .lines()
        .skip(1)
        .map(|l| l.rsplit(',').next().unwrap())
        .collect();
    // In the file, a line is a value as it is ("C\,y" is not a value in the table).
    assert_eq!(first, ["1", "2", "3"]);
    // Typed: \| is a | inside a value.
    let (code, _, err) = sort(&d, &a, "pri:order=C|B\\|x|A");
    assert_eq!(code, 0, "{err}");
    let out = std::fs::read_to_string(d.path("out.csv")).unwrap();
    let n: Vec<&str> = out
        .lines()
        .skip(1)
        .map(|l| l.rsplit(',').next().unwrap())
        .collect();
    assert_eq!(n, ["1", "2", "3"]);
}

#[test]
fn mistakes_are_errors() {
    let d = Folder::new("bad");
    let a = input(&d);
    let (code, _, err) = sort(&d, &a, "level:weekdayz");
    assert_eq!(code, 2);
    assert!(
        err.contains("weekdays-short") && err.contains("low-medium-high"),
        "{err}"
    );
    let (code, _, err) = sort(&d, &a, "level:order=");
    assert_eq!(code, 2);
    assert!(err.contains("empty"), "{err}");
    let (code, _, err) = sort(&d, &a, "level:months:order=A|B");
    assert_eq!(code, 2);
    assert!(err.contains("not both"), "{err}");
    let (code, _, err) = sort(&d, &a, &format!("level:order=@{}", d.path("missing.txt")));
    assert_eq!(code, 2);
    assert!(err.contains("cannot read"), "{err}");
}

#[test]
fn the_ready_made_orders_match_the_app() {
    // ui/src/lib/m10.ts keeps the app's copy of the ready-made orders.
    let m10 = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/src/lib/m10.ts"),
    )
    .unwrap();
    let start = m10.find("export const BUILTIN_LISTS").unwrap();
    let block = &m10[start..start + m10[start..].find("];").unwrap()];
    let ui: Vec<(String, Vec<String>)> = block
        .lines()
        .filter_map(|l| {
            let name = l.split("name: \"").nth(1)?.split('"').next()?.to_string();
            let items = l.split("items: [").nth(1)?.split(']').next()?;
            Some((
                name,
                items
                    .split(',')
                    .map(|i| i.trim().trim_matches('"').to_string())
                    .collect(),
            ))
        })
        .collect();
    let engine: Vec<(String, Vec<String>)> = natendo_engine::sort::BUILTIN_LISTS
        .iter()
        .map(|b| (b.1.to_string(), b.2.iter().map(|s| s.to_string()).collect()))
        .collect();
    assert_eq!(ui, engine);
}
