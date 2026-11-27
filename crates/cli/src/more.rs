//! `convert`, `compute`, `validate` and JSON output.

use std::path::Path;

use natendo_engine::dbf::Sidecars;
use natendo_engine::export::{ExportFormat, ExportOptions};
use natendo_engine::expr::ComputeSpec;
use natendo_engine::import::{self, ImportOptions};
use natendo_engine::validate::{CheckSpec, MAX_CHECKS};
use natendo_engine::{
    Dialect, Document, Encoding, Format, LineBreak, Progress, Quoting, SaveOptions,
};
use serde_json::{json, Value};

use crate::{delimiter_name, quote_name, USAGE};

/// Largest checks file read.
const MAX_CHECKS_FILE: u64 = 4 << 20;

pub fn print_json(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

pub fn sniff_json(d: &Document) -> Value {
    let choices: Vec<Value> = d
        .detection()
        .map(|f| {
            f.choices
                .iter()
                .map(|c| {
                    json!({
                        "separator": delimiter_name(&c.dialect),
                        "quote": quote_name(&c.dialect),
                        "columns": c.columns,
                        "lines_above": c.skip_lines,
                        "score": c.score,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "separator": delimiter_name(&d.format().dialect),
        "confidence": d.detection().map(|f| f.confidence),
        "choices": choices,
    })
}

pub fn info_json(d: &Document) -> Value {
    let f = d.format();
    let s = d.index().stats();
    json!({
        "encoding": f.encoding.to_string(),
        "bom": f.bom,
        "separator": delimiter_name(&f.dialect),
        "quote": quote_name(&f.dialect),
        "lines_above_table": d.preamble_lines(),
        "line_breaks": format!("{:?}", f.line_break),
        "mixed_line_breaks": s.mixed_line_breaks(),
        "rows": d.row_count(),
        "columns": d.col_count(),
        "usual_columns": s.usual_fields(),
        "repaired_rows": s.repair_count,
        "lossy": d.is_lossy(),
    })
}

pub fn view_json(d: &Document, first: u64, count: u64) -> Result<(), String> {
    let end = first.saturating_add(count.min(10_000)).min(d.row_count());
    let mut rows = Vec::new();
    for r in first..end {
        rows.push(json!({ "row": r + 1, "cells": d.row_values(r).map_err(|e| e.to_string())? }));
    }
    print_json(&json!({ "rows": rows, "total_rows": d.row_count() }));
    Ok(())
}

fn ext_of(p: &str) -> Option<String> {
    Path::new(p)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
}

const IMPORT_TYPES: &[&str] = &[
    "xlsx", "xlsm", "ods", "json", "jsonl", "ndjson", "geojson", "xml", "html", "htm", "md",
    "markdown", "log", "prn", "fwf", "dbf",
];

/// Open any table file: delimited text directly, other formats by import.
pub(crate) fn load(path: &str, sheet: u32) -> Result<Document, String> {
    let ext = ext_of(path);
    if !ext.as_deref().is_some_and(|e| IMPORT_TYPES.contains(&e)) {
        return crate::open(path);
    }
    crate::refuse_encrypted(path)?;
    if crate::READ_AS.get().is_some() {
        return Err(format!(
            "{path}: --encoding is for delimited text files; this one is imported"
        ));
    }
    let meta = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
    if meta.len() > import::MAX_TEXT_INPUT as u64 {
        return Err(format!("{path}: too large to import"));
    }
    let data = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let opts = ImportOptions {
        part: sheet,
        beside: (ext.as_deref() == Some("dbf"))
            .then(|| std::sync::Arc::new(Sidecars::beside(Path::new(path)))),
        ..Default::default()
    };
    let (t, st) = import::import_report(&data, ext.as_deref(), &opts, None, &Progress::new())
        .map_err(|e| format!("{path}: {e}"))?;
    // Standard error, so the output (and --json) stays clean.
    for w in st.warnings {
        eprintln!("{path}: {w}");
    }
    t.into_document().map_err(|e| format!("{path}: {e}"))
}

enum Target {
    Delimited(u8),
    Export(ExportFormat),
}

fn target(out: &str, to: Option<&str>) -> Result<Target, String> {
    let kind = match to {
        Some(t) => t.to_ascii_lowercase(),
        None => ext_of(out).ok_or("give the output a file extension, or use --to")?,
    };
    Ok(match kind.as_str() {
        "csv" => Target::Delimited(b','),
        "tsv" | "tab" | "txt" => Target::Delimited(b'\t'),
        "psv" => Target::Delimited(b'|'),
        "ssv" => Target::Delimited(b';'),
        "json" => Target::Export(ExportFormat::Json),
        "xml" => Target::Export(ExportFormat::Xml),
        "html" | "htm" => Target::Export(ExportFormat::Html),
        "md" | "markdown" => Target::Export(ExportFormat::Markdown),
        "xlsx" => Target::Export(ExportFormat::Xlsx),
        "pdf" => Target::Export(ExportFormat::Pdf),
        "fixed" | "prn" => Target::Export(ExportFormat::FixedWidth),
        "sql" => Target::Export(ExportFormat::Sql),
        k => return Err(format!("unknown output type {k}")),
    })
}

pub(crate) struct Out<'a> {
    pub path: &'a str,
    pub to: Option<&'a str>,
    pub header: bool,
    pub landscape: bool,
    pub overwrite: bool,
    /// Delimited text only: the encoding to write (`--to-encoding`); UTF-8 when `None`.
    pub encoding: Option<Encoding>,
}

/// Write a document to `out` (delimited text or an export format).
pub(crate) fn write_out(d: &mut Document, input: &str, o: &Out) -> Result<String, String> {
    let out_path = Path::new(o.path);
    let same = matches!(
        (std::fs::canonicalize(input), std::fs::canonicalize(out_path)),
        (Ok(a), Ok(b)) if a == b
    );
    if same {
        return Err("write to a different file than the input".into());
    }
    if out_path.exists() && !o.overwrite {
        return Err(format!(
            "{} already exists; add --overwrite to replace it",
            o.path
        ));
    }
    match target(o.path, o.to)? {
        Target::Delimited(sep) => {
            let dialect = Dialect::new(sep, Some(b'"')).map_err(|e| e.to_string())?;
            let encoding = o.encoding.unwrap_or(Encoding::Utf8);
            let r = d
                .save(SaveOptions {
                    path: Some(out_path.to_path_buf()),
                    format: Some(Format {
                        encoding,
                        // UTF-16 and UTF-32 files are read by their byte order mark.
                        bom: !matches!(encoding, Encoding::Utf8 | Encoding::Legacy(_)),
                        dialect,
                        line_break: LineBreak::CrLf,
                    }),
                    convert_line_breaks: true,
                    rewrite: true,
                    quoting: Quoting::Needed,
                    ..Default::default()
                })
                .map_err(|e| format!("{}: {e}", o.path))?;
            Ok(format!("saved {} rows to {}", r.rows, r.path.display()))
        }
        Target::Export(_) if o.encoding.is_some() => {
            Err("--to-encoding is for delimited text (csv, tsv, psv, txt)".into())
        }
        Target::Export(format) => {
            let mut opts = ExportOptions {
                format,
                header: o.header,
                ..Default::default()
            };
            opts.page.landscape = o.landscape;
            opts.page.title = Path::new(input)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let r = d
                .export_file(out_path, &opts, &Progress::new())
                .map_err(|e| format!("{}: {e}", o.path))?;
            Ok(format!("wrote {} rows to {}", r.rows, o.path))
        }
    }
}

struct Args<'a> {
    plain: Vec<&'a str>,
    out: Option<&'a str>,
    to: Option<&'a str>,
    sheet: u32,
    header: bool,
    landscape: bool,
    overwrite: bool,
    expr: Option<&'a str>,
    title: Option<&'a str>,
    checks: Option<&'a str>,
    force: bool,
    dry_run: bool,
    to_encoding: Option<&'a str>,
}

fn parse<'a>(args: &[&'a str], allowed: &[&str]) -> Result<Args<'a>, String> {
    let mut a = Args {
        plain: Vec::new(),
        out: None,
        to: None,
        sheet: 0,
        header: true,
        landscape: false,
        overwrite: false,
        expr: None,
        title: None,
        checks: None,
        force: false,
        dry_run: false,
        to_encoding: None,
    };
    let mut it = args.iter();
    while let Some(&x) = it.next() {
        if x.starts_with('-') && !allowed.contains(&x) {
            return Err(format!("unknown option {x}\n{USAGE}"));
        }
        let mut value = || {
            it.next()
                .copied()
                .ok_or_else(|| format!("{x} needs a value"))
        };
        match x {
            "-o" | "--out" => a.out = Some(value()?),
            "--to" => a.to = Some(value()?),
            "--sheet" => {
                a.sheet = value()?
                    .parse::<u32>()
                    .ok()
                    .and_then(|n| n.checked_sub(1))
                    .ok_or("--sheet takes a sheet number from 1")?
            }
            "--no-header" => a.header = false,
            "--landscape" => a.landscape = true,
            "--overwrite" => a.overwrite = true,
            "--expr" => a.expr = Some(value()?),
            "--title" => a.title = Some(value()?),
            "--checks" => a.checks = Some(value()?),
            "--force" => a.force = true,
            "--dry-run" => a.dry_run = true,
            "--to-encoding" => a.to_encoding = Some(value()?),
            _ => a.plain.push(x),
        }
    }
    Ok(a)
}

