//! CLI parity commands through the built program: the same engine calls the
//! app makes (byte for byte where the output is deterministic), every exit
//! code they document, secrets never on the command line, and no output
//! file after a failure.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use natendo_engine::privacy::{ColumnAction, SafeCopy, Secret};
use natendo_engine::{Document, OpenOptions, Progress};
use serde_json::Value;

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Folder {
        let d = std::env::temp_dir().join(format!("nd-cli-parity-{name}-{}", std::process::id()));
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
    /// No app temp files and no file of this name left behind.
    fn clean_of(&self, name: &str) -> bool {
        let temps = std::fs::read_dir(&self.0)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .is_ok_and(|e| e.file_name().to_string_lossy().starts_with(".ndtmp-"))
            })
            .count();
        temps == 0 && !self.0.join(name).exists()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Exit code, standard output, standard error; `stdin` is typed in; `env`
/// is added to the environment.
fn cli(args: &[&str], stdin: Option<&str>, env: &[(&str, &str)]) -> (i32, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_natendo-cli"));
    c.args(args)
        .env("APPDATA", std::env::temp_dir().join("nd-cli-no-app"))
        .env_remove("NATENDO_PHRASE")
        .env_remove("NATENDO_PASSWORD")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        c.env(k, v);
    }
    let mut child = c.spawn().unwrap();
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }
    let o = child.wait_with_output().unwrap();
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).to_string(),
        String::from_utf8_lossy(&o.stderr).to_string(),
    )
}

fn run(args: &[&str]) -> (i32, String, String) {
    cli(args, None, &[])
}

fn json(args: &[&str]) -> (i32, Value) {
    let mut a = args.to_vec();
    a.push("--json");
    let (code, out, err) = run(&a);
    let v = serde_json::from_str(out.trim())
        .unwrap_or_else(|e| panic!("{args:?}: not one JSON value ({e}): {out} {err}"));
    (code, v)
}

fn open(p: &str) -> Document {
    Document::open_indexed(Path::new(p), OpenOptions::default(), None).unwrap()
}

const PEOPLE: &str = "id,name,city,dob\r\n1,Ann Lee,Oak,1980-01-02\r\n2,Bob Ray,Elm,1975-05-06\r\n3,Cy Dunn,Fir,1990-07-08\r\n";
const PEOPLE2: &str = "id,name,city,dob\r\n1,Ann Lee,Oak,1980-01-02\r\n2,Bob Ray,Pine,1975-05-06\r\n4,Di Moe,Ash,1991-01-01\r\n";
const PHRASE: &str = "correct horse battery staple";

#[test]
fn compare_counts_are_the_engines_and_values_stay_out_unless_asked() {
    let d = Folder::new("compare");
    let a = d.write("a.csv", PEOPLE);
    let b = d.write("b.csv", PEOPLE2);
    let (code, v) = json(&["compare", &a, &b, "--key", "id"]);
    assert_eq!(code, 0, "{v}");
    let spec = natendo_engine::diff::DiffSpec {
        keys: vec![0],
        keep_first: true,
        match_case: false,
        trim: true,
    };
    let want = open(&a)
        .diff_files(&open(&b), &spec, &Progress::new())
        .unwrap()
        .report;
    for (k, n) in [
        ("same", want.same),
        ("changed", want.changed),
        ("added", want.added),
        ("removed", want.removed),
        ("cells", want.cells),
    ] {
        assert_eq!(v[k], n, "{k}: {v}");
    }
    assert_eq!(v["changed_columns"][0]["column"], "city");
    // No cell values without --values.
    let text = v.to_string();
    assert!(
        !text.contains("Pine") && !text.contains("Di Moe") && v.get("rows").is_none(),
        "{text}"
    );
    let (code, v) = json(&["compare", &a, &b, "--key", "id", "--values"]);
    assert_eq!(code, 0);
    assert_eq!(v["rows"].as_array().unwrap().len(), 3);
    // Exit 1 only with --fail-on-diff; the same file never differs.
    assert_eq!(
        json(&["compare", &a, &b, "--key", "id", "--fail-on-diff"]).0,
        1
    );
    assert_eq!(
        json(&["compare", &a, &a, "--key", "id", "--fail-on-diff"]).0,
        0
    );
    // The differences file, and the usual refusals.
    let out = d.path("diff.csv");
    assert_eq!(run(&["compare", &a, &b, "--key", "id", "-o", &out]).0, 0);
    let diff = std::fs::read_to_string(&out).unwrap();
    assert!(
        diff.starts_with("Change,Row (first)") && diff.contains("Pine"),
        "{diff}"
    );
    assert_eq!(run(&["compare", &a, &b, "--key", "id", "-o", &out]).0, 2);
    assert_eq!(
        run(&["compare", &a, &b, "--key", "id", "-o", &b, "--overwrite"]).0,
        2
    );
    assert_eq!(run(&["compare", &a, &b]).0, 2);
    assert_eq!(run(&["compare", &a, &b, "--key", "nope"]).0, 2);
}

