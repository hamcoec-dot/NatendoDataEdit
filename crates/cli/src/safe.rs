//! `share-safe`, `encrypt` and `decrypt`: the app's share-safe copy and
//! password-protected files. Every file is written beside its target and
//! moved into place only when complete, so a wrong password, an error or a
//! failed write leaves no output file.

use std::io::{Read, Write};
use std::path::Path;

use natendo_engine::crypt::{self, MIN_PASSWORD_CHARS};
use natendo_engine::privacy::{wipe, ColumnAction, SafeCopy, MIN_PHRASE_CHARS};
use natendo_engine::value::ReadOptions;
use natendo_engine::{Document, Progress};
use serde::Deserialize;
use serde_json::json;

use crate::more::load;
use crate::secret::{self, Kind, ALLOW_IN_FILE};
use crate::tools::{done, one_input, opts, sheet};

/// Largest share-safe spec read.
const MAX_SPEC_BYTES: u64 = 4 << 20;

fn same_file(a: &str, b: &str) -> bool {
    matches!(
        (std::fs::canonicalize(a), std::fs::canonicalize(b)),
        (Ok(x), Ok(y)) if x == y
    )
}

/// Refuse an output that is one of the inputs, or that exists without
/// `--overwrite`.
pub(crate) fn check_out(out: &str, inputs: &[&str], overwrite: bool) -> Result<(), String> {
    if inputs.iter().any(|i| same_file(i, out)) {
        return Err("write to a different file than the input".into());
    }
    if Path::new(out).exists() && !overwrite {
        return Err(format!(
            "{out} already exists; add --overwrite to replace it"
        ));
    }
    Ok(())
}

/// Read a small file whole, refusing one past `max` bytes.
pub(crate) fn read_small(path: &str, max: u64, what: &str) -> Result<Vec<u8>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let mut bytes = Vec::new();
    f.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{path}: {e}"))?;
    if bytes.len() as u64 > max {
        return Err(format!("{path}: {what} can be at most {} MB", max >> 20));
    }
    Ok(bytes)
}

fn yes() -> bool {
    true
}

/// A share-safe spec as a file: each column found by its title or its
/// number (from 1), never by place alone.
#[derive(Deserialize)]
struct SpecFile {
    /// `None`: from `--no-header` (titles by default).
    #[serde(default)]
    header: Option<bool>,
    #[serde(default)]
    read: ReadOptions,
    #[serde(default = "yes")]
    guard: bool,
    columns: Vec<SpecColumn>,
}

#[derive(Deserialize)]
struct SpecColumn {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    column: Option<u64>,
    #[serde(flatten)]
    action: ColumnAction,
}

impl SpecColumn {
    fn name(&self) -> String {
        match (&self.title, self.column) {
            (Some(t), _) => format!("\"{t}\""),
            (None, Some(n)) => format!("column {n}"),
            (None, None) => "a column with no title or number".into(),
        }
    }
}

