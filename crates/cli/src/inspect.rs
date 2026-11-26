//! `xray` (hidden and look-alike characters, and `--sanitize` to fix them),
//! `encoding detect` / `encoding scan`, and `private` (columns that look
//! like private data). Reports give counts, column titles and row numbers,
//! never cell text.

use std::io::Read;

use natendo_engine::clean::{CleanSpec, Cleaner, Step, CLEANER_VERSION};
use natendo_engine::encoding::{detect, DETECT_SAMPLE};
use natendo_engine::find::Scope;
use natendo_engine::xray::SanitizeOptions;
use natendo_engine::{Document, OpenOptions, Progress};
use serde_json::{json, Value};

use crate::more::{load, write_out};
use crate::tools::{cols_of, done, one_input, opts, out_of, sheet};

/// Row numbers listed at most.
const MAX_ROWS: usize = 1_000;

/// The first rows in a report's row set, numbered as the app numbers them
/// (the title row is 1).
fn row_numbers(d: &Document, set: &[u64]) -> Vec<u64> {
    d.shown_rows_in(set, MAX_ROWS)
        .into_iter()
        .map(|r| r + 1)
        .collect()
}

fn kind_name<T: serde::Serialize>(k: &T) -> Value {
    serde_json::to_value(k).unwrap_or(Value::Null)
}

const FIX_FLAGS: [&str; 5] = [
    "--zero-width",
    "--odd-spaces",
    "--smart-quotes",
    "--tabs-breaks",
    "--homoglyphs",
];

/// `xray <in>`: `Ok(false)` when something was found and `--fail-on-found`.
pub fn xray(args: &[&str]) -> Result<bool, String> {
    let mut flags = vec!["--fail-on-found", "--sanitize"];
    flags.extend(FIX_FLAGS);
    let o = opts(args, &["--columns"], &flags)?;
    let input = one_input(&o)?;
    let header = o.header();
    let picked: Vec<&str> = FIX_FLAGS.iter().copied().filter(|f| o.has(f)).collect();
    if o.has("--sanitize") {
        return sanitize(&o, input, header, &picked).map(|()| true);
    }
    if let Some(f) = picked.first() {
        return Err(format!("{f} goes with --sanitize"));
    }
    if o.get("-o").is_some() {
        return Err("-o goes with --sanitize (the report is printed)".into());
    }
    let d = load(input, sheet(&o)?)?;
    let cols = match o.get("--columns") {
        Some(l) => cols_of(&d, header, l)?,
        None => Vec::new(),
    };
    let r = d
        .xray_report(&cols, header, &Progress::new())
        .map_err(|e| e.to_string())?;
    let all: Vec<u32> = (0..d.col_count()).collect();
    let titles = d.titles(&all, header);
    let rows = row_numbers(&d, &r.failing);
    let mut msg = format!(
        "{} cells in {} rows hold hidden or look-alike characters ({} cells looked at)",
        r.cells, r.rows, r.looked
    );
    for (k, n) in &r.kinds {
        msg += &format!("\n  {}: {n} cells", kind_name(k).as_str().unwrap_or("?"));
    }
    for c in &r.columns {
        msg += &format!(
            "\n  column {} {}: {} cells",
            c.col + 1,
            titles[c.col as usize],
            c.cells
        );
    }
    if !rows.is_empty() {
        let list: Vec<String> = rows.iter().take(20).map(u64::to_string).collect();
        msg += &format!(
            "\n  rows: {}{}",
            list.join(", "),
            if rows.len() > 20 { ", ..." } else { "" }
        );
    }
    let kinds = |ks: &[(natendo_engine::xray::GremlinKind, u64)]| -> Vec<Value> {
        ks.iter()
            .map(|(k, n)| json!({ "kind": kind_name(k), "cells": n }))
            .collect()
    };
    done(
        &o,
        msg,
        json!({
            "looked": r.looked,
            "cells": r.cells,
            "rows": r.rows,
            "kinds": kinds(&r.kinds),
            "columns": r.columns.iter().map(|c| json!({
                "column": titles[c.col as usize],
                "number": c.col + 1,
                "cells": c.cells,
                "kinds": kinds(&c.kinds),
            })).collect::<Vec<_>>(),
            "first_rows": rows,
        }),
    );
    Ok(!(o.has("--fail-on-found") && r.cells > 0))
}

/// `xray --sanitize`: the Sanitize clean-up step (the app's X-ray "Fix"),
/// run like `natendo-cli clean`, written to `-o`.
fn sanitize(
    o: &crate::tools::Opts,
    input: &str,
    header: bool,
    picked: &[&str],
) -> Result<(), String> {
    let options = if picked.is_empty() {
        SanitizeOptions::usual()
    } else {
        SanitizeOptions {
            zero_width: picked.contains(&"--zero-width"),
            odd_spaces: picked.contains(&"--odd-spaces"),
            smart_quotes: picked.contains(&"--smart-quotes"),
            tabs_breaks: picked.contains(&"--tabs-breaks"),
            homoglyphs: picked.contains(&"--homoglyphs"),
        }
    };
    let out = out_of(o)?;
    crate::safe::check_out(out.path, &[input], out.overwrite)?;
    let mut d = load(input, sheet(o)?)?;
    let cols = match o.get("--columns") {
        Some(l) => cols_of(&d, header, l)?,
        None => Vec::new(),
    };
    let spec = CleanSpec {
        cleaner: Cleaner {
            version: CLEANER_VERSION,
            name: "Sanitize".into(),
            steps: vec![Step::Sanitize { options }],
        },
        scope: if cols.is_empty() {
            Scope::All
        } else {
            Scope::Columns { cols }
        },
        skip_first: header,
        read: Default::default(),
    };
    let plan = d
        .plan_clean(
            &spec,
            natendo_engine::clean::now_utc(),
            1,
            0,
            &Progress::new(),
        )
        .map_err(|e| e.to_string())?;
    let changed = plan.changed;
    d.apply_clean(plan).map_err(|e| e.to_string())?;
    let w = write_out(&mut d, input, &out)?;
    done(
        o,
        format!("{changed} cells sanitized; {w}"),
        json!({ "changed": changed, "rows": d.row_count(), "file": out.path }),
    );
    Ok(())
}

