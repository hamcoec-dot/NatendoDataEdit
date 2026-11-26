//! `compare` (two versions of a table by key), `match` (rows that look like
//! the same person) and `lookalikes` (values in one column that look
//! alike): the engine calls the app's Compare, Match and Value editor make.
//! The differences file, the pairs file and the look-alike groups hold cell
//! values by design; `compare --json` holds none unless `--values`.

use std::io::Write;
use std::path::Path;

use natendo_engine::diff::{DiffSpec, RESULT_LEAD};
use natendo_engine::editing::LookAlikeSpec;
use natendo_engine::fuzzy::{match_documents, FieldKind, MatchField, MatchSpec};
use natendo_engine::lookalike::Method;
use natendo_engine::{Dialect, Document, Progress};
use serde_json::{json, Value};

use crate::more::{load, write_out, Out};
use crate::safe::check_out;
use crate::tools::{col_of, cols_of, done, one_input, opts, sheet, Opts};
use crate::USAGE;

/// Rows listed by `compare --values`, groups printed by `lookalikes`.
const MAX_LISTED: usize = 1_000;

fn titles(d: &Document, header: bool) -> Vec<String> {
    let all: Vec<u32> = (0..d.col_count()).collect();
    d.titles(&all, header)
}

/// Write a CSV (formula guard on: these files are opened in spreadsheets),
/// beside the target and moved into place.
fn write_csv(
    out: &str,
    head: &[String],
    rows: impl Iterator<Item = Vec<String>>,
) -> Result<u64, String> {
    natendo_engine::save::write_atomic(Path::new(out), |w| {
        let mut line = String::new();
        let mut put = |w: &mut dyn Write, cells: &[String]| -> std::io::Result<()> {
            line.clear();
            for (i, c) in cells.iter().enumerate() {
                if i > 0 {
                    line.push(',');
                }
                natendo_engine::scan::write_field_guarded(c, Dialect::CSV, true, &mut line);
            }
            line.push_str("\r\n");
            w.write_all(line.as_bytes())
        };
        put(w, head)?;
        let mut n = 0u64;
        for r in rows {
            put(w, &r)?;
            n += 1;
        }
        Ok(n)
    })
    .map_err(|e| format!("{out}: {e}"))
}

/// `compare <a> <b> --key <col>`: `Ok(false)` when they differ and
/// `--fail-on-diff` was given.
pub fn compare(args: &[&str]) -> Result<bool, String> {
    let o = opts(
        args,
        &["--key"],
        &["--match-case", "--no-trim", "--fail-on-diff", "--values"],
    )?;
    let [a_path, b_path] = o.plain.as_slice() else {
        return Err(USAGE.to_string());
    };
    let header = o.header();
    let out = o.get("-o");
    if let Some(out) = out {
        check_out(out, &[a_path, b_path], o.has("--overwrite"))?;
    }
    let a = load(a_path, sheet(&o)?)?;
    let b = load(b_path, sheet(&o)?)?;
    let mut keys = Vec::new();
    for k in o.all("--key") {
        keys.extend(cols_of(&a, header, k)?);
    }
    if keys.is_empty() {
        return Err("give the column(s) that identify a row with --key".into());
    }
    let spec = DiffSpec {
        keys: keys.clone(),
        keep_first: header,
        match_case: o.has("--match-case"),
        trim: !o.has("--no-trim"),
    };
    let outcome = a
        .diff_files(&b, &spec, &Progress::new())
        .map_err(|e| e.to_string())?;
    let rep = outcome.report;
    let titles_a = titles(&a, header);
    let changed_cols = outcome.index.changed_columns();
    let differs = rep.changed + rep.added + rep.removed > 0;
    let values = o.has("--values");
    let mut result = if out.is_some() || values {
        Some(outcome.table.into_document().map_err(|e| e.to_string())?)
    } else {
        None
    };
    let mut msg = format!(
        "same {}, changed {}, only in the first {}, only in the second {}; {} cells changed",
        rep.same, rep.changed, rep.removed, rep.added, rep.cells
    );
    if !changed_cols.is_empty() {
        let list: Vec<String> = changed_cols
            .iter()
            .map(|&(c, n)| format!("{} ({n})", titles_a[c as usize]))
            .collect();
        msg += &format!("\nchanged columns (rows): {}", list.join(", "));
    }
    if !rep.only_a.is_empty() {
        msg += &format!("\ncolumns only in the first: {}", rep.only_a.join(", "));
    }
    if !rep.only_b.is_empty() {
        msg += &format!("\ncolumns only in the second: {}", rep.only_b.join(", "));
    }
    if rep.repeated > 0 {
        msg += &format!(
            "\n{} keys appear more than once (paired in order)",
            rep.repeated
        );
    }
    if rep.empty_a + rep.empty_b > 0 {
        msg += &format!(
            "\nrows with an empty key (never paired): {} in the first, {} in the second",
            rep.empty_a, rep.empty_b
        );
    }
    if rep.truncated {
        msg += &format!("\nonly the first {} differing rows are listed", rep.listed);
    }
    if let (Some(out), Some(t)) = (out, result.as_mut()) {
        let w = write_out(
            t,
            a_path,
            &Out {
                path: out,
                to: o.get("--to"),
                header: true,
                landscape: false,
                overwrite: o.has("--overwrite"),
                encoding: None,
            },
        )?;
        msg += &format!("\ndifferences: {w}");
    }
    let mut extra = json!({
        "same": rep.same,
        "changed": rep.changed,
        "added": rep.added,
        "removed": rep.removed,
        "cells": rep.cells,
        "listed": rep.listed,
        "truncated": rep.truncated,
        "only_a": rep.only_a,
        "only_b": rep.only_b,
        "moved": rep.moved,
        "repeated": rep.repeated,
        "empty_a": rep.empty_a,
        "empty_b": rep.empty_b,
        "differs": differs,
        "changed_columns": changed_cols.iter().map(|&(c, n)| json!({
            "column": titles_a[c as usize],
            "number": c + 1,
            "rows": n,
        })).collect::<Vec<Value>>(),
    });
    if values {
        extra["dup_a"] = json!(rep.dup_a);
        extra["dup_b"] = json!(rep.dup_b);
        if let Some(t) = &result {
            let n = t.row_count().min(MAX_LISTED as u64 + 1);
            let cell = |r: u64, c: u32| t.cell(r, c).unwrap_or_default();
            let rows: Vec<Value> = (1..n)
                .map(|r| {
                    json!({
                        "change": cell(r, 0),
                        "row_first": cell(r, 1),
                        "row_second": cell(r, 2),
                        "key": keys.iter().map(|&k| cell(r, k + RESULT_LEAD)).collect::<Vec<_>>(),
                    })
                })
                .collect();
            extra["rows"] = json!(rows);
        }
    }
    done(&o, msg, extra);
    Ok(!(o.has("--fail-on-diff") && differs))
}