/// What each explode step of a recipe would do on `input`: the steps run
/// in memory, in order; nothing is written.
fn explode_dry_run(
    recipe: &natendo_engine::recipe::Recipe,
    input: &str,
    sheet: u32,
) -> Vec<String> {
    use natendo_engine::recipe::RecipeOp;
    let mut out = Vec::new();
    let Ok(mut d) = load(input, sheet) else {
        return out;
    };
    let Ok(recipe) = recipe.answered(&[]) else {
        return out;
    };
    let now = natendo_engine::clean::now_utc();
    for (i, s) in recipe.steps.iter().enumerate() {
        let p = Progress::new();
        let done = match s {
            RecipeOp::ExplodeColumns { spec } | RecipeOp::ExplodeRows { spec } => {
                let title = d
                    .titles(&[spec.col], spec.keep_first)
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                let title = format!("\u{201c}{title}\u{201d}");
                let planned = if matches!(s, RecipeOp::ExplodeRows { .. }) {
                    d.plan_explode_rows(spec, &p)
                        .and_then(|(plan, r)| d.apply_rows(plan).map(|()| r))
                } else {
                    d.plan_explode_columns(spec, &p)
                        .and_then(|(plan, r)| d.apply_cols(plan).map(|()| r))
                };
                planned.map(|r| {
                    let mut l = format!(
                        "Step {} would add {} column(s) and {} row(s)",
                        i + 1,
                        r.cols,
                        r.rows
                    );
                    let note = r.note(&title);
                    if !note.is_empty() {
                        l.push_str(&format!(": {note}"));
                    }
                    out.push(l);
                })
            }
            s => d.run_recipe_step(s, now, 1, &p),
        };
        if let Err(e) = done {
            out.push(format!("The recipe would stop at step {}: {e}", i + 1));
            break;
        }
    }
    out
}

