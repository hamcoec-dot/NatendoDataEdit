//! `natendo-cli drift` (C9): a file checked against a file contract. Exit 0 when nothing drifted, 1 when
//! something did, 2 on an error. Made-up data only.

use std::path::PathBuf;
use std::process::Command;

use natendo_engine::contract::DraftSpec;
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
    fn write(&self, name: &str, bytes: &[u8]) -> String {
        std::fs::write(self.0.join(name), bytes).unwrap();
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
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&o.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&o.stderr));
    (o.status.code().unwrap_or(-1), text)
}

const JAN: &str =
    "id,name,party,phone\r\n1,Ann,DEM,555\r\n2,Bo,REP,556\r\n3,Cy,DEM,\r\n4,Di,REP,558\r\n";

#[test]
fn drift_exit_codes_and_json() {
    let f = Folder::new("drift");
    let d =
        Document::from_bytes(JAN.as_bytes().to_vec(), OpenOptions::default(), Some("csv")).unwrap();
    let c = d
        .contract_draft(
            &DraftSpec {
                name: "Monthly voters".into(),
                keep_first: true,
                allowed_cols: vec![2],
                ..Default::default()
            },
            &Progress::new(),
        )
        .unwrap()
        .contract;
    let contract = f.write("Monthly voters.ndcontract", &c.to_bytes().unwrap());
    let same = f.write("voters_jan.csv", JAN.as_bytes());
    let (code, out) = run(&["drift", &same, "--contract", &contract]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("nothing drifted"), "{out}");

    let feb = f.write(
        "voters_feb.csv",
        b"id,full_name,party,phone\r\n5,Ed,GRN,1\r\n6,Flo,DEM,2\r\n7,Gus,DEM,3\r\n8,Hal,REP,4\r\n",
    );
    let (code, out) = run(&["drift", &feb, "--contract", &contract]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("renamed: full_name is name"), "{out}");
    assert!(out.contains("party: 1 new values (GRN)"), "{out}");
    let (code, out) = run(&["drift", &feb, "--contract", &contract, "--json"]);
    assert_eq!(code, 1, "{out}");
    let v: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(v["drifted"], true);
    assert_eq!(v["columns"][2]["first_new"][0], "GRN");

    // A broken contract, and a missing one: errors (2).
    let broken = f.write("broken.ndcontract", b"{oops");
    let (code, out) = run(&["drift", &feb, "--contract", &broken]);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("cannot be read"), "{out}");
    let (code, _) = run(&["drift", &feb]);
    assert_eq!(code, 2);
}