fn field_kind(s: &str) -> Result<FieldKind, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "name" => Ok(FieldKind::Name),
        "text" => Ok(FieldKind::Text),
        "exact" => Ok(FieldKind::Exact),
        k => Err(format!("unknown field kind {k}: use name, text or exact")),
    }
}

/// `<col>[=<other col>][:name|text|exact][:weight]`.
fn field(
    o: &Opts,
    here: &Document,
    there: &Document,
    there_header: bool,
    spec: &str,
) -> Result<MatchField, String> {
    let mut parts = spec.split(':');
    let cols = parts.next().unwrap_or_default();
    let kind = parts
        .next()
        .map(field_kind)
        .transpose()?
        .unwrap_or_default();
    let weight = match parts.next() {
        Some(w) => w
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|w| w.is_finite() && *w > 0.0)
            .ok_or_else(|| format!("--field {spec}: the weight is a number above 0"))?,
        None => 1.0,
    };
    if parts.next().is_some() {
        return Err(format!(
            "--field {spec}: give <col>[=<other col>][:kind][:weight] (a title with a colon goes by number)"
        ));
    }
    let (a, b) = cols.split_once('=').unwrap_or((cols, cols));
    Ok(MatchField {
        col: col_of(here, o.header(), a)?,
        other_col: col_of(there, there_header, b)?,
        kind,
        weight,
    })
}