/// Run a recipe saved in the app on a file: the steps are listed first,
/// the file's columns are checked against the recipe's (`--force` runs it
/// anyway), and the result is written to `-o` (`--dry-run` only lists).
pub fn run(args: &[&str], json_out: bool) -> Result<(), String> {
    let (ask, args) = crate::receipt::Ask::take(args)?;
    let args = args.as_slice();
    // `--set name=value`: answers to the recipe's questions.
    let mut answers: Vec<(String, String)> = Vec::new();
    let mut rest: Vec<&str> = Vec::new();
    let mut it = args.iter();
    while let Some(&x) = it.next() {
        if x == "--set" {
            let v = it.next().ok_or("--set needs question=answer")?;
            let (k, a) = v
                .split_once('=')
                .ok_or("--set takes question=answer, like --set county=Hamilton")?;
            answers.push((k.trim().to_string(), a.to_string()));
        } else {
            rest.push(x);
        }
    }
    let (allow_secret, rest) = crate::secret::take_flag(&rest, crate::secret::ALLOW_IN_FILE);
    let args = rest.as_slice();
    let a = parse(
        args,
        &[
            "-o",
            "--out",
            "--to",
            "--sheet",
            "--no-header",
            "--overwrite",
            "--force",
            "--dry-run",
        ],
    )?;
    let [recipe_path, input] = a.plain.as_slice() else {
        return Err(USAGE.to_string());
    };
    let size = std::fs::metadata(recipe_path)
        .map_err(|e| format!("{recipe_path}: {e}"))?
        .len();
    if size > 16 << 20 {
        return Err(format!("{recipe_path}: a recipe file can be at most 16 MB"));
    }
    let bytes = std::fs::read(recipe_path).map_err(|e| format!("{recipe_path}: {e}"))?;
    crate::secret::refuse_secret_in_file(recipe_path, &bytes, allow_secret)?;
    let recipe = natendo_engine::recipe::Recipe::from_json(&bytes)
        .map_err(|e| format!("{recipe_path}: {e}"))?;
    let recipe = recipe
        .answered(&answers)
        .map_err(|e| format!("{recipe_path}: {e}"))?;
    let questions: Vec<(&str, &str)> = recipe
        .questions
        .iter()
        .map(|q| {
            let ask = if q.prompt.is_empty() {
                &q.name
            } else {
                &q.prompt
            };
            (ask.as_str(), q.default.as_str())
        })
        .collect();
    let steps: Vec<String> = recipe.steps.iter().map(|s| s.describe()).collect();
    // With --json the listing is part of the one result printed at the end.
    let mut report = json!({
        "recipe": recipe.name,
        "questions": questions
            .iter()
            .map(|(q, a)| json!({"question": q, "answer": a}))
            .collect::<Vec<_>>(),
        "steps": steps,
        "left_out": recipe.left_out,
        "dry_run": a.dry_run,
    });
    if !json_out {
        for (ask, answer) in &questions {
            println!("{ask}: {answer}");
        }
        println!(
            "Recipe{}: {} step(s)",
            if recipe.name.is_empty() {
                String::new()
            } else {
                format!(" \"{}\"", recipe.name)
            },
            recipe.steps.len()
        );
        for (i, s) in steps.iter().enumerate() {
            println!("  {}. {s}", i + 1);
        }
        for l in &recipe.left_out {
            println!("  (not in the recipe: {l})");
        }
    }
    if ask.on && a.dry_run {
        return Err("--receipt records a run: it does not go with --dry-run".into());
    }
    let mut making = crate::receipt::Making::start(ask, "run recipe", input)?;
    let mut d = load(input, a.sheet)?;
    if let Some(m) = making.as_mut() {
        m.read(&d)?;
    }
    let problems = recipe.column_problems(&d);
    if !problems.is_empty() {
        let text = problems.join("; ");
        if !a.force {
            return Err(format!(
                "the file's columns differ from the recipe's: {text}. Use --force to run it anyway."
            ));
        }
        if json_out {
            report["columns_differ"] = json!(problems);
        } else {
            println!("Columns differ ({text}); running anyway (--force).");
        }
    }
    if a.dry_run {
        // A "set aside" step shows the lines it would take out of the table
        // (worked out in memory: the steps run, nothing is written).
        if recipe
            .steps
            .iter()
            .any(|s| matches!(s, natendo_engine::recipe::RecipeOp::SetAside { .. }))
        {
            match d.run_recipe(
                &recipe,
                natendo_engine::clean::now_utc(),
                1,
                &Progress::new(),
            ) {
                Ok(_) => {
                    let n = d.report_notes();
                    let lines = |t: &str| t.lines().map(str::to_string).collect::<Vec<_>>();
                    if json_out {
                        report["set_aside"] = json!({
                            "above": lines(&n.above_set),
                            "below": lines(&n.below_set),
                        });
                    } else {
                        for (what, text) in [("above", &n.above_set), ("below", &n.below_set)] {
                            if text.is_empty() {
                                continue;
                            }
                            println!("Lines it would set aside {what} the table:");
                            for l in text.lines() {
                                println!("  | {l}");
                            }
                        }
                    }
                }
                Err(e) => {
                    if json_out {
                        report["would_stop"] = json!(e.to_string());
                    } else {
                        println!("The recipe would stop: {e}");
                    }
                }
            }
        }
        // Explode steps say what they would add, and the keys new or
        // missing since the recipe was made (worked out in memory).
        if recipe.steps.iter().any(|s| {
            matches!(
                s,
                natendo_engine::recipe::RecipeOp::ExplodeColumns { .. }
                    | natendo_engine::recipe::RecipeOp::ExplodeRows { .. }
            )
        }) {
            let said = explode_dry_run(&recipe, input, a.sheet);
            if json_out {
                report["explode"] = json!(said);
            } else {
                for l in &said {
                    println!("{l}");
                }
            }
        }
        let msg = "Dry run: nothing was changed or written.";
        if json_out {
            report["message"] = json!(msg);
            print_json(&report);
        } else {
            println!("{msg}");
        }
        return Ok(());
    }
    let Some(out) = a.out else {
        return Err(USAGE.to_string());
    };
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |t| t.as_nanos() as u64);
    d.run_recipe(
        &recipe,
        natendo_engine::clean::now_utc(),
        seed,
        &Progress::new(),
    )
    .map_err(|e| e.to_string())?;
    let msg = write_out(
        &mut d,
        input,
        &Out {
            path: out,
            to: a.to,
            header: a.header,
            landscape: false,
            overwrite: a.overwrite,
            encoding: None,
        },
    )?;
    let receipt = match making {
        Some(m) => {
            let steps = serde_json::to_value(&recipe.steps).map_err(|e| e.to_string())?;
            Some(m.finish(
                out,
                d.row_count(),
                d.col_count(),
                Some((steps, recipe.columns.clone())),
            )?)
        }
        None => None,
    };
    if json_out {
        report["message"] = json!(msg);
        report["rows"] = json!(d.row_count());
        report["file"] = json!(out);
        if let Some(r) = &receipt {
            report["receipt"] = json!(r.to_string_lossy());
        }
        print_json(&report);
    } else {
        println!("{msg}");
        if let Some(r) = &receipt {
            println!("Receipt written: {}", r.display());
        }
    }
    Ok(())
}

