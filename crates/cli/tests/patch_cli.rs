//! `natendo-cli patch`: exit codes, protection, and refusals that never show
//! a value.

use std::io::Write;
use std::process::{Command, Stdio};

const PHRASE: &str = "a long made-up phrase";

/// (exit code, standard output, standard error).
fn run(args: &[&str], stdin: &str) -> (i32, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_natendo-cli"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A command that refuses early may not read it.
    let _ = c.stdin.take().unwrap().write_all(stdin.as_bytes());
    let o = c.wait_with_output().unwrap();
    (
        o.status.code().unwrap(),
        String::from_utf8(o.stdout).unwrap(),
        String::from_utf8(o.stderr).unwrap(),
    )
}

#[test]
fn create_apply_and_the_exit_codes() {
    let dir = std::env::temp_dir().join(format!("nde-cli-patch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = |n: &str| dir.join(n).display().to_string();
    let write = |n: &str, t: &str| std::fs::write(dir.join(n), t).unwrap();
    write("old.csv", "id,name\r\n1,ann\r\n2,bob\r\n");
    write("new.csv", "id,name\r\n1,amy\r\n3,cyd\r\n");
    let (old, new, fix, out) = (
        path("old.csv"),
        path("new.csv"),
        path("fix.ndpatch"),
        path("out.csv"),
    );
    let make = ["patch", "create", &old, &new, "--key", "ID", "-o", &fix];
    let with = |more: &[&'static str]| {
        let mut v = make.to_vec();
        v.extend(more);
        v
    };
    // Protection is not skipped by accident, and a phrase is not an argument.
    let (code, _, err) = run(&make, "");
    assert_eq!(code, 2, "{err}");
    assert!(err.contains("--plain"), "{err}");
    assert!(!dir.join("fix.ndpatch").exists());
    let (code, _, err) = run(&with(&["--phrase=hunter2 hunter2"]), "");
    assert_eq!(code, 2);
    assert!(!err.contains("hunter2"), "{err}");
    let (code, said, err) = run(&with(&["--phrase-stdin"]), &format!("{PHRASE}\r\n"));
    assert_eq!(code, 0, "{err}");
    assert!(said.contains("1 rows changed (1 cells), 1 added, 1 removed"));
    assert!(said.contains("protected with the phrase"));
    let bytes = std::fs::read(dir.join("fix.ndpatch")).unwrap();
    assert!(!bytes.windows(3).any(|w| w == b"amy"));
    // It is there now.
    assert_eq!(run(&with(&["--plain"]), "").0, 2);
    write("phrase.txt", &format!("{PHRASE}\n"));
    let phrase = path("phrase.txt");
    let apply = |data: &str, more: &[&str]| {
        let mut v = vec!["patch", "apply", &fix, data, "--phrase-file", &phrase];
        v.extend(more);
        run(&v, "")
    };
    assert_eq!(run(&["patch", "apply", &fix, &old, "--dry-run"], "").0, 2);
    let (code, said, _) = apply(&old, &["--dry-run"]);
    assert_eq!(code, 0, "{said}");
    assert!(said.starts_with("3 would apply"), "{said}");
    assert!(!said.contains("amy"));
    let (code, _, err) = apply(&old, &["-o", &old, "--overwrite"]);
    assert_eq!(code, 2);
    assert!(err.contains("different file"), "{err}");
    assert_eq!(apply(&old, &["-o", &fix, "--overwrite"]).0, 2);
    let (code, said, err) = apply(&old, &["-o", &out]);
    assert_eq!(code, 0, "{said}{err}");
    assert_eq!(
        std::fs::read_to_string(dir.join("out.csv")).unwrap(),
        "id,name\r\n1,amy\r\n3,cyd\r\n"
    );
    // Again: already applied is success.
    let (code, said, _) = apply(&out, &["--dry-run"]);
    assert_eq!(code, 0);
    assert!(said.contains("3 already applied"), "{said}");
    // A conflict: exit 1, and the value stays.
    write("other.csv", "id,name\r\n1,zed\r\n2,bob\r\n");
    let (code, said, _) = apply(&path("other.csv"), &["-o", &out, "--overwrite"]);
    assert_eq!(code, 1, "{said}");
    assert!(std::fs::read_to_string(dir.join("out.csv"))
        .unwrap()
        .contains("1,zed"));
    // A repeated key is refused by its row number, never its value.
    write("twice.csv", "id,name\r\nQX-77,a\r\nQX-77,b\r\n");
    let twice = path("twice.csv");
    let plain = path("p.ndpatch");
    let (code, said, err) = run(
        &[
            "patch", "create", &twice, &new, "--key", "id", "-o", &plain, "--plain",
        ],
        "",
    );
    assert_eq!(code, 2);
    assert!(err.contains("row 3 of the older table"), "{err}");
    assert!(!err.contains("QX-77") && !said.contains("QX-77"));
    std::fs::remove_dir_all(&dir).unwrap();
}