/// One action per column of the file, in order (P1: fails closed). Refused,
/// naming the columns, when a spec column is not in the file, when two
/// columns of the file share a title the spec uses, when a column is given
/// two actions, or when the file has a column the spec says nothing about.
fn resolve(
    d: &Document,
    header: bool,
    spec: &[SpecColumn],
    spec_path: &str,
    input: &str,
) -> Result<Vec<ColumnAction>, String> {
    let n = d.col_count() as usize;
    let all: Vec<u32> = (0..n as u32).collect();
    let titles = d.titles(&all, header);
    let mut actions: Vec<Option<ColumnAction>> = vec![None; n];
    let (mut missing, mut shared, mut twice) = (Vec::new(), Vec::new(), Vec::new());
    for sc in spec {
        let at: Vec<usize> = match (&sc.title, sc.column) {
            (Some(t), None) => {
                if !header {
                    return Err(format!(
                        "{spec_path}: {input} has no title row here, so give its columns by \"column\" number"
                    ));
                }
                let t = t.trim();
                let exact: Vec<usize> = (0..n).filter(|&i| titles[i].trim() == t).collect();
                if exact.is_empty() {
                    (0..n)
                        .filter(|&i| titles[i].trim().eq_ignore_ascii_case(t))
                        .collect()
                } else {
                    exact
                }
            }
            (None, Some(k)) if k >= 1 && k as usize <= n => vec![k as usize - 1],
            (None, Some(_)) => Vec::new(),
            _ => {
                return Err(format!(
                "{spec_path}: each column needs a \"title\" or a \"column\" number (one of them)"
            ))
            }
        };
        match at.as_slice() {
            [] => missing.push(sc.name()),
            [i] if actions[*i].is_some() => twice.push(sc.name()),
            [i] => actions[*i] = Some(sc.action.clone()),
            _ => shared.push(sc.name()),
        }
    }
    let left: Vec<String> = (0..n)
        .filter(|&i| actions[i].is_none())
        .map(|i| {
            if header {
                format!("\"{}\" (column {})", titles[i], i + 1)
            } else {
                format!("column {}", i + 1)
            }
        })
        .collect();
    let mut problems = Vec::new();
    if !missing.is_empty() {
        problems.push(format!("not in the file: {}", missing.join(", ")));
    }
    if !shared.is_empty() {
        problems.push(format!(
            "more than one column of the file has this title: {}",
            shared.join(", ")
        ));
    }
    if !twice.is_empty() {
        problems.push(format!("given two actions: {}", twice.join(", ")));
    }
    if !left.is_empty() {
        problems.push(format!(
            "the file has columns the spec gives no action (keep, drop or clean) for: {}",
            left.join(", ")
        ));
    }
    if !problems.is_empty() {
        return Err(format!(
            "{spec_path} does not fit {input}: {}. Nothing was written",
            problems.join("; ")
        ));
    }
    Ok(actions.into_iter().flatten().collect())
}

/// `share-safe <in> -o <out> --spec <spec.json>`: the app's share-safe copy.
pub fn share_safe(args: &[&str]) -> Result<(), String> {
    let (phrase, args) = secret::take(args, Kind::Phrase, MIN_PHRASE_CHARS)?;
    let (allow, args) = secret::take_flag(&args, ALLOW_IN_FILE);
    let o = opts(&args, &["--spec"], &[])?;
    let input = one_input(&o)?;
    let out = o.get("-o").ok_or("give the output file with -o")?;
    let spec_path = o
        .get("--spec")
        .ok_or("give the share-safe spec with --spec <spec.json>")?;
    if o.get("--to").is_some() {
        return Err(
            "a share-safe copy is written in the file's own separator; convert it afterwards"
                .into(),
        );
    }
    check_out(out, &[input, spec_path], o.has("--overwrite"))?;
    let bytes = read_small(spec_path, MAX_SPEC_BYTES, "a share-safe spec")?;
    secret::refuse_secret_in_file(spec_path, &bytes, allow)?;
    let file: SpecFile = serde_json::from_slice(&bytes)
        .map_err(|e| format!("{spec_path}: this is not a share-safe spec: {e}"))?;
    let header = file.header.unwrap_or(o.header());
    let d = load(input, sheet(&o)?)?;
    let mut columns = resolve(&d, header, &file.columns, spec_path, input)?;
    if columns.len() > natendo_engine::document::MAX_MAPPED_COLS as usize {
        return Err("too many columns".into());
    }
    let mut scrambles = 0;
    for c in &mut columns {
        if let ColumnAction::Clean { cleaner } = c {
            if let Some(p) = &phrase {
                scrambles += secret::fill_scramble(cleaner, p);
            }
            if secret::scramble_without_phrase(cleaner) {
                return Err(format!(
                    "{spec_path} scrambles a column: give the phrase with --phrase-stdin, --phrase-file <file> or {}",
                    Kind::Phrase.var()
                ));
            }
            cleaner.check().map_err(|e| format!("{spec_path}: {e}"))?;
        }
    }
    if phrase.is_some() && scrambles == 0 {
        return Err(format!(
            "{spec_path} scrambles nothing: leave the phrase out"
        ));
    }
    let spec = SafeCopy {
        columns,
        header,
        read: file.read,
        guard: file.guard,
    };
    let r = d
        .save_safe_copy(
            Path::new(out),
            &spec,
            natendo_engine::clean::now_utc(),
            &Progress::new(),
        )
        .map_err(|e| format!("{out}: {e}"))?;
    let msg = format!(
        "wrote a share-safe copy of {} rows and {} columns to {out}: {} values changed, {} written blank, {} given an apostrophe",
        r.rows, r.cols, r.changed, r.blanked, r.guarded
    );
    done(
        &o,
        msg,
        json!({
            "rows": r.rows,
            "cols": r.cols,
            "changed": r.changed,
            "blanked": r.blanked,
            "guarded": r.guarded,
            "bytes": r.bytes,
            "file": out,
        }),
    );
    Ok(())
}