#[test]
fn match_and_lookalikes_write_their_files() {
    let d = Folder::new("match");
    let a = d.write("a.csv", PEOPLE);
    let c = d.write("c.csv", "id,full name\r\n7,Anne Lee\r\n8,Robert Ray\r\n");
    let pairs = d.path("pairs.csv");
    let (code, v) = json(&[
        "match",
        &a,
        &c,
        "--field",
        "name=full name:name",
        "--min-score",
        "60",
        "-o",
        &pairs,
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["total"], 2);
    assert!(v["histogram"].is_array());
    let text = std::fs::read_to_string(&pairs).unwrap();
    assert!(text.starts_with("Row,Other row,Score,name,full name (other),name likeness"));
    assert!(text.contains("Bob Ray,Robert Ray"), "{text}");
    for bad in [
        vec!["match", &a, &c],
        vec!["match", &a, &c, "--field", "name:fuzzy"],
        vec!["match", &a, &c, "--field", "name:name:0"],
        vec!["match", &a, &c, "--field", "name", "--min-score", "120"],
        vec!["match", &a, &c, "--field", "name", "-o", &a, "--overwrite"],
    ] {
        assert_eq!(run(&bad).0, 2, "{bad:?}");
    }
    let l = d.write("l.csv", "name\r\nSmith\r\nSmyth\r\nSmith\r\nJones\r\n");
    let groups = d.path("g.csv");
    let (code, v) = json(&[
        "lookalikes",
        &l,
        "--column",
        "name",
        "--method",
        "both",
        "-o",
        &groups,
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["groups"][0][0]["value"], "Smith");
    assert_eq!(v["groups"][0][0]["count"], 2);
    let g = std::fs::read_to_string(&groups).unwrap();
    assert!(g.contains("1,Smyth,1,Smith"), "{g}");
    assert_eq!(
        run(&["lookalikes", &l, "--column", "name", "--method", "x"]).0,
        2
    );
}

#[test]
fn xray_encoding_and_private_report_counts_not_text() {
    let d = Folder::new("inspect");
    let g = d.write(
        "g.csv",
        "name,note\r\nAnn\u{200b},plain\r\nBo,\u{201c}hi\u{201d}\r\nCy,ok\r\n",
    );
    let (code, v) = json(&["xray", &g]);
    assert_eq!(code, 0);
    let want = open(&g).xray_report(&[], true, &Progress::new()).unwrap();
    assert_eq!(v["cells"], want.cells);
    assert_eq!(v["rows"], want.rows);
    assert_eq!(v["first_rows"], serde_json::json!([2, 3]));
    assert!(!v.to_string().contains("Ann"));
    assert_eq!(json(&["xray", &g, "--fail-on-found"]).0, 1);
    // --sanitize: the cleaned file has none left.
    let clean = d.path("clean.csv");
    let (code, v) = json(&["xray", &g, "--sanitize", "-o", &clean]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["changed"], 2);
    assert_eq!(json(&["xray", &clean, "--fail-on-found"]).0, 0);
    assert_eq!(run(&["xray", &g, "--homoglyphs"]).0, 2);
    assert_eq!(run(&["xray", &g, "-o", &clean]).0, 2);

    // A Windows-1252 file: detected, scanned, and read as asked.
    let w = d.path("w.csv");
    std::fs::write(&w, b"name\r\nRen\xe9\r\nZo\xeb\r\n").unwrap();
    assert_eq!(
        json(&["encoding", "detect", &w]).1["encoding"],
        "windows-1252"
    );
    let (code, v) = json(&["encoding", "scan", &w, "--as", "utf-8", "--fail-on-found"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["invalid"], 2);
    assert_eq!(v["first_rows"], serde_json::json!([2, 3]));
    let (code, out, _) = run(&["--encoding", "windows-1252", "view", &w]);
    assert_eq!(code, 0);
    assert!(out.contains("René"), "{out}");
    let (code, _, err) = run(&["--encoding", "klingon", "view", &w]);
    assert_eq!(code, 2);
    assert!(err.contains("windows-1252"), "{err}");
    // convert --to-encoding: UTF-16 with its BOM, and back.
    let u16 = d.path("u16.csv");
    assert_eq!(
        run(&["convert", &w, "-o", &u16, "--to-encoding", "utf-16le"]).0,
        0
    );
    assert!(std::fs::read(&u16).unwrap().starts_with(b"\xff\xfe"));
    let back = open(&u16);
    assert_eq!(back.cell(1, 0).unwrap(), "René");
    assert_eq!(
        run(&[
            "convert",
            &w,
            "-o",
            &d.path("x.json"),
            "--to-encoding",
            "utf-16le"
        ])
        .0,
        2
    );

    let p = d.write("p.csv", PEOPLE);
    let (code, v) = json(&["private", &p, "--fail-on-found"]);
    assert_eq!(code, 1);
    assert_eq!(v["columns"][0]["column"], "dob");
    assert_eq!(v["columns"][0]["kind"], "birth_date");
    assert!(!v.to_string().contains("1980"));
    assert_eq!(json(&["private", &p]).0, 0);
}

/// The app's share-safe spec for PEOPLE, by place, with the phrase.
fn engine_spec() -> SafeCopy {
    let clean = |steps: &str| ColumnAction::Clean {
        cleaner: serde_json::from_str(&format!(r#"{{"steps":{steps}}}"#)).unwrap(),
    };
    let mut scramble = clean(r#"[{"tool":"scramble"}]"#);
    if let ColumnAction::Clean { cleaner } = &mut scramble {
        if let natendo_engine::clean::Step::Scramble { phrase, .. } = &mut cleaner.steps[0] {
            *phrase = Secret::new(PHRASE);
        }
    }
    SafeCopy {
        columns: vec![
            ColumnAction::Keep,
            scramble,
            ColumnAction::Drop,
            clean(r#"[{"tool":"mask","keep_last":4}]"#),
        ],
        header: true,
        read: Default::default(),
        guard: true,
    }
}

#[test]
fn share_safe_is_byte_identical_to_the_app_path_and_fails_closed() {
    let d = Folder::new("safe");
    let a = d.write("a.csv", PEOPLE);
    // By title, in another order than the file's.
    let spec = d.write(
        "s.json",
        r#"{"columns":[
            {"title":"dob","action":"clean","cleaner":{"steps":[{"tool":"mask","keep_last":4}]}},
            {"title":"name","action":"clean","cleaner":{"steps":[{"tool":"scramble"}]}},
            {"title":"id","action":"keep"},
            {"column":3,"action":"drop"}]}"#,
    );
    let phrase = d.write("phrase.txt", &format!("{PHRASE}\r\n"));
    let out = d.path("out.csv");
    let (code, v) = json(&[
        "share-safe",
        &a,
        "--spec",
        &spec,
        "-o",
        &out,
        "--phrase-file",
        &phrase,
    ]);
    assert_eq!(code, 0, "{v}");
    let app = d.path("app.csv");
    open(&a)
        .save_safe_copy(Path::new(&app), &engine_spec(), 20_000.0, &Progress::new())
        .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), std::fs::read(&app).unwrap());
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(!text.contains("Ann Lee") && !text.contains("Oak"), "{text}");
    // The phrase from standard input and from the environment gives the same bytes.
    let out2 = d.path("out2.csv");
    let (code, _, err) = cli(
        &[
            "share-safe",
            &a,
            "--spec",
            &spec,
            "-o",
            &out2,
            "--phrase-stdin",
        ],
        Some(&format!("{PHRASE}\n")),
        &[],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(std::fs::read(&out2).unwrap(), std::fs::read(&app).unwrap());
    let out3 = d.path("out3.csv");
    let (code, _, err) = cli(
        &["share-safe", &a, "--spec", &spec, "-o", &out3],
        None,
        &[("NATENDO_PHRASE", PHRASE)],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(std::fs::read(&out3).unwrap(), std::fs::read(&app).unwrap());

    // P1: a spec that does not fit is refused, naming the columns, and writes nothing.
    let bad = |name: &str, cols: &str, file: &str, says: &str| {
        let s = d.write(name, &format!(r#"{{"columns":{cols}}}"#));
        let o = d.path("bad.csv");
        let (code, _, err) = run(&[
            "share-safe",
            file,
            "--spec",
            &s,
            "-o",
            &o,
            "--phrase-file",
            &phrase,
        ]);
        assert_eq!(code, 2, "{name}: {err}");
        assert!(err.contains(says), "{name}: {err}");
        assert!(d.clean_of("bad.csv"), "{name}: a file was left");
    };
    let keep = |t: &str| format!(r#"{{"title":"{t}","action":"keep"}}"#);
    let scr = r#"{"title":"name","action":"clean","cleaner":{"steps":[{"tool":"scramble"}]}}"#;
    bad(
        "missing.json",
        &format!(
            "[{},{scr},{},{},{}]",
            keep("id"),
            keep("city"),
            keep("dob"),
            keep("ssn")
        ),
        &a,
        "not in the file: \"ssn\"",
    );
    bad(
        "left.json",
        &format!("[{},{scr},{}]", keep("id"), keep("city")),
        &a,
        "no action (keep, drop or clean) for: \"dob\" (column 4)",
    );
    bad(
        "twice.json",
        &format!(
            r#"[{},{scr},{},{},{{"column":1,"action":"drop"}}]"#,
            keep("id"),
            keep("city"),
            keep("dob")
        ),
        &a,
        "given two actions: column 1",
    );
    let dup = d.write("dup.csv", "id,name,dob,dob\r\n1,Ann,1980,1981\r\n");
    bad(
        "shared.json",
        &format!("[{},{scr},{}]", keep("id"), keep("dob")),
        &dup,
        "more than one column of the file has this title: \"dob\"",
    );
    // A scramble with no phrase, or a phrase too short: refused before any file exists.
    let o = d.path("np.csv");
    assert_eq!(run(&["share-safe", &a, "--spec", &spec, "-o", &o]).0, 2);
    let short = d.write("short.txt", "too short\n");
    let (code, _, err) = run(&[
        "share-safe",
        &a,
        "--spec",
        &spec,
        "-o",
        &o,
        "--phrase-file",
        &short,
    ]);
    assert_eq!(code, 2);
    assert!(err.contains("at least 16"), "{err}");
    assert!(d.clean_of("np.csv"));
    // P5: a phrase in the spec is refused, unless allowed.
    let inside = d.write(
        "in.json",
        &format!(
            r#"{{"columns":[{},{{"title":"name","action":"clean","cleaner":{{"steps":[{{"tool":"scramble","Phrase":"{PHRASE}"}}]}}}},{},{}]}}"#,
            keep("id"),
            keep("city"),
            keep("dob")
        ),
    );
    let (code, _, err) = run(&["share-safe", &a, "--spec", &inside, "-o", &o]);
    assert_eq!(code, 2);
    assert!(
        err.contains("--allow-phrase-in-file") && !err.contains(PHRASE),
        "{err}"
    );
    let lower = d.write(
        "lower.json",
        &std::fs::read_to_string(&inside)
            .unwrap()
            .replace("Phrase", "phrase"),
    );
    assert_eq!(
        run(&[
            "share-safe",
            &a,
            "--spec",
            &lower,
            "-o",
            &o,
            "--allow-phrase-in-file"
        ])
        .0,
        0
    );
}

#[test]
fn encrypt_and_decrypt_agree_with_the_app_both_ways() {
    let d = Folder::new("crypt");
    let a = d.write("a.csv", PEOPLE);
    let pw = d.write("pw.txt", "a long test password\n");
    let enc = d.path("a.ndenc");
    assert_eq!(
        run(&["encrypt", &a, "-o", &enc, "--password-file", &pw]).0,
        0
    );
    // The engine (the app's open path) reads what the command line wrote.
    let opened =
        Document::open_encrypted(Path::new(&enc), &Secret::new("a long test password")).unwrap();
    assert_eq!(opened.cell(2, 1).unwrap(), "Bob Ray");
    let plain = d.path("plain.csv");
    let (code, _, err) = cli(
        &["decrypt", &enc, "-o", &plain, "--password-stdin"],
        Some("a long test password\n"),
        &[],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(std::fs::read_to_string(&plain).unwrap(), PEOPLE);
    // And the command line reads what the app wrote.
    let app = d.path("app.ndenc");
    open(&a)
        .save_encrypted(
            Path::new(&app),
            &Secret::new("a long test password"),
            &Progress::new(),
        )
        .unwrap();
    let back = d.path("back.csv");
    let (code, _, err) = cli(
        &["decrypt", &app, "-o", &back],
        None,
        &[("NATENDO_PASSWORD", "a long test password")],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(std::fs::read_to_string(&back).unwrap(), PEOPLE);

    // A wrong password: exit 2 and no file.
    let wrong = d.write("wrong.txt", "not the right password\n");
    let (code, _, err) = run(&[
        "decrypt",
        &enc,
        "-o",
        &d.path("w.csv"),
        "--password-file",
        &wrong,
    ]);
    assert_eq!(code, 2);
    assert!(err.contains("wrong password"), "{err}");
    assert!(d.clean_of("w.csv"));
    // A write that fails at the end (the target is a folder): no file, no temp file.
    let folder = d.path("folder.csv");
    std::fs::create_dir(&folder).unwrap();
    assert_eq!(
        run(&[
            "decrypt",
            &enc,
            "-o",
            &folder,
            "--password-file",
            &pw,
            "--overwrite"
        ])
        .0,
        2
    );
    assert!(Path::new(&folder).is_dir());
    assert!(d.clean_of("none"));
    // A short password, or none: refused before any file exists.
    let short = d.write("short.txt", "eleven char\n");
    let (code, _, err) = run(&[
        "encrypt",
        &a,
        "-o",
        &d.path("s.ndenc"),
        "--password-file",
        &short,
    ]);
    assert_eq!(code, 2);
    assert!(err.contains("at least 12"), "{err}");
    assert_eq!(run(&["encrypt", &a, "-o", &d.path("s.ndenc")]).0, 2);
    assert!(d.clean_of("s.ndenc"));
    assert_eq!(
        run(&[
            "encrypt",
            &a,
            "-o",
            &d.path("s.csv"),
            "--password-file",
            &pw
        ])
        .0,
        2
    );
    // Not an encrypted file.
    assert_eq!(
        run(&[
            "decrypt",
            &a,
            "-o",
            &d.path("n.csv"),
            "--password-file",
            &pw
        ])
        .0,
        2
    );
    // An encrypted file under another name is still refused as input (found by its header).
    let renamed = d.path("looks-plain.csv");
    std::fs::copy(&enc, &renamed).unwrap();
    for args in [
        vec!["info", renamed.as_str()],
        vec!["headers", renamed.as_str()],
        vec!["convert", renamed.as_str(), "-o", "x.csv"],
        vec!["compare", renamed.as_str(), a.as_str(), "--key", "id"],
        vec!["private", renamed.as_str()],
    ] {
        let (code, _, err) = run(&args);
        assert_eq!(code, 2, "{args:?}");
        assert!(
            err.contains("encrypted file: decrypt it first"),
            "{args:?}: {err}"
        );
    }
}

#[test]
fn secrets_never_come_from_arguments_and_come_one_way_only() {
    let d = Folder::new("secret");
    let a = d.write("a.csv", PEOPLE);
    let pw = d.write("pw.txt", "a long test password\n");
    let o = d.path("o.ndenc");
    for typed in [
        "--password=hunter2hunter2",
        "--password",
        "--passphrase=hunter2hunter2",
    ] {
        let (code, out, err) = run(&["encrypt", &a, "-o", &o, typed, "hunter2hunter2"]);
        assert_eq!(code, 2, "{typed}");
        assert!(
            !out.contains("hunter2") && !err.contains("hunter2"),
            "{err}"
        );
    }
    // Two sources, or standard input asked to carry two things.
    let (code, _, err) = cli(
        &["encrypt", &a, "-o", &o, "--password-file", &pw],
        None,
        &[("NATENDO_PASSWORD", "a long test password")],
    );
    assert_eq!(code, 2);
    assert!(err.contains("one way only"), "{err}");
    let (code, _, err) = run(&["encrypt", "-", "-o", &o, "--password-stdin"]);
    assert_eq!(code, 2);
    assert!(err.contains("standard input"), "{err}");
    let (code, _, _) = run(&[
        "patch",
        "create",
        &a,
        &a,
        "--key",
        "id",
        "-o",
        &d.path("p.ndpatch"),
        "--phrase-stdin",
        "--password-stdin",
    ]);
    assert_eq!(code, 2);
    assert!(d.clean_of("o.ndenc"));

    // A cleaner or recipe that holds a secret is refused (any case, any depth).
    let cleaner = d.write(
        "c.ndclean",
        &format!(
            r#"{{"version":1,"name":"s","steps":[{{"tool":"scramble","PHRASE":"{PHRASE}"}}]}}"#
        ),
    );
    let out = d.path("c.csv");
    let (code, _, err) = run(&["clean", &cleaner, &a, "-o", &out]);
    assert_eq!(code, 2);
    assert!(err.contains("--allow-phrase-in-file"), "{err}");
    let recipe = d.write(
        "r.json",
        r#"{"version":1,"name":"r","steps":[],"notes":{"password":"x"}}"#,
    );
    let (code, _, err) = run(&["run", &recipe, &a, "-o", &out]);
    assert_eq!(code, 2);
    assert!(err.contains("\"password\""), "{err}");
    assert!(d.clean_of("c.csv"));
    // A scramble cleaner takes the phrase from standard input, as share-safe does.
    let plain = d.write(
        "p.ndclean",
        r#"{"version":1,"name":"s","steps":[{"tool":"scramble"}]}"#,
    );
    let (code, _, err) = cli(
        &[
            "clean",
            &plain,
            &a,
            "-o",
            &out,
            "--columns",
            "2",
            "--skip-header",
            "--phrase-stdin",
        ],
        Some(PHRASE),
        &[],
    );
    assert_eq!(code, 0, "{err}");
    assert!(!std::fs::read_to_string(&out).unwrap().contains("Ann Lee"));
    assert_eq!(run(&["clean", &plain, &a, "-o", &d.path("c2.csv")]).0, 2);
}

#[test]
fn cat_with_a_column_map_gives_the_apps_rows_and_refuses_what_it_cannot_decide() {
    use natendo_engine::combine::{Extras, SavedColumn, SavedMapping, MAPPING_KIND};
    let d = Folder::new("cat-map");
    let a = d.write("a.csv", "Cust_ID,Name,Precinct 1\r\n1,Ann,P1\r\n");
    let b = d.write("b.csv", "Name,CustID,Precinct 2,Notes\r\nBo,2,P2,new\r\n");
    let map = SavedMapping {
        kind: MAPPING_KIND.into(),
        version: 1,
        tool: "t".into(),
        header: true,
        file_name_column: false,
        extras: Extras::Add,
        columns: vec![
            SavedColumn {
                title: "Cust_ID".into(),
                from: vec!["CustID".into()],
            },
            SavedColumn {
                title: "Name".into(),
                from: vec![],
            },
            SavedColumn {
                title: "Precinct 1".into(),
                from: vec![],
            },
            SavedColumn {
                title: "Precinct 2".into(),
                from: vec![],
            },
        ],
    };
    let m = d.path("m.ndcombine");
    std::fs::write(&m, map.to_json()).unwrap();
    let out = d.path("out.csv");
    let (code, v) = json(&["cat", &a, &b, "-o", &out, "--mapping", &m]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["fit"]["extras"].as_array().unwrap().len(), 1, "{v}");
    // The app path: the same map fitted and combined by the engine.
    let docs = [open(&a), open(&b)];
    let rows: Vec<Vec<String>> = docs.iter().map(|x| x.row_values(0).unwrap()).collect();
    let mut fit = map.apply(&rows);
    fit.plan.compact();
    let tables: Vec<(String, &Document)> = docs.iter().map(|x| (String::new(), x)).collect();
    let spec = natendo_engine::tools::Combine {
        header: true,
        by_title: true,
        source: false,
    };
    let want =
        natendo_engine::tools::combine_with_plan(&tables, &fit.plan, &spec, &Progress::new())
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
    assert_eq!(
        got.row_values(0).unwrap(),
        ["Cust_ID", "Name", "Precinct 1", "Precinct 2", "Notes"]
    );
    // --strict: an extra column (or a missing one) is exit 2, and nothing is written.
    let strict = d.path("strict.csv");
    let (code, _, err) = run(&["cat", &a, &b, "-o", &strict, "--mapping", &m, "--strict"]);
    assert_eq!(code, 2);
    assert!(err.contains("--strict") && err.contains("Notes"), "{err}");
    assert!(d.clean_of("strict.csv"));
    // Two columns of one file matching one map column: never guessed.
    let c = d.write("c.csv", "Cust_ID,CustID\r\n1,2\r\n");
    let (code, _, err) = run(&["cat", &a, &c, "-o", &strict, "--mapping", &m]);
    assert_eq!(code, 2);
    assert!(err.contains("c.csv") && err.contains("both go to"), "{err}");
    assert!(d.clean_of("strict.csv"));
    // --plan writes nothing and is one JSON value, with the suggestion.
    let (code, v) = json(&["cat", &a, &b, "--plan", "--by-name"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["titles"][3], "CustID");
    assert_eq!(v["suggestions"][0]["why"]["kind"], "tidied");
    assert_eq!(
        v["suggestions"].as_array().unwrap().len(),
        1,
        "Precinct 1 / 2 are not suggested: {v}"
    );
    // A bad map file.
    let bad = d.write("bad.ndcombine", "{\"kind\":\"other\"}");
    assert_eq!(
        run(&["cat", &a, &b, "-o", &out, "--overwrite", "--mapping", &bad]).0,
        2
    );
    assert_eq!(
        run(&["cat", &a, &b, "-o", &out, "--overwrite", "--strict"]).0,
        2
    );
}