/// Make a pivot table from a layout saved in the app (`.ndpivot`) and
/// write it to `-o`. Columns are found by their titles, so the layout fits
/// next month's file even if its columns moved.
pub fn pivot(args: &[&str], json_out: bool) -> Result<(), String> {
    use natendo_engine::pivot::{SavedPivot, MAX_SAVED_PIVOT_BYTES};
    let a = parse(args, &["-o", "--out", "--to", "--no-header", "--overwrite"])?;
    let ([layout, input], Some(out)) = (a.plain.as_slice(), a.out) else {
        return Err(USAGE.to_string());
    };
    let bytes = {
        use std::io::Read;
        let f = std::fs::File::open(layout).map_err(|e| format!("{layout}: {e}"))?;
        let mut b = Vec::new();
        f.take(MAX_SAVED_PIVOT_BYTES as u64 + 1)
            .read_to_end(&mut b)
            .map_err(|e| format!("{layout}: {e}"))?;
        if b.len() > MAX_SAVED_PIVOT_BYTES {
            return Err(format!("{layout}: a pivot layout file can be at most 4 MB"));
        }
        b
    };
    let saved = SavedPivot::from_json(&bytes).map_err(|e| format!("{layout}: {e}"))?;
    let d = crate::open(input)?;
    let keep_first = a.header && saved.spec.keep_first;
    let all: Vec<u32> = (0..d.col_count()).collect();
    let titles = d.titles(&all, keep_first);
    let mut spec = saved.fit(&titles).map_err(|e| format!("{layout}: {e}"))?;
    spec.keep_first = keep_first;
    let p = Progress::new();
    let cube = d.pivot_cube(&spec, None, &p).map_err(|e| e.to_string())?;
    let table = cube
        .layout(&spec, &titles, None)
        .and_then(|g| g.to_table(spec.layout.repeat_labels))
        .map_err(|e| e.to_string())?;
    let mut t = table.into_document().map_err(|e| e.to_string())?;
    let msg = write_out(
        &mut t,
        input,
        &Out {
            path: out,
            to: a.to,
            header: true,
            landscape: false,
            overwrite: a.overwrite,
            encoding: None,
        },
    )?;
    report_written(json_out, msg, t.row_count(), out, json!({}));
    Ok(())
}