/// `encrypt <in> -o <out.ndenc>`: the shown rows as an encrypted CSV, the
/// way the app's Save encrypted writes it.
pub fn encrypt(args: &[&str]) -> Result<(), String> {
    let (password, args) = secret::take(args, Kind::Password, MIN_PASSWORD_CHARS)?;
    let o = opts(&args, &[], &[])?;
    let input = one_input(&o)?;
    let out = o.get("-o").ok_or("give the output file with -o")?;
    let password = password.ok_or_else(|| {
        format!(
            "give the password with --password-stdin, --password-file <file> or {}",
            Kind::Password.var()
        )
    })?;
    if o.get("--to").is_some() {
        return Err("an encrypted file always holds CSV".into());
    }
    if !Path::new(out)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(crypt::EXTENSION))
    {
        return Err(format!(
            "an encrypted file's name ends in .{}",
            crypt::EXTENSION
        ));
    }
    check_out(out, &[input], o.has("--overwrite"))?;
    let d = load(input, sheet(&o)?)?;
    let rows = d
        .save_encrypted(Path::new(out), &password, &Progress::new())
        .map_err(|e| format!("{out}: {e}"))?;
    done(
        &o,
        format!("encrypted {rows} rows to {out}"),
        json!({ "rows": rows, "file": out }),
    );
    Ok(())
}

/// `decrypt <in.ndenc> -o <out>`: the whole file is decrypted and checked in
/// memory first; only then is the plain CSV written (beside the target,
/// moved into place).
pub fn decrypt(args: &[&str]) -> Result<(), String> {
    let (password, args) = secret::take(args, Kind::Password, 1)?;
    let o = opts(&args, &[], &[])?;
    let input = one_input(&o)?;
    let out = o.get("-o").ok_or("give the output file with -o")?;
    let password = password.ok_or_else(|| {
        format!(
            "give the password with --password-stdin, --password-file <file> or {}",
            Kind::Password.var()
        )
    })?;
    if o.get("--to").is_some() {
        return Err("decrypt writes the CSV the file holds; convert it afterwards".into());
    }
    check_out(out, &[input], o.has("--overwrite"))?;
    let size = std::fs::metadata(input)
        .map_err(|e| format!("{input}: {e}"))?
        .len();
    if size > (crypt::MAX_PLAIN as u64) / 100 * 101 {
        return Err(format!("{input}: the file is too large to decrypt"));
    }
    let data = std::fs::read(input).map_err(|e| format!("{input}: {e}"))?;
    if !crypt::is_encrypted(&data) {
        return Err(format!("{input} is not an encrypted file (.ndenc)"));
    }
    let mut plain = crypt::decrypt(&data, &password).map_err(|e| format!("{input}: {e}"))?;
    let written = natendo_engine::save::write_atomic(Path::new(out), |w| {
        w.write_all(&plain)?;
        Ok(())
    });
    let bytes = plain.len();
    wipe(&mut plain);
    written.map_err(|e| format!("{out}: {e}"))?;
    done(
        &o,
        format!("decrypted {bytes} bytes to {out}"),
        json!({ "bytes": bytes, "file": out }),
    );
    Ok(())
}