/// `encoding detect <in>`: what the app reads the file as.
pub fn detect_cmd(args: &[&str]) -> Result<(), String> {
    let o = opts(args, &[], &[])?;
    let input = one_input(&o)?;
    crate::refuse_encrypted(input)?;
    let f = std::fs::File::open(input).map_err(|e| format!("{input}: {e}"))?;
    let mut sample = Vec::new();
    f.take(DETECT_SAMPLE as u64)
        .read_to_end(&mut sample)
        .map_err(|e| format!("{input}: {e}"))?;
    let found = detect(&sample);
    let name = found.encoding.name();
    let mut msg = format!(
        "{name}{}",
        if found.bom_len > 0 { " (with BOM)" } else { "" }
    );
    if found.invalid_bytes {
        msg += "\nnote: a few bytes are not valid UTF-8 (they read as replacement characters)";
    }
    done(
        &o,
        msg,
        json!({
            "encoding": name,
            "bom": found.bom_len > 0,
            "invalid_bytes": found.invalid_bytes,
            "sample_bytes": sample.len(),
        }),
    );
    Ok(())
}

/// `encoding scan <in> [--as <label>]`: rows that do not read cleanly.
/// `Ok(false)` when some were found and `--fail-on-found`.
pub fn scan(args: &[&str]) -> Result<bool, String> {
    let o = opts(args, &["--as"], &["--fail-on-found"])?;
    let input = one_input(&o)?;
    let header = o.header();
    let d = match o.get("--as") {
        Some(label) => {
            crate::refuse_encrypted(input)?;
            let opts = OpenOptions {
                encoding: Some(crate::encoding_named(label)?),
                ..Default::default()
            };
            Document::open_indexed(std::path::Path::new(input), opts, None)
                .map_err(|e| format!("{input}: {e}"))?
        }
        None => load(input, sheet(&o)?)?,
    };
    let s = d
        .scan_encodings(header, &Progress::new())
        .map_err(|e| e.to_string())?;
    let rows = row_numbers(&d, &s.failing);
    let mut msg = format!(
        "read as {}: {} rows looked at, {} plain ASCII, {} UTF-8, {} not valid UTF-8, {} double-encoded{}",
        d.format().encoding,
        s.looked,
        s.ascii,
        s.utf8,
        s.invalid,
        s.double_encoded,
        if s.mixed { "; two encodings are mixed" } else { "" }
    );
    if !rows.is_empty() {
        let list: Vec<String> = rows.iter().take(20).map(u64::to_string).collect();
        msg += &format!(
            "\nrows: {}{}",
            list.join(", "),
            if rows.len() > 20 { ", ..." } else { "" }
        );
    }
    done(
        &o,
        msg,
        json!({
            "encoding": d.format().encoding.name(),
            "looked": s.looked,
            "ascii": s.ascii,
            "utf8": s.utf8,
            "invalid": s.invalid,
            "double_encoded": s.double_encoded,
            "mixed": s.mixed,
            "rows": s.rows,
            "first_rows": rows,
        }),
    );
    Ok(!(o.has("--fail-on-found") && s.rows > 0))
}

/// `private <in>`: columns that look like they hold private data (by their
/// title, or most of their first values). `Ok(false)` when some were found
/// and `--fail-on-found`.
pub fn private(args: &[&str]) -> Result<bool, String> {
    let o = opts(args, &[], &["--fail-on-found"])?;
    let input = one_input(&o)?;
    let header = o.header();
    let d = load(input, sheet(&o)?)?;
    let found = d
        .find_private(
            header,
            Default::default(),
            natendo_engine::clean::now_utc(),
            &Progress::new(),
        )
        .map_err(|e| e.to_string())?;
    let mut msg = format!("{} columns look like they hold private data", found.len());
    for f in &found {
        msg += &format!(
            "\n  column {} {}: {} ({} of {} values{})",
            f.col + 1,
            f.title,
            kind_name(&f.kind).as_str().unwrap_or("?"),
            f.hits,
            f.looked,
            if f.by_title { ", and its title" } else { "" }
        );
    }
    done(
        &o,
        msg,
        json!({
            "columns": found.iter().map(|f| json!({
                "column": f.title,
                "number": f.col + 1,
                "kind": kind_name(&f.kind),
                "looked": f.looked,
                "hits": f.hits,
                "share": if f.looked > 0 { f.hits as f64 / f.looked as f64 } else { 0.0 },
                "by_title": f.by_title,
            })).collect::<Vec<_>>(),
        }),
    );
    Ok(!(o.has("--fail-on-found") && !found.is_empty()))
}