/// The result of a command that wrote a file: a line, or with `--json` an
/// object. `rows` counts every row of the table written, titles too.
fn report_written(json_out: bool, msg: String, rows: u64, file: &str, mut extra: Value) {
    if json_out {
        extra["message"] = json!(msg);
        extra["rows"] = json!(rows);
        extra["file"] = json!(file);
        print_json(&extra);
    } else {
        println!("{msg}");
    }
}

pub fn convert(args: &[&str], json_out: bool) -> Result<(), String> {
    let (ask, args) = crate::receipt::Ask::take(args)?;
    let args = args.as_slice();
    let a = parse(
        args,
        &[
            "-o",
            "--out",
            "--to",
            "--sheet",
            "--no-header",
            "--landscape",
            "--overwrite",
            "--to-encoding",
        ],
    )?;
    let ([input], Some(out)) = (a.plain.as_slice(), a.out) else {
        return Err(USAGE.to_string());
    };
    let mut making = crate::receipt::Making::start(ask, "convert", input)?;
    let to_encoding = a.to_encoding.map(crate::encoding_named).transpose()?;
    let mut d = load(input, a.sheet)?;
    if let Some(m) = making.as_mut() {
        m.read(&d)?;
    }
    let msg = write_out(
        &mut d,
        input,
        &Out {
            path: out,
            to: a.to,
            header: a.header,
            landscape: a.landscape,
            overwrite: a.overwrite,
            encoding: to_encoding,
        },
    )?;
    let extra = match making {
        Some(m) => {
            let r = m.finish(out, d.row_count(), d.col_count(), None)?;
            if !json_out {
                eprintln!("Receipt written: {}", r.display());
            }
            json!({ "receipt": r.to_string_lossy() })
        }
        None => json!({}),
    };
    report_written(json_out, msg, d.row_count(), out, extra);
    Ok(())
}

pub fn compute(args: &[&str], json_out: bool) -> Result<(), String> {
    let a = parse(
        args,
        &[
            "-o",
            "--out",
            "--to",
            "--sheet",
            "--no-header",
            "--overwrite",
            "--expr",
            "--title",
        ],
    )?;
    let ([input], Some(out), Some(expr)) = (a.plain.as_slice(), a.out, a.expr) else {
        return Err(USAGE.to_string());
    };
    let mut d = load(input, a.sheet)?;
    let spec = ComputeSpec {
        expr: expr.to_string(),
        title: a.title.unwrap_or("Computed").to_string(),
        keep_first: a.header,
        now: natendo_engine::clean::now_utc(),
        ..Default::default()
    };
    let (plan, rep) = d
        .plan_compute(&spec, &Progress::new())
        .map_err(|e| e.to_string())?;
    d.apply_cols(plan).map_err(|e| e.to_string())?;
    let msg = write_out(
        &mut d,
        input,
        &Out {
            path: out,
            to: a.to,
            header: a.header,
            landscape: false,
            overwrite: a.overwrite,
            encoding: None,
        },
    )?;
    if rep.errors > 0 && !json_out {
        println!("{} cells came out as errors (like #VALUE!)", rep.errors);
    }
    report_written(
        json_out,
        msg,
        d.row_count(),
        out,
        json!({"errors": rep.errors}),
    );
    Ok(())
}

