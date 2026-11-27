//! Output cut short (`natendo-cli … | head`): the program stops printing quietly, finishes
//! its own work and exits with its own result. Made-up data only.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        let d = std::env::temp_dir().join(format!("nd-pipe-{name}-{}", std::process::id()));
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

/// Runs the program, reads `keep` bytes of its output (0: none), closes the output and
/// waits: (exit code, stderr).
fn cut(args: &[&str], keep: usize) -> (i32, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_natendo-cli"))
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    if keep > 0 {
        let mut buf = vec![0u8; keep];
        stdout.read_exact(&mut buf).unwrap();
    }
    drop(stdout);
    let mut err = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    let code = child.wait().unwrap().code().unwrap();
    (code, err)
}

fn voters(rows: usize) -> String {
    let mut t = String::from("id,first,last,ward\r\n");
    for i in 0..rows {
        t += &format!("{i},Ann{},Quorrin{},W{}\r\n", i % 97, i % 89, i % 4 + 1);
    }
    t
}

#[test]
fn printing_commands_stop_quietly_with_their_own_result() {
    let d = Folder::new("print");
    let a = d.write("a.csv", &voters(50_000));
    // Every id repeats once: 1,000 problems listed, far more output than a pipe holds.
    let mut dup = String::from("id,ward\r\n");
    for i in 0..20_000 {
        dup += &format!("{},W1\r\n", i / 2);
    }
    let b = d.write("b.csv", &dup);
    let checks = d.write(
        "c.json",
        r#"{"checks": [{"column": "id", "kind": {"type": "unique"}}]}"#,
    );
    let wide: Vec<String> = (0..6_000)
        .map(|c| format!("a rather long column title number {c}"))
        .collect();
    let w = d.write("w.csv", &(wide.join(",") + "\r\n"));
    for (args, want) in [
        (vec!["view", &a, "0", "50000"], 0),
        (vec!["view", &a, "0", "50000", "--json"], 0),
        (vec!["validate", &b, "--checks", &checks, "--json"], 1),
        (vec!["validate", &b, "--checks", &checks], 1),
        (vec!["headers", &w, "--json"], 0),
        (vec!["headers", &w], 0),
        (vec!["--version"], 0),
    ] {
        for keep in [0, 10] {
            if args[0] == "--version" && keep > 0 {
                continue;
            }
            let (code, err) = cut(&args, keep);
            assert_eq!(code, want, "{args:?} keep {keep}: {err}");
            assert!(err.is_empty(), "{args:?} keep {keep}: {err}");
        }
    }
}

#[test]
fn a_command_that_writes_a_file_finishes_it_when_nobody_reads_its_output() {
    let d = Folder::new("file");
    let a = d.write("a.csv", &voters(20_000));
    let recipe = d.write(
        "r.json",
        r#"{"version":1,"name":"r","header":true,"columns":["id","first","last","ward"],"steps":[{"op":"delete_cols","at":1,"count":1}]}"#,
    );
    let read = d.path("read.csv");
    let cut_out = d.path("cut.csv");
    let o = Command::new(env!("CARGO_BIN_EXE_natendo-cli"))
        .args(["run", &recipe, &a, "-o", &read, "--receipt"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0));
    let (code, err) = cut(&["run", &recipe, &a, "-o", &cut_out, "--receipt"], 0);
    assert_eq!(code, 0, "{err}");
    assert!(err.is_empty(), "{err}");
    assert_eq!(
        std::fs::read(&read).unwrap(),
        std::fs::read(&cut_out).unwrap()
    );
    assert!(std::path::Path::new(&(cut_out.clone() + ".ndreceipt.json")).exists());
    let temps: Vec<_> = std::fs::read_dir(&d.0)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".ndtmp"))
        .collect();
    assert!(temps.is_empty(), "{temps:?}");
}

#[test]
fn output_up_to_a_real_panic_is_not_lost() {
    let o = Command::new(env!("CARGO_BIN_EXE_natendo-cli"))
        .arg("--version")
        .env("NATENDO_CLI_TEST_PANIC", "1")
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&o.stdout);
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(out, "printed before the panic\n");
    assert!(err.contains("a test panic"), "{err}");
    assert_eq!(o.status.code(), Some(101));
}

#[test]
fn no_print_bypasses_the_output_module() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for e in std::fs::read_dir(src).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name == "out.rs" || !name.ends_with(".rs") {
            continue;
        }
        let text = std::fs::read_to_string(e.path()).unwrap();
        for (i, line) in text.lines().enumerate() {
            let bare = line
                .replace("crate::outln!", "")
                .replace("crate::out!", "")
                .replace("crate::errln!", "");
            assert!(
                !bare.contains("println!")
                    && !bare.contains("print!(")
                    && !bare.contains("eprint!("),
                "{name}:{}: print through crate::outln! / crate::errln!: {line}",
                i + 1
            );
        }
    }
}

#[test]
fn another_write_failure_fails_the_run_without_a_panic() {
    let d = Folder::new("fail");
    let a = d.write("a.csv", &voters(100));
    // Standard output where every write fails: on Windows a file opened for reading only;
    // elsewhere /dev/full (a bad handle is taken as a sink there, so it would not fail).
    #[cfg(windows)]
    let sink = std::fs::File::open(&a).unwrap();
    #[cfg(unix)]
    let sink = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_natendo-cli"))
        .args(["view", &a, "0", "50"])
        .stdout(Stdio::from(sink))
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(
        err.starts_with("natendo-cli: cannot write the output:"),
        "{err}"
    );
    assert!(!err.contains("panicked"), "{err}");
}
