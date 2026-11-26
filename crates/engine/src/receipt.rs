//! Processing receipts: `<output>.ndreceipt.json`, written beside a file only when the person
//! asks, saying which files went in and came out (names, sizes, SHA-256 of their bytes), when,
//! with which tool, and which kinds of steps ran, sealed with [`crate::hashing::seal`] so a
//! change to the receipt shows. A receipt beside an input is chained by its own seal.
//!
//! This is tamper-evidence by hashing, **not a signature**: whoever can edit the receipt and
//! recompute the hashes can forge one. No cell values are written, and step settings only when
//! "include full steps" is asked for (they can hold data). Verifying never opens a path
//! written inside a receipt: only files the person names, and the receipt beside each of them.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::document::{Document, Progress};
use crate::hashing::{
    canonical_json, check_seal, file_sha256, parse_strict, seal, sha256_hex, SealKind,
};
use crate::Error;

/// The receipt format this release writes and reads.
pub const RECEIPT_VERSION: u64 = 1;
/// Largest receipt read, in bytes.
pub const MAX_RECEIPT_BYTES: usize = 1 << 20;
/// The field that holds the seal.
pub const SEAL_FIELD: &str = "receipt_sha256";

/// The receipt beside `file`: `<file>.ndreceipt.json`.
pub fn receipt_path(file: &Path) -> PathBuf {
    let mut name = file
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".ndreceipt.json");
    file.with_file_name(name)
}

/// Seconds since 1970 as an RFC 3339 UTC time (`2026-11-26T14:05:09Z`).
pub fn utc_text(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

/// One file in a receipt.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileRecord {
    /// The file's name (no folder).
    pub name: String,
    /// The full path, only when asked for (a path can name a person).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub bytes: u64,
    /// SHA-256 of the file's bytes; `None` when the file changed while it was read, so no hash
    /// is claimed for it.
    pub sha256: Option<String>,
    /// Rows and columns, when known (the shown data rows read or written).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cols: Option<u64>,
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl FileRecord {
    /// A file as it is on disk now (an output, after its save).
    pub fn of_file(path: &Path, full_path: bool, progress: &Progress) -> Result<FileRecord, Error> {
        let before = std::fs::metadata(path)?.len();
        let (sha, n) = file_sha256(path, progress)?;
        let after = std::fs::metadata(path)?.len();
        Ok(FileRecord {
            name: file_name(path),
            path: full_path.then(|| path.to_string_lossy().into_owned()),
            bytes: n,
            sha256: (n == before && n == after).then_some(sha),
            rows: None,
            cols: None,
        })
    }

    /// The file `doc` was read from, hashed so the hash covers the bytes that were parsed: a
    /// mapped file (locked against writers) is hashed in its mapping; a file read into memory
    /// is read again and its hash given only when it still matches what was read when it
    /// opened (same stamp, same content check). Otherwise `sha256` is `None`.
    pub fn of_input(
        doc: &Document,
        full_path: bool,
        progress: &Progress,
    ) -> Result<Option<FileRecord>, Error> {
        let Some(path) = doc.path().map(Path::to_path_buf) else {
            return Ok(None);
        };
        let (sha, bytes) = doc.source_sha256(progress)?;
        Ok(Some(FileRecord {
            name: file_name(&path),
            path: full_path.then(|| path.to_string_lossy().into_owned()),
            bytes,
            sha256: sha,
            rows: None,
            cols: None,
        }))
    }
}

/// What ran, without settings unless asked: step kinds in order, the column titles the steps
/// use, and the SHA-256 of the steps' canonical JSON (left out for private files, and when the
/// steps cannot be put in canonical form).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct StepsRecord {
    pub kinds: Vec<String>,
    pub titles: Vec<String>,
    pub sha256: Option<String>,
    /// The steps themselves, only with "include full steps" (they can hold data).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full: Option<Value>,
}

impl StepsRecord {
    /// `steps` as JSON (a list of objects with an `op` field, as recipes keep them).
    pub fn of(steps: &Value, titles: Vec<String>, private: bool, full: bool) -> StepsRecord {
        let kinds = steps
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|s| {
                        s.get("op")
                            .and_then(Value::as_str)
                            .unwrap_or("step")
                            .to_string()
                    })
                    .collect()
            })
            .unwrap_or_default();
        StepsRecord {
            kinds,
            titles: if private { Vec::new() } else { titles },
            sha256: if private {
                None
            } else {
                canonical_json(steps).ok().map(|b| sha256_hex(&b))
            },
            full: (full && !private).then(|| steps.clone()),
        }
    }
}