/// Read a checks file: `{"checks": [...]}` or a bare list. A check may name
/// its column with `"column": "Title"` (or a number from 1) instead of
/// `"col"` (from 0), and a unique check the other columns of its key with
/// `"also_columns": ["Title", ...]` instead of `"also"` (from 0) in its kind, and a
/// settled-by check its settling column with `"by_column": "Title"` instead of `"by"`.
fn read_checks(path: &str, d: &Document, header: bool) -> Result<CheckSpec, String> {
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{path}: {e}"))?
        .len();
    if size > MAX_CHECKS_FILE {
        return Err(format!("{path}: a checks file can be at most 4 MB"));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let mut v: Value = serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let names = d.expr_names(header);
    let table_checks = match &mut v {
        Value::Object(o) => match o.remove("table_checks") {
            Some(tc) => read_table_checks(path, tc, &names)?,
            None => Vec::new(),
        },
        _ => Vec::new(),
    };
    let list = match &mut v {
        Value::Array(a) => a,
        Value::Object(o) => match o.get_mut("checks") {
            Some(Value::Array(a)) => a,
            _ => return Err(format!("{path}: expected a \"checks\" list")),
        },
        _ => return Err(format!("{path}: expected a \"checks\" list")),
    };
    if list.len() > MAX_CHECKS {
        return Err(format!("{path}: at most {MAX_CHECKS} checks"));
    }
    for c in list.iter_mut() {
        let Value::Object(o) = c else {
            return Err(format!("{path}: each check is an object"));
        };
        let find = |col: &Value| find_column(path, col, &names);
        if let Some(col) = o.remove("column") {
            let n = find(&col)?;
            o.insert("col".into(), json!(n));
        }
        // A unique rule over several columns: the others by title (or number from 1).
        if let Some(also) = o.remove("also_columns") {
            let Value::Array(list) = &also else {
                return Err(format!("{path}: \"also_columns\" is a list of columns"));
            };
            if list.len() >= natendo_engine::validate::MAX_KEY_COLS {
                return Err(format!(
                    "{path}: a unique rule can use at most {} columns together",
                    natendo_engine::validate::MAX_KEY_COLS
                ));
            }
            let cols = list.iter().map(&find).collect::<Result<Vec<_>, _>>()?;
            match o.get_mut("kind") {
                Some(Value::Object(k)) if k.get("type") == Some(&json!("unique")) => {
                    k.insert("also".into(), json!(cols));
                }
                _ => {
                    return Err(format!(
                        "{path}: \"also_columns\" goes with a unique check only"
                    ))
                }
            }
        }
        if let Some(by) = o.remove("by_column") {
            let n = find(&by)?;
            match o.get_mut("kind") {
                Some(Value::Object(k)) if k.get("type") == Some(&json!("settled_by")) => {
                    k.insert("by".into(), json!(n));
                }
                _ => {
                    return Err(format!(
                        "{path}: \"by_column\" goes with a settled_by check only"
                    ))
                }
            }
        }
    }
    let checks = serde_json::from_value(Value::Array(std::mem::take(list)))
        .map_err(|e| format!("{path}: {e}"))?;
    let spec = CheckSpec {
        checks,
        keep_first: header,
        now: natendo_engine::clean::now_utc(),
        table_checks,
        ..Default::default()
    };
    natendo_engine::validate::check_spec(&spec).map_err(|e| format!("{path}: {e}"))?;
    Ok(spec)
}

/// A checks file's column: a title (exact, else any case) or a number from 1.
fn find_column(path: &str, col: &Value, names: &[String]) -> Result<u64, String> {
    match col {
        Value::Number(n) => n.as_u64().and_then(|n| n.checked_sub(1)),
        Value::String(s) => names
            .iter()
            .position(|t| t == s)
            .or_else(|| names.iter().position(|t| t.eq_ignore_ascii_case(s)))
            .map(|i| i as u64),
        _ => None,
    }
    .filter(|&n| (n as usize) < names.len())
    .ok_or_else(|| format!("{path}: no column {col}"))
}

/// `"table_checks": [...]`: each `{"name"?, "kind": {...}}` as the app keeps them, with a
/// `"column"` (title or number from 1) in the kind of a no-blanks or distinct check.
fn read_table_checks(
    path: &str,
    mut v: Value,
    names: &[String],
) -> Result<Vec<natendo_engine::tablecheck::TableCheck>, String> {
    let Value::Array(list) = &mut v else {
        return Err(format!("{path}: \"table_checks\" is a list"));
    };
    if list.len() > natendo_engine::tablecheck::MAX_TABLE_CHECKS {
        return Err(format!(
            "{path}: at most {} table checks",
            natendo_engine::tablecheck::MAX_TABLE_CHECKS
        ));
    }
    for c in list.iter_mut() {
        if let Some(Value::Object(k)) = c.get_mut("kind") {
            if let Some(col) = k.remove("column") {
                let n = find_column(path, &col, names)?;
                k.insert("col".into(), json!(n));
            }
        }
    }
    serde_json::from_value(v).map_err(|e| format!("{path}: table checks: {e}"))
}

/// Most `--table` files for one run.
const MAX_TABLES: usize = 8;

/// `--table` names and their files.
type NamedFiles<'a> = Vec<(String, &'a str)>;