/// `match <in> [<other>] --field <col>...`: the best pair for each row.
pub fn matches(args: &[&str]) -> Result<(), String> {
    let o = opts(args, &["--field", "--min-score"], &["--other-no-header"])?;
    let (input, other) = match o.plain.as_slice() {
        [a] => (*a, None),
        [a, b] => (*a, Some(*b)),
        _ => return Err(USAGE.to_string()),
    };
    let header = o.header();
    if let Some(out) = o.get("-o") {
        let mut inputs = vec![input];
        inputs.extend(other);
        check_out(out, &inputs, o.has("--overwrite"))?;
    }
    let d = load(input, sheet(&o)?)?;
    let od = other.map(|p| load(p, sheet(&o)?)).transpose()?;
    let (there, there_header) = match &od {
        Some(x) => (x, !o.has("--other-no-header")),
        None => (&d, header),
    };
    let fields: Vec<MatchField> = o
        .all("--field")
        .into_iter()
        .map(|f| field(&o, &d, there, there_header, f))
        .collect::<Result<_, _>>()?;
    if fields.is_empty() {
        return Err(
            "give the columns to compare with --field <col>[=<other col>][:name|text|exact][:weight]"
                .into(),
        );
    }
    let min_score = match o.get("--min-score") {
        Some(s) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| (0.0..=100.0).contains(v))
            .ok_or("--min-score takes a score from 0 to 100")?,
        None => MatchSpec::default().min_score,
    };
    let spec = MatchSpec {
        fields,
        keep_first: header,
        other_header: there_header,
        min_score,
        read: Default::default(),
    };
    let rep =
        match_documents(&d, od.as_ref(), &spec, &Progress::new()).map_err(|e| e.to_string())?;
    // Scores in steps of 5 from the lowest listed.
    let mut hist: std::collections::BTreeMap<u32, u64> = Default::default();
    for c in &rep.listed {
        *hist
            .entry(((c.score as f64 / 5.0).floor() as u32 * 5).min(100))
            .or_default() += 1;
    }
    let mut msg = format!(
        "{} pairs at or above {min_score}; {} rows with a pair; {} pairs compared",
        rep.total, rep.rows_matched, rep.compared
    );
    if rep.windowed > 0 {
        msg += &format!(
            "\n{} rows were in a very large group and compared with their nearest rows only",
            rep.windowed
        );
    }
    if let Some(out) = o.get("-o") {
        let th = titles(&d, header);
        let tt = titles(there, there_header);
        let mut head = vec!["Row".to_string(), "Other row".into(), "Score".into()];
        for f in &spec.fields {
            head.push(th[f.col as usize].clone());
            head.push(format!("{} (other)", tt[f.other_col as usize]));
            head.push(format!("{} likeness", th[f.col as usize]));
        }
        let rows = rep.listed.iter().map(|c| {
            let mut r = vec![
                (c.row + 1).to_string(),
                (c.other_row + 1).to_string(),
                format!("{:.1}", c.score),
            ];
            for (k, f) in spec.fields.iter().enumerate() {
                r.push(d.cell(c.row, f.col).unwrap_or_default());
                r.push(there.cell(c.other_row, f.other_col).unwrap_or_default());
                r.push(match c.parts.get(k) {
                    Some(p) if *p >= 0.0 => format!("{p:.2}"),
                    _ => String::new(),
                });
            }
            r
        });
        let n = write_csv(out, &head, rows)?;
        msg += &format!("\nwrote {n} pairs to {out}");
    }
    done(
        &o,
        msg,
        json!({
            "total": rep.total,
            "listed": rep.listed.len(),
            "rows_matched": rep.rows_matched,
            "compared": rep.compared,
            "windowed": rep.windowed,
            "min_score": min_score,
            "histogram": hist.iter().map(|(k, v)| json!({ "from": k, "pairs": v })).collect::<Vec<_>>(),
        }),
    );
    Ok(())
}

/// `lookalikes <in> --column <c>`: groups of values that look alike, the
/// suggested spelling first.
pub fn lookalikes(args: &[&str]) -> Result<(), String> {
    let o = opts(args, &["--column", "--method"], &[])?;
    let input = one_input(&o)?;
    let header = o.header();
    if let Some(out) = o.get("-o") {
        check_out(out, &[input], o.has("--overwrite"))?;
    }
    let method = match o.get("--method").unwrap_or("sounds") {
        "sounds" => Method::Sounds,
        "spelling" => Method::Spelling,
        "both" => Method::Both,
        "abbreviations" => Method::Abbreviations,
        m => {
            return Err(format!(
                "unknown method {m}: use sounds, spelling, both or abbreviations"
            ))
        }
    };
    let d = load(input, sheet(&o)?)?;
    let col = col_of(
        &d,
        header,
        o.get("--column").ok_or("give the column with --column")?,
    )?;
    let g = d
        .lookalike_groups(
            &LookAlikeSpec {
                col,
                keep_first: header,
                method,
                extra: Vec::new(),
            },
            &Progress::new(),
        )
        .map_err(|e| e.to_string())?;
    let mut msg = format!(
        "{} groups of look-alike values among {} values{}",
        g.groups.len(),
        g.values,
        if g.complete {
            ""
        } else {
            " (only the most common were looked at)"
        }
    );
    if !o.has("--json") {
        for grp in g.groups.iter().take(MAX_LISTED) {
            let list: Vec<String> = grp
                .iter()
                .map(|t| format!("{} ({})", t.value, t.count))
                .collect();
            msg += &format!("\n{}", list.join(" | "));
        }
        if g.groups.len() > MAX_LISTED {
            msg += &format!("\n... and {} more", g.groups.len() - MAX_LISTED);
        }
    }
    if let Some(out) = o.get("-o") {
        let head = ["Group", "Value", "Count", "Suggested"].map(String::from);
        let rows = g.groups.iter().enumerate().flat_map(|(i, grp)| {
            let best = grp.first().map(|t| t.value.clone()).unwrap_or_default();
            grp.iter().map(move |t| {
                vec![
                    (i + 1).to_string(),
                    t.value.clone(),
                    t.count.to_string(),
                    best.clone(),
                ]
            })
        });
        let n = write_csv(out, &head, rows)?;
        msg += &format!("\nwrote {n} values to {out}");
    }
    done(
        &o,
        msg,
        json!({
            "groups": g.groups,
            "values": g.values,
            "complete": g.complete,
            "too_long": g.too_long,
            "crowded": g.crowded,
        }),
    );
    Ok(())
}
