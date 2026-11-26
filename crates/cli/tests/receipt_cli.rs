//! `--receipt` on run and convert, and `receipt verify`, through the built program.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        let d = std::env::temp_dir().join(format!("nd-cli-receipt-{name}-{}", std::process::id()));
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
        .env("APPDATA", std::env::temp_dir().join("nd-cli-no-app"))
        .output()
        .unwrap();
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).to_string() + &String::from_utf8_lossy(&o.stderr),
    )
}

fn json(args: &[&str]) -> (i32, Value) {
    let mut a = args.to_vec();
    a.push("--json");
    let (code, out) = run(&a);
    (
        code,
        serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("{args:?}: {e}: {out}")),
    )
}

#[test]
fn receipts_from_runs_verify_chain_and_show_changes() {
    let d = Folder::new("run");
    let a = d.write("a.csv", "name,n\r\nAnn Smith,1\r\nBo,2\r\n");
    let recipe = d.write(
        "r.json",
        r#"{"version":1,"name":"r","header":true,"columns":["name","n"],"steps":[{"op":"delete_cols","at":1,"count":1}]}"#,
    );
    let b = d.path("b.csv");
    let c = d.path("c.csv");
    let (code, v) = json(&["run", &recipe, &a, "-o", &b, "--receipt"]);
    assert_eq!(code, 0, "{v}");
    let rb = v["receipt"].as_str().unwrap().to_string();
    assert!(rb.ends_with("b.csv.ndreceipt.json"));
    let text = std::fs::read_to_string(&rb).unwrap();
    // File names and step kinds only: no cell values, no folder.
    assert!(
        text.contains("\"delete_cols\"")
            && text.contains("\"a.csv\"")
            && text.contains("\"b.csv\"")
    );
    assert!(
        !text.contains("Ann") && !text.contains(&*d.0.to_string_lossy().replace('\\', "\\\\")),
        "{text}"
    );
    let (code, _) = run(&["convert", &b, "-o", &c, "--receipt"]);
    assert_eq!(code, 0);
    let rc = d.path("c.csv.ndreceipt.json");
    // Everything matches, and the chain holds.
    let (code, v) = json(&[
        "receipt", "verify", &rc, "--input", &b, "--output", &c, "--chain",
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["receipt"]["action"], "convert");
    let (code, out) = run(&["receipt", "verify", &rb, "--input", &a, "--output", &b]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("checks out"));
    // A seal alone: not a pass (exit 1, ok false), and it says no files were compared.
    let (code, v) = json(&["receipt", "verify", &rc]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["ok"], false);
    assert_eq!(v["compared"], 0);
    assert_eq!(v["checks"][0]["ok"], true);
    let (code, out) = run(&["receipt", "verify", &rc]);
    assert_eq!(code, 1);
    assert!(
        out.contains("The seal matches, but no files were compared"),
        "{out}"
    );
    // The output changed: exit 1 and says which.
    std::fs::write(&c, "name\r\nAnn\r\n").unwrap();
    let (code, v) = json(&["receipt", "verify", &rc, "--output", &c]);
    assert_eq!(code, 1);
    assert_eq!(v["checks"][1]["ok"], false);
    // The earlier receipt edited: the chain breaks.
    std::fs::write(&rb, text.replace("\"run recipe\"", "\"something\"")).unwrap();
    let (code, _) = json(&["receipt", "verify", &rc, "--input", &b, "--chain"]);
    assert_eq!(code, 1);
    // Full paths only when asked.
    let e = d.path("e.csv");
    let (code, _) = run(&["convert", &a, "-o", &e, "--receipt", "--receipt-paths"]);
    assert_eq!(code, 0);
    let t = std::fs::read_to_string(d.path("e.csv.ndreceipt.json")).unwrap();
    assert!(t.contains("\"path\""));
}

#[test]
fn receipt_mistakes_are_errors() {
    let d = Folder::new("bad");
    let a = d.write("a.csv", "n\r\n1\r\n");
    let recipe = d.write("r.json", r#"{"version":1,"name":"r","steps":[]}"#);
    let o = d.path("o.csv");
    for args in [
        vec!["run", recipe.as_str(), a.as_str(), "--dry-run", "--receipt"],
        vec!["convert", a.as_str(), "-o", o.as_str(), "--receipt-paths"],
        vec!["receipt", "verify"],
        vec!["receipt", "verify", a.as_str()],
        vec!["receipt", "verify", a.as_str(), "--input"],
    ] {
        let (code, out) = run(&args);
        assert_eq!(code, 2, "{args:?}: {out}");
    }
    assert!(!std::path::Path::new(&o).exists());
}