/// `--table name=file` pairs (split at the first `=`, so a path may hold `=`), taken out of
/// `args`. Refused: no `=`, an empty name or file, a name given twice, more than 8.
fn take_tables<'a>(args: &[&'a str]) -> Result<(NamedFiles<'a>, Vec<&'a str>), String> {
    let mut tables: Vec<(String, &str)> = Vec::new();
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(&x) = it.next() {
        if x != "--table" {
            rest.push(x);
            continue;
        }
        let v = it.next().ok_or("--table needs name=file")?;
        let (name, file) = v
            .split_once('=')
            .ok_or("--table takes name=file, like --table precincts=C:\\data\\precincts.csv")?;
        let name = name.trim();
        if name.is_empty() || file.is_empty() {
            return Err("--table takes name=file: the name and the file may not be empty".into());
        }
        if tables.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)) {
            return Err(format!("--table {name} is given twice"));
        }
        tables.push((name.to_string(), file));
        if tables.len() > MAX_TABLES {
            return Err(format!("at most {MAX_TABLES} --table files"));
        }
    }
    Ok((tables, rest))
}

/// Read the tables the "value must exist in" rules name: from `--table` files, or the input
/// file itself when a rule names it (by file name, or file name without its extension).
/// A rule whose table is not given, or a `--table` no rule uses, is an error (exit 2), so a
/// script never passes because a rule did not run.
fn bind_tables(
    spec: &mut CheckSpec,
    d: &Document,
    input: &str,
    tables: &[(String, &str)],
) -> Result<(), String> {
    use natendo_engine::keyset::{KeyBinding, KeySet};
    use natendo_engine::validate::CheckKind;
    use std::sync::Arc;
    let file_name = std::path::Path::new(input)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let is_input = |name: &str| {
        let n = name.trim().to_lowercase();
        n == file_name
            || file_name
                .rsplit_once('.')
                .is_some_and(|(stem, _)| stem == n)
    };
    let mut loaded: Vec<(String, Document)> = Vec::new();
    let mut used = vec![false; tables.len()];
    let mut keys = Vec::with_capacity(spec.checks.len());
    let budget = natendo_engine::sysmem::installed_bytes()
        .map_or(1 << 30, |b| b / 4)
        .min(2 << 30);
    for (i, c) in spec.checks.iter().enumerate() {
        let CheckKind::ForeignKey {
            table,
            column,
            match_case,
            trim,
            ..
        } = &c.kind
        else {
            keys.push(KeyBinding::None);
            continue;
        };
        let given = tables
            .iter()
            .position(|(n, _)| n.eq_ignore_ascii_case(table.trim()));
        let other: &Document = match given {
            Some(k) => {
                used[k] = true;
                if !loaded.iter().any(|(n, _)| n == &tables[k].0) {
                    let o = load(tables[k].1, 0)?;
                    loaded.push((tables[k].0.clone(), o));
                }
                &loaded
                    .iter()
                    .find(|(n, _)| n == &tables[k].0)
                    .expect("loaded")
                    .1
            }
            None if is_input(table) => d,
            None => {
                return Err(format!(
                    "rule {} reads the table {}: give it with --table {}=<file>",
                    i + 1,
                    table.trim(),
                    table.trim()
                ))
            }
        };
        let keep_first = if std::ptr::eq(other, d) {
            spec.keep_first
        } else {
            true
        };
        let titles = other.expr_names(keep_first);
        let want = column.trim().to_lowercase();
        let col = titles
            .iter()
            .position(|t| t.trim().to_lowercase() == want)
            .ok_or_else(|| {
                format!(
                    "rule {}: {} has no column {}",
                    i + 1,
                    table.trim(),
                    column.trim()
                )
            })?;
        let mut set = KeySet::build(
            other,
            col as u32,
            keep_first,
            *match_case,
            *trim,
            budget,
            &Progress::new(),
        )
        .map_err(|e| format!("rule {}: {e}", i + 1))?;
        set.source.table = table.trim().to_string();
        set.source.column = titles[col].clone();
        keys.push(KeyBinding::Ready(Arc::new(set)));
    }
    // "As many rows as" table checks: shown data rows of the file given (titles in its first row).
    let mut rows = Vec::with_capacity(spec.table_checks.len());
    for (i, c) in spec.table_checks.iter().enumerate() {
        let natendo_engine::tablecheck::TableCheckKind::RowCountSameAs { table } = &c.kind else {
            rows.push(None);
            continue;
        };
        let n = match tables
            .iter()
            .position(|(n, _)| n.eq_ignore_ascii_case(table.trim()))
        {
            Some(k) => {
                used[k] = true;
                match loaded.iter().find(|(n, _)| n == &tables[k].0) {
                    Some((_, o)) => o.row_count().saturating_sub(1),
                    None => {
                        let o = load(tables[k].1, 0)?;
                        let n = o.row_count().saturating_sub(1);
                        loaded.push((tables[k].0.clone(), o));
                        n
                    }
                }
            }
            None if is_input(table) => {
                let n = d.row_count();
                n - u64::from(spec.keep_first && n > 0)
            }
            None => {
                return Err(format!(
                    "table check {} reads the table {}: give it with --table {}=<file>",
                    i + 1,
                    table.trim(),
                    table.trim()
                ))
            }
        };
        rows.push(Some(Ok(n)));
    }
    if let Some(k) = used.iter().position(|u| !u) {
        return Err(format!("--table {} is not read by any rule", tables[k].0));
    }
    spec.keys = keys;
    spec.table_rows = rows;
    Ok(())
}

