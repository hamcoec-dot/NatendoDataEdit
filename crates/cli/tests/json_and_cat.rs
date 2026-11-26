//! The built program: every command answers `--json` with one JSON value, and
//! `cat --by-name` keeps every column.

use std::path::{Path, PathBuf};
use std::process::Command;

use natendo_engine::tools::{combine, Combine};
use natendo_engine::{Document, OpenOptions, Progress};
use serde_json::Value;

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        let d = std::env::temp_dir().join(format!("nd-cli-{name}-{}", std::process::id()));
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

fn run(args: &[&str]) -> (i32, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_natendo-cli"))
        .args(args)
        // No running app is found, so `app` fails the same way everywhere.
        .env("APPDATA", std::env::temp_dir().join("nd-cli-no-app"))
        .output()
        .unwrap();
    (
        o.status.code().unwrap(),
        String::from_utf8(o.stdout).unwrap(),
    )
}

/// All of standard output as one JSON value.
fn json(args: &[&str]) -> (i32, Value) {
    let mut a = args.to_vec();
    a.push("--json");
    let (code, out) = run(&a);
    let v = serde_json::from_str(out.trim())
        .unwrap_or_else(|e| panic!("{args:?}: not one JSON value ({e}): {out}"));
    (code, v)
}

#[test]
fn every_command_answers_in_json() {
    let d = Folder::new("json");
    let a = d.write(
        "a.csv",
        "name,party,amount\r\nAnn,D,10\r\nBo,R,x\r\nAnn,D,10\r\n",
    );
    let checks = d.write(
        "c.json",
        r#"{"checks":[{"column":"amount","kind":{"type":"number"}}]}"#,
    );
    let cleaner = d.write(
        "c.ndclean",
        r#"{"version":1,"name":"t","steps":[{"tool":"reverse"}]}"#,
    );
    let recipe = d.write("r.json", r#"{"version":1,"name":"r","steps":[]}"#);
    let pivot = d.write(
        "p.ndpivot",
        r#"{"version":1,"name":"p","spec":{"rows":[{"col":1}],"values":[{"col":2,"agg":"count"}]},"columns":[[1,"party"],[2,"amount"]]}"#,
    );
    let out = d.path("out.csv");
    let ok = |args: &[&str]| {
        let (code, v) = json(args);
        assert_eq!(code, 0, "{args:?}: {v}");
        v
    };
    let o = ["-o", out.as_str(), "--overwrite"];
    let w = |head: &[&str], tail: &[&str]| {
        let mut v = head.to_vec();
        v.extend(o);
        v.extend(tail);
        ok(&v)
    };
    assert!(ok(&["sniff", &a])["separator"].is_string());
    assert_eq!(ok(&["info", &a])["columns"], 3);
    assert!(ok(&["view", &a, "0", "2"])["rows"].is_array());
    assert_eq!(ok(&["headers", &a])[1]["title"], "party");
    assert_eq!(w(&["select", &a], &["--columns", "1,2"])["columns"], 2);
    assert!(w(&["filter", &a], &["--expr", "[party] = \"D\""])["kept"].is_number());
    assert!(w(&["sort", &a], &["--by", "name"])["message"].is_string());
    assert_eq!(w(&["dedupe", &a], &[])["removed"], 1);
    assert_eq!(ok(&["stats", &a])[0]["title"], "name");
    assert!(ok(&["count", &a, "--columns", "party"])["lines"].is_array());
    assert!(w(&["count", &a], &["--columns", "party"])["values"].is_number());
    assert_eq!(
        w(&["sample", &a], &["--rows", "1", "--seed", "7"])["seed"],
        7
    );
    assert!(w(&["slice", &a], &["--rows", "1-2"])["rows"].is_number());
    let parts = d.path("parts");
    assert!(ok(&["split", &a, "-o", &parts, "--rows", "2"]).is_array());
    assert!(w(&["cat", &a, &a], &[])["rows"].is_number());
    assert!(w(&["convert", &a], &[])["rows"].is_number());
    let v = w(&["compute", &a], &["--expr", "[amount] * 2"]);
    assert!(v["errors"].as_u64().unwrap() >= 1);
    assert!(w(&["clean", &cleaner, &a], &[])["changed"].is_number());
    assert!(w(&["run", &recipe, &a], &[])["steps"].is_array());
    assert_eq!(ok(&["run", &recipe, &a, "--dry-run"])["dry_run"], true);
    assert!(w(&["pivot", &pivot, &a], &[])["rows"].is_number());
    assert!(w(&["join", &a, &a], &["--key", "name", "--columns", "party"]).is_object());
    let (code, v) = json(&["validate", &a, "--checks", &checks]);
    assert_eq!(code, 1);
    assert!(v["problems"].is_number());

    // Failures are one JSON object and exit code 2.
    let missing = d.path("none.csv");
    for args in [
        vec!["info", &missing],
        vec!["headers", &missing],
        vec!["convert", &missing, "-o", &out],
        vec!["compute", &a, "-o", &out],
        vec!["clean", &cleaner, &missing, "-o", &out],
        vec!["run", &missing, &a],
        vec!["pivot", &missing, &a, "-o", &out],
        vec!["join", &a, &missing, "-o", &out, "--key", "name"],
        vec!["validate", &a, "--checks", &missing],
        vec!["app", "tabs"],
    ] {
        let (code, v) = json(&args);
        assert_eq!(code, 2, "{args:?}: {v}");
        assert!(v["error"].is_string(), "{args:?}: {v}");
    }

    // Without the flag the text is as before.
    let (_, text) = run(&["headers", &a]);
    assert_eq!(text.lines().next().unwrap().trim(), "1  name");
}

#[test]
fn cat_by_name_gives_what_combine_gives() {
    let d = Folder::new("cat");
    let texts = [
        "id,Note,Note\r\n1,a,b\r\n",
        "note ,id,extra,Note (2),,\r\nc,2,x,d,e,f\r\n",
        "id,,Note\r\n3,g,h\r\n4,,\r\n",
    ];
    let paths: Vec<String> = texts
        .iter()
        .enumerate()
        .map(|(i, t)| d.write(&format!("{i}.csv"), t))
        .collect();
    let out = d.path("out.csv");
    let mut args = vec!["cat"];
    args.extend(paths.iter().map(String::as_str));
    args.extend(["-o", &out, "--by-name"]);
    assert_eq!(run(&args).0, 0);

    let open =
        |p: &str| Document::open_indexed(Path::new(p), OpenOptions::default(), None).unwrap();
    let docs: Vec<Document> = paths.iter().map(|p| open(p)).collect();
    let tables: Vec<(String, &Document)> = docs.iter().map(|d| (String::new(), d)).collect();
    let spec = Combine {
        header: true,
        by_title: true,
        source: false,
    };
    let want = combine(&tables, &spec, &Progress::new())
        .unwrap()
        .into_document()
        .unwrap();
    let got = open(&out);
    assert_eq!(got.row_count(), want.row_count());
    for r in 0..got.row_count() {
        assert_eq!(
            got.row_values(r).unwrap(),
            want.row_values(r).unwrap(),
            "row {r}"
        );
    }
    // Every filled cell of every file came through.
    let all = std::fs::read_to_string(&out).unwrap();
    for v in ["a", "b", "c", "x", "d", "e", "f", "g", "h"] {
        assert!(
            all.lines().skip(1).any(|l| l.split(',').any(|c| c == v)),
            "{v} is missing:\n{all}"
        );
    }
}

#[test]
fn validate_finds_rows_that_repeat_several_columns_together() {
    let d = Folder::new("key");
    let a = d.write(
        "a.csv",
        "first,last,city\r\nAnn,Lee,Rome\r\nAnn,Lee,Oslo\r\nann,Lee,Rome\r\n",
    );
    let both = d.write(
        "both.json",
        r#"{"checks":[{"column":"first","also_columns":["city"],"kind":{"type":"unique"}}]}"#,
    );
    let (code, v) = json(&["validate", a.as_str(), "--checks", both.as_str()]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["problems"], 2, "{v}");
    assert_eq!(v["listed"][0]["value"], "Ann | Rome", "{v}");
    let clean = d.write(
        "clean.json",
        r#"{"checks":[{"column":"city","also_columns":[1, "LAST"],"kind":{"type":"unique","match_case":true}}]}"#,
    );
    let (code, v) = json(&["validate", a.as_str(), "--checks", clean.as_str()]);
    assert_eq!(code, 0, "{v}");
    for bad in [
        r#"{"checks":[{"column":"first","also_columns":["nope"],"kind":{"type":"unique"}}]}"#,
        r#"{"checks":[{"column":"first","also_columns":["city"],"kind":{"type":"required"}}]}"#,
        r#"{"checks":[{"column":"first","also_columns":"city","kind":{"type":"unique"}}]}"#,
        r#"{"checks":[{"column":"first","also_columns":["first"],"kind":{"type":"unique"}}]}"#,
    ] {
        let f = d.write("bad.json", bad);
        let (code, _) = run(&["validate", a.as_str(), "--checks", f.as_str()]);
        assert_eq!(code, 2, "{bad}");
    }
}