/// A receipt being made: started when the run starts, finished and written after the output's save.
#[derive(Debug, Clone)]
pub struct Receipt {
    pub started: SystemTime,
    pub action: String,
    pub inputs: Vec<FileRecord>,
    pub outputs: Vec<FileRecord>,
    pub steps: Option<StepsRecord>,
    /// An input changed while the run read it (its hash is left out).
    pub input_changed_during_run: bool,
    /// The seal of the receipt beside the first input, when there is a sound one.
    pub previous: Option<String>,
    /// There was a receipt beside the input, but its seal did not check (or it could not be read).
    pub previous_unsound: bool,
}

impl Receipt {
    pub fn start(action: &str) -> Receipt {
        Receipt {
            started: SystemTime::now(),
            action: action.to_string(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            steps: None,
            input_changed_during_run: false,
            previous: None,
            previous_unsound: false,
        }
    }

    /// Add an input; looks for a receipt beside it to chain to (the first input's only).
    pub fn add_input(&mut self, record: FileRecord, at: &Path) {
        if record.sha256.is_none() {
            self.input_changed_during_run = true;
        }
        if self.inputs.is_empty() {
            match read_sealed(&receipt_path(at)) {
                Ok(Some(seal)) => self.previous = Some(seal),
                Ok(None) => {}
                Err(_) => self.previous_unsound = true,
            }
        }
        self.inputs.push(record);
    }

    /// The sealed receipt as JSON.
    pub fn finish(&self) -> Result<Value, Error> {
        let mut m = Map::new();
        m.insert("kind".into(), json!("natendo-receipt"));
        m.insert("version".into(), json!(RECEIPT_VERSION));
        m.insert(
            "tool".into(),
            json!(format!("Natendo Data Edit {}", crate::version())),
        );
        m.insert("action".into(), json!(self.action));
        m.insert("started".into(), json!(utc_text(self.started)));
        m.insert("finished".into(), json!(utc_text(SystemTime::now())));
        m.insert(
            "inputs".into(),
            serde_json::to_value(&self.inputs).map_err(|e| Error::Invalid(e.to_string()))?,
        );
        m.insert(
            "outputs".into(),
            serde_json::to_value(&self.outputs).map_err(|e| Error::Invalid(e.to_string()))?,
        );
        m.insert(
            "steps".into(),
            match &self.steps {
                Some(s) => serde_json::to_value(s).map_err(|e| Error::Invalid(e.to_string()))?,
                None => Value::Null,
            },
        );
        m.insert("previous_receipt_sha256".into(), json!(self.previous));
        if self.previous_unsound {
            m.insert("previous_receipt_unsound".into(), json!(true));
        }
        m.insert(
            "input_changed_during_run".into(),
            json!(self.input_changed_during_run),
        );
        seal(&mut m, SealKind::Receipt, SEAL_FIELD)?;
        Ok(Value::Object(m))
    }

    /// Write the receipt beside `output` (a temp file moved into place). Called after the
    /// output's own save; a failure here never undoes that save.
    pub fn write_beside(&self, output: &Path) -> Result<PathBuf, Error> {
        let v = self.finish()?;
        let target = receipt_path(output);
        let mut bytes = serde_json::to_vec_pretty(&v).map_err(|e| Error::Invalid(e.to_string()))?;
        bytes.extend_from_slice(b"\r\n");
        let (temp, mut file) = crate::save::create_temp(&target)?;
        let done = file.write_all(&bytes).and_then(|_| file.sync_all());
        drop(file);
        if let Err(e) = done.and_then(|_| crate::save::replace(&temp, &target)) {
            let _ = std::fs::remove_file(&temp);
            return Err(e.into());
        }
        Ok(target)
    }
}

/// The seal of a sound receipt at `path`: `None` when there is no file; an error when there is
/// one but it cannot be read or its seal does not check.
fn read_sealed(path: &Path) -> Result<Option<String>, Error> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if !meta.is_file() || meta.len() > MAX_RECEIPT_BYTES as u64 {
        return Err(Error::Invalid("not a receipt".into()));
    }
    let v = parse_strict(&std::fs::read(path)?, MAX_RECEIPT_BYTES)?;
    let Value::Object(m) = v else {
        return Err(Error::Invalid("not a receipt".into()));
    };
    if !check_seal(&m, SealKind::Receipt, SEAL_FIELD) {
        return Err(Error::Invalid(
            "the receipt's seal does not match its contents".into(),
        ));
    }
    Ok(m.get(SEAL_FIELD)
        .and_then(Value::as_str)
        .map(str::to_string))
}