/// Run checks; `Ok(true)` when there are no problems.
pub fn validate(args: &[&str], json_out: bool) -> Result<bool, String> {
    let (tables, args) = take_tables(args)?;
    let a = parse(&args, &["--checks", "--no-header", "--sheet"])?;
    let ([input], Some(checks)) = (a.plain.as_slice(), a.checks) else {
        return Err(USAGE.to_string());
    };
    let d = load(input, a.sheet)?;
    let mut spec = read_checks(checks, &d, a.header)?;
    bind_tables(&mut spec, &d, input, &tables)?;
    let r = d
        .run_checks(&spec, &Progress::new())
        .map_err(|e| e.to_string())?;
    if let Some(n) = r.not_run.first() {
        return Err(format!("rule {} did not run: {}", n.check + 1, n.why));
    }
    if let Some(t) = r.table.iter().find(|t| t.ok.is_none()) {
        return Err(format!(
            "table check {} did not run: {}",
            t.check + 1,
            t.why.as_deref().unwrap_or("")
        ));
    }
    let table_ok = r.table.iter().all(|t| t.ok == Some(true));
    let table: Vec<Value> = r
        .table
        .iter()
        .map(|t| {
            json!({
                "check": t.check + 1,
                "name": t.name,
                "ok": t.ok,
                "actual": t.actual,
                "expected": t.expected,
            })
        })
        .collect();
    let names = d.expr_names(a.header);
    let foreign: Vec<Value> = r
        .tables
        .iter()
        .map(|t| {
            json!({
                "check": t.check + 1,
                "table": t.source.table,
                "column": t.source.column,
                "missing": r.per_check.get(t.check as usize).copied().unwrap_or(0),
            })
        })
        .collect();
    if json_out {
        let problems: Vec<Value> = r
            .listed
            .iter()
            .map(|p| {
                json!({
                    "row": p.row + 1,
                    "column": p.col.map(|c| names.get(c as usize).cloned().unwrap_or_default()),
                    "check": p.check + 1,
                    "value": p.value,
                })
            })
            .collect();
        print_json(&json!({
            "rows_checked": r.checked,
            "rows_with_problems": r.rows,
            "problems": r.problems,
            "per_check": r.per_check,
            "listed": problems,
            "foreign_keys": foreign,
            "table_checks": table,
        }));
    } else {
        println!(
            "{} rows checked, {} with problems ({} problems)",
            r.checked, r.rows, r.problems
        );
        for (i, n) in r.per_check.iter().enumerate() {
            println!("  check {}: {n}", i + 1);
        }
        for p in r.listed.iter().take(20) {
            let col = p
                .col
                .map(|c| names.get(c as usize).cloned().unwrap_or_default())
                .unwrap_or_else(|| "(row)".into());
            println!(
                "  row {}, {col}: check {} ({})",
                p.row + 1,
                p.check + 1,
                p.value.escape_debug()
            );
        }
        for t in &r.table {
            println!(
                "  table check {} {}: {}: {} (wanted {})",
                t.check + 1,
                if t.ok == Some(true) {
                    "passed"
                } else {
                    "FAILED"
                },
                t.name,
                t.actual,
                t.expected
            );
        }
    }
    Ok(r.problems == 0 && table_ok)
}