/// One check made by [`verify`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VerifyLine {
    /// What was checked ("the receipt's seal", "input voters.csv", ...).
    pub what: String,
    pub ok: bool,
    pub why: String,
}

/// What [`verify`] found; `ok` when every line is.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VerifyReport {
    /// Every line is ok and at least one file was compared: a seal alone
    /// only shows the receipt was not changed, not that the files match.
    pub ok: bool,
    /// Files compared with the receipt's entries.
    pub compared: usize,
    /// The one-line answer, the same in the app and on the command line.
    pub verdict: String,
    pub lines: Vec<VerifyLine>,
    /// What the receipt says, for showing: action, tool, times, file names.
    pub summary: Value,
}

/// Check the receipt at `receipt`: its seal; each file in `inputs` / `outputs` (named by the
/// person, matched to the receipt's entries by name) against its size and hash; with `chain`,
/// the receipt beside each given input, whose seal must check and equal the receipt's
/// `previous_receipt_sha256`. Paths written inside the receipt are never opened.
pub fn verify(
    receipt: &Path,
    inputs: &[PathBuf],
    outputs: &[PathBuf],
    chain: bool,
    progress: &Progress,
) -> Result<VerifyReport, Error> {
    let meta = std::fs::metadata(receipt)?;
    if !meta.is_file() || meta.len() > MAX_RECEIPT_BYTES as u64 {
        return Err(Error::Invalid(format!(
            "a receipt is a file of at most {} KB",
            MAX_RECEIPT_BYTES >> 10
        )));
    }
    let v = parse_strict(&std::fs::read(receipt)?, MAX_RECEIPT_BYTES)?;
    let Value::Object(m) = v else {
        return Err(Error::Invalid("this is not a receipt".into()));
    };
    if m.get("kind") != Some(&json!("natendo-receipt")) {
        return Err(Error::Invalid("this is not a receipt".into()));
    }
    if m.get("version")
        .and_then(Value::as_u64)
        .is_none_or(|v| v > RECEIPT_VERSION)
    {
        return Err(Error::Invalid(
            "the receipt was written by a newer version of the app".into(),
        ));
    }
    let mut lines = vec![{
        let ok = check_seal(&m, SealKind::Receipt, SEAL_FIELD);
        VerifyLine {
            what: "the receipt's seal".into(),
            ok,
            why: if ok {
                "matches its contents".into()
            } else {
                "does not match its contents: the receipt was changed after it was written".into()
            },
        }
    }];
    let entries = |key: &str| -> Vec<Map<String, Value>> {
        m.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|x| x.as_object().cloned()).collect())
            .unwrap_or_default()
    };
    let mut compared = 0usize;
    for (key, label, given) in [("inputs", "input", inputs), ("outputs", "output", outputs)] {
        let list = entries(key);
        for f in given {
            let name = file_name(f);
            let what = format!("{label} {name}");
            // Names can repeat (two inputs called data.csv from two folders):
            // the file is compared with every entry of its name.
            let same_name: Vec<&Map<String, Value>> = list
                .iter()
                .filter(|e| e.get("name").and_then(Value::as_str) == Some(name.as_str()))
                .collect();
            if same_name.is_empty() {
                lines.push(VerifyLine {
                    what,
                    ok: false,
                    why: format!("the receipt names no {label} called {name}"),
                });
                continue;
            }
            compared += 1;
            let rec = FileRecord::of_file(f, false, progress)?;
            let wants: Vec<Option<&str>> = same_name
                .iter()
                .map(|e| e.get("sha256").and_then(Value::as_str))
                .collect();
            let (ok, why) = match rec.sha256.as_deref() {
                None => (false, "it changed while it was being checked".into()),
                Some(h) if wants.contains(&Some(h)) => (
                    true,
                    if same_name.len() > 1 {
                        format!(
                            "same bytes ({} bytes) as one of the {} {label}s of this name",
                            rec.bytes,
                            same_name.len()
                        )
                    } else {
                        format!("same bytes ({} bytes)", rec.bytes)
                    },
                ),
                Some(_) if same_name.len() > 1 => (
                    false,
                    format!(
                        "the receipt names {} {label}s called {name}, and this file has the bytes of none of them",
                        same_name.len()
                    ),
                ),
                Some(_) if wants[0].is_none() => (
                    false,
                    "the receipt holds no hash for it (it changed while the run read it)".into(),
                ),
                Some(_) => (
                    false,
                    format!(
                        "different bytes: {} bytes now, {} in the receipt",
                        rec.bytes,
                        same_name[0].get("bytes").and_then(Value::as_u64).unwrap_or(0)
                    ),
                ),
            };
            lines.push(VerifyLine { what, ok, why });
        }
    }
    if chain {
        let prev = m.get("previous_receipt_sha256").and_then(Value::as_str);
        if inputs.is_empty() {
            lines.push(VerifyLine {
                what: "the chain".into(),
                ok: false,
                why: "name the input file to check the receipt beside it".into(),
            });
        }
        for f in inputs {
            let beside = receipt_path(f);
            let what = format!("the receipt beside {}", file_name(f));
            let (ok, why) = match (read_sealed(&beside), prev) {
                (Ok(None), None) => (true, "none, and the receipt names none".into()),
                (Ok(None), Some(_)) => (
                    false,
                    "the receipt names an earlier receipt, but there is none beside the input"
                        .into(),
                ),
                (Ok(Some(s)), Some(p)) if s == p => (
                    true,
                    "its seal matches, and it is the one this receipt names".into(),
                ),
                (Ok(Some(_)), Some(_)) => (false, "it is not the receipt this one names".into()),
                (Ok(Some(_)), None) => (false, "there is one, but this receipt names none".into()),
                (Err(e), _) => (false, e.to_string()),
            };
            lines.push(VerifyLine { what, ok, why });
        }
    }
    let summary = json!({
        "action": m.get("action"),
        "tool": m.get("tool"),
        "started": m.get("started"),
        "finished": m.get("finished"),
        "inputs": entries("inputs").iter().map(|e| e.get("name").cloned().unwrap_or(Value::Null)).collect::<Vec<_>>(),
        "outputs": entries("outputs").iter().map(|e| e.get("name").cloned().unwrap_or(Value::Null)).collect::<Vec<_>>(),
        "steps": m.get("steps").and_then(|s| s.get("kinds")).cloned().unwrap_or(Value::Null),
        "input_changed_during_run": m.get("input_changed_during_run"),
    });
    let all_ok = lines.iter().all(|l| l.ok);
    let sealed = lines.first().is_some_and(|l| l.ok);
    let verdict = if all_ok && compared > 0 {
        "The receipt checks out: its seal matches and the files compared are the ones it names."
    } else if sealed && compared == 0 && all_ok {
        "The seal matches, but no files were compared: name the files read or written to check them."
    } else {
        "The receipt does not check out."
    };
    Ok(VerifyReport {
        ok: all_ok && compared > 0,
        compared,
        verdict: verdict.into(),
        lines,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenOptions;

    struct Folder(PathBuf);
    impl Folder {
        fn new(name: &str) -> Folder {
            let d = std::env::temp_dir().join(format!("nd-receipt-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Folder(d)
        }
        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let p = self.0.join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        }
    }
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn open(p: &Path) -> Document {
        Document::open_indexed(p, OpenOptions::default(), None).unwrap()
    }

    /// Run "a step" from `input` to `output` and write a receipt beside the output.
    fn run(input: &Path, output: &Path, private: bool) -> PathBuf {
        let p = Progress::new();
        let mut r = Receipt::start("run recipe");
        let d = open(input);
        let rec = FileRecord::of_input(&d, false, &p).unwrap().unwrap();
        r.add_input(rec, input);
        drop(d);
        std::fs::copy(input, output).unwrap();
        r.outputs
            .push(FileRecord::of_file(output, false, &p).unwrap());
        r.steps = Some(StepsRecord::of(
            &json!([{"op": "replace", "find": "Ann Smith", "with": "x"}]),
            vec!["name".into()],
            private,
            false,
        ));
        r.write_beside(output).unwrap()
    }

    #[test]
    fn utc_times() {
        assert_eq!(utc_text(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        let t = UNIX_EPOCH + std::time::Duration::from_secs(1_795_704_309);
        assert_eq!(utc_text(t), "2026-11-26T14:45:09Z");
        let leap = UNIX_EPOCH + std::time::Duration::from_secs(951_825_600); // 2000-02-29 12:00
        assert_eq!(utc_text(leap), "2000-02-29T12:00:00Z");
    }

    #[test]
    fn written_receipts_verify_and_any_change_shows() {
        let f = Folder::new("verify");
        let input = f.write("in.csv", b"name,n\r\nAnn,1\r\nBo,2\r\n");
        let output = f.0.join("out.csv");
        let receipt = run(&input, &output, false);
        assert_eq!(receipt, f.0.join("out.csv.ndreceipt.json"));
        let p = Progress::new();
        let text = std::fs::read_to_string(&receipt).unwrap();
        // No cell values and no step settings by default; file names only.
        assert!(
            !text.contains("Ann") && !text.contains(&*f.0.to_string_lossy()),
            "{text}"
        );
        assert!(text.contains("\"kinds\"") && text.contains("replace"));
        let ok = verify(
            &receipt,
            std::slice::from_ref(&input),
            std::slice::from_ref(&output),
            false,
            &p,
        )
        .unwrap();
        assert!(ok.ok, "{:?}", ok.lines);
        assert_eq!(ok.lines.len(), 3);
        // The output changed: that line fails, the seal still holds.
        std::fs::write(&output, b"name,n\r\nAnn,9\r\nBo,2\r\n").unwrap();
        let bad = verify(&receipt, &[], std::slice::from_ref(&output), false, &p).unwrap();
        assert!(
            !bad.ok && bad.lines[0].ok && !bad.lines[1].ok,
            "{:?}",
            bad.lines
        );
        // A file the receipt does not name.
        let other = f.write("other.csv", b"x\r\n");
        assert!(!verify(&receipt, &[other], &[], false, &p).unwrap().ok);
        // The receipt edited by hand: the seal fails. Reordered keys alone still verify.
        let v: Value = serde_json::from_str(&text).unwrap();
        let mut m = v.as_object().unwrap().clone();
        m.insert("action".into(), json!("something else"));
        std::fs::write(&receipt, serde_json::to_vec(&Value::Object(m)).unwrap()).unwrap();
        let r = verify(&receipt, &[], &[], false, &p).unwrap();
        assert!(!r.ok && !r.lines[0].ok);
        let mut keys: Vec<(String, Value)> = v.as_object().unwrap().clone().into_iter().collect();
        keys.reverse();
        let reordered: Map<String, Value> = keys.into_iter().collect();
        std::fs::write(
            &receipt,
            serde_json::to_vec(&Value::Object(reordered)).unwrap(),
        )
        .unwrap();
        assert!(verify(&receipt, &[], &[], false, &p).unwrap().lines[0].ok);
    }

    #[test]
    fn a_seal_alone_is_not_a_pass_and_repeated_names_are_all_compared() {
        let f = Folder::new("names");
        let input = f.write("in.csv", b"n\r\n1\r\n");
        let output = f.0.join("out.csv");
        let receipt = run(&input, &output, false);
        let p = Progress::new();
        // F1: no files named: the seal holds, but that is not "checks out".
        let r = verify(&receipt, &[], &[], false, &p).unwrap();
        assert!(r.lines[0].ok && !r.ok && r.compared == 0, "{r:?}");
        assert!(r
            .verdict
            .starts_with("The seal matches, but no files were compared"));
        let r = verify(&receipt, std::slice::from_ref(&input), &[], false, &p).unwrap();
        assert!(
            r.ok && r.compared == 1 && r.verdict.contains("checks out"),
            "{r:?}"
        );
        // F2: two inputs with the same name from two folders.
        std::fs::create_dir_all(f.0.join("x")).unwrap();
        std::fs::create_dir_all(f.0.join("y")).unwrap();
        let x = f.write("x/data.csv", b"n\r\n1\r\n");
        let y = f.write("y/data.csv", b"n\r\n2\r\n");
        let mut rec = Receipt::start("combine");
        for at in [&x, &y] {
            rec.inputs.push(FileRecord::of_file(at, false, &p).unwrap());
        }
        let out2 = f.write("both.csv", b"n\r\n1\r\n2\r\n");
        rec.outputs
            .push(FileRecord::of_file(&out2, false, &p).unwrap());
        let both = rec.write_beside(&out2).unwrap();
        // The second file of that name matches its own entry, not only the first.
        for at in [&x, &y] {
            let r = verify(&both, std::slice::from_ref(at), &[], false, &p).unwrap();
            assert!(r.ok, "{:?}", r.lines);
            assert!(
                r.lines[1].why.contains("one of the 2 inputs"),
                "{:?}",
                r.lines
            );
        }
        // A third data.csv with other bytes: says the names repeat.
        std::fs::create_dir_all(f.0.join("z")).unwrap();
        let z = f.write("z/data.csv", b"n\r\n3\r\n");
        let r = verify(&both, std::slice::from_ref(&z), &[], false, &p).unwrap();
        assert!(!r.ok, "{:?}", r.lines);
        assert!(
            r.lines[1].why.contains("names 2 inputs called data.csv"),
            "{:?}",
            r.lines
        );
    }

    #[test]
    fn receipts_chain_through_the_receipt_beside_the_input() {
        let f = Folder::new("chain");
        let a = f.write("a.csv", b"n\r\n1\r\n");
        let b = f.0.join("b.csv");
        let c = f.0.join("c.csv");
        let first = run(&a, &b, false);
        let second = run(&b, &c, false);
        let p = Progress::new();
        let r = verify(&second, std::slice::from_ref(&b), &[], true, &p).unwrap();
        assert!(r.ok, "{:?}", r.lines);
        // The earlier receipt changed: the chain breaks.
        let mut v: Value = serde_json::from_slice(&std::fs::read(&first).unwrap()).unwrap();
        v["action"] = json!("edited");
        std::fs::write(&first, serde_json::to_vec(&v).unwrap()).unwrap();
        let r = verify(&second, std::slice::from_ref(&b), &[], true, &p).unwrap();
        assert!(!r.ok);
        assert!(
            r.lines.last().unwrap().why.contains("seal"),
            "{:?}",
            r.lines
        );
        // A new run from b now records that the receipt beside its input is unsound.
        let d = f.0.join("d.csv");
        let third = run(&b, &d, false);
        let t = std::fs::read_to_string(third).unwrap();
        assert!(t.contains("\"previous_receipt_unsound\": true"), "{t}");
        // With no input named, the chain cannot be checked.
        assert!(!verify(&second, &[], &[], true, &p).unwrap().ok);
    }

    #[test]
    fn paths_written_in_a_receipt_are_never_opened() {
        let f = Folder::new("paths");
        let input = f.write("in.csv", b"n\r\n1\r\n");
        let output = f.0.join("out.csv");
        let receipt = run(&input, &output, false);
        // Names and paths pointing at a share, a device and a huge file: verify reads none of them.
        let mut v: Value = serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
        v["inputs"][0]["path"] = json!(r"\\server\share\in.csv");
        v["outputs"][0]["path"] = json!(r"\\.\PhysicalDrive0");
        v["inputs"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "CON", "path": "/dev/zero", "bytes": 1, "sha256": null}));
        let mut m = v.as_object().unwrap().clone();
        seal(&mut m, SealKind::Receipt, SEAL_FIELD).unwrap();
        std::fs::write(&receipt, serde_json::to_vec(&Value::Object(m)).unwrap()).unwrap();
        let started = std::time::Instant::now();
        let r = verify(&receipt, &[], &[], true, &Progress::new()).unwrap();
        assert!(started.elapsed().as_secs() < 5);
        assert!(r.lines[0].ok, "the seal (re-made here) holds");
        assert!(!r.ok, "no input named for the chain");
        // Only names are matched to the files the person gave.
        let r = verify(
            &receipt,
            std::slice::from_ref(&input),
            std::slice::from_ref(&output),
            false,
            &Progress::new(),
        )
        .unwrap();
        assert!(r.ok, "{:?}", r.lines);
        // A receipt that is too large, or not a receipt, is refused before reading it.
        let big = f.write("big.ndreceipt.json", &vec![b' '; MAX_RECEIPT_BYTES + 1]);
        assert!(verify(&big, &[], &[], false, &Progress::new()).is_err());
        let not = f.write("not.json", b"{\"kind\":\"something\"}");
        assert!(verify(&not, &[], &[], false, &Progress::new()).is_err());
        let patch_sealed = {
            let mut m = Map::new();
            m.insert("kind".into(), json!("natendo-receipt"));
            m.insert("version".into(), json!(1));
            seal(&mut m, SealKind::Patch, SEAL_FIELD).unwrap();
            f.write(
                "patch.json",
                &serde_json::to_vec(&Value::Object(m)).unwrap(),
            )
        };
        assert!(
            !verify(&patch_sealed, &[], &[], false, &Progress::new())
                .unwrap()
                .lines[0]
                .ok,
            "a patch's seal is not a receipt's"
        );
    }

    #[test]
    fn an_input_that_changed_since_it_was_read_gets_no_hash() {
        let f = Folder::new("changed");
        let input = f.write("in.csv", b"n\r\n1\r\n2\r\n");
        let d = open(&input);
        let p = Progress::new();
        let (h, n) = d.source_sha256(&p).unwrap();
        assert_eq!(h.as_deref(), Some(sha256_hex(b"n\r\n1\r\n2\r\n").as_str()));
        assert_eq!(n, 9);
        // Grows while the run goes on.
        std::fs::write(&input, b"n\r\n1\r\n2\r\n3\r\n").unwrap();
        let (h, _) = d.source_sha256(&p).unwrap();
        assert!(h.is_none());
        let mut r = Receipt::start("save");
        r.add_input(
            FileRecord::of_input(&d, false, &p).unwrap().unwrap(),
            &input,
        );
        assert!(r.input_changed_during_run);
        let v = r.finish().unwrap();
        assert_eq!(v["input_changed_during_run"], json!(true));
        assert_eq!(v["inputs"][0]["sha256"], Value::Null);
        // Same length, other bytes (and the time put back): the content check catches it.
        let g = f.write("same.csv", b"n\r\n1\r\n");
        let d = open(&g);
        let when = std::fs::metadata(&g).unwrap().modified().unwrap();
        std::fs::write(&g, b"n\r\n9\r\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&g)
            .unwrap()
            .set_modified(when)
            .unwrap();
        assert!(d.source_sha256(&p).unwrap().0.is_none());
        // Cancel gives no hash.
        let stop = Progress::new();
        stop.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let h = f.write("h.csv", b"n\r\n1\r\n");
        assert!(matches!(
            open(&h).source_sha256(&stop),
            Err(Error::Cancelled)
        ));
    }

    #[test]
    fn private_files_keep_titles_and_the_step_hash_out() {
        let steps = json!([{"op": "replace", "find": "Ann Smith", "with": ""}]);
        let open = StepsRecord::of(&steps, vec!["name".into()], false, false);
        assert_eq!(open.kinds, vec!["replace"]);
        assert!(open.sha256.is_some() && open.full.is_none());
        let private = StepsRecord::of(&steps, vec!["name".into()], true, true);
        assert!(private.titles.is_empty() && private.sha256.is_none() && private.full.is_none());
        let full = StepsRecord::of(&steps, vec![], false, true);
        assert_eq!(full.full, Some(steps.clone()));
        // Steps that cannot be put in canonical form (a fraction) get no hash, not a wrong one.
        let frac = StepsRecord::of(&json!([{"op": "round", "by": 0.5}]), vec![], false, false);
        assert!(frac.sha256.is_none());
    }

    #[test]
    fn a_receipt_that_cannot_be_written_says_so() {
        let f = Folder::new("fail");
        let r = Receipt::start("save");
        assert!(r
            .write_beside(&f.0.join("no such folder").join("out.csv"))
            .is_err());
    }
}
