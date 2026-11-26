//! `patch create` and `patch apply`: portable patch files. A patch holds
//! data, so it is protected with a phrase unless `--plain` says otherwise;
//! the phrase is read from standard input or a file, never from the command
//! line.

use std::io::BufRead;
use std::path::Path;

use natendo_engine::patch::{Patch, PatchReport, PatchSpec};
use natendo_engine::privacy::Secret;
use natendo_engine::Progress;
use serde_json::json;

use crate::more::{load, print_json, write_out, Out};
use crate::USAGE;

struct Args<'a> {
    plain: Vec<&'a str>,
    keys: Vec<&'a str>,
    out: Option<&'a str>,
    to: Option<&'a str>,
    sheet: u32,
    phrase_file: Option<&'a str>,
    flags: Vec<&'a str>,
}

impl Args<'_> {
    fn has(&self, flag: &str) -> bool {
        self.flags.contains(&flag)
    }
}

fn parse<'a>(args: &[&'a str], flags: &[&str]) -> Result<Args<'a>, String> {
    let mut a = Args {
        plain: Vec::new(),
        keys: Vec::new(),
        out: None,
        to: None,
        sheet: 0,
        phrase_file: None,
        flags: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(&x) = it.next() {
        let mut value = || {
            it.next()
                .copied()
                .ok_or_else(|| format!("{x} needs a value"))
        };
        match x {
            "-o" | "--out" => a.out = Some(value()?),
            "--key" => a.keys.push(value()?),
            "--to" => a.to = Some(value()?),
            "--phrase-file" => a.phrase_file = Some(value()?),
            "--sheet" => {
                a.sheet = value()?
                    .parse::<u32>()
                    .ok()
                    .and_then(|n| n.checked_sub(1))
                    .ok_or("--sheet takes a sheet number from 1")?;
            }
            _ if flags.contains(&x) => a.flags.push(x),
            _ if x.starts_with('-') && x.len() > 1 => {
                // The value may be a phrase typed by mistake: name the
                // option only.
                let name = x.split('=').next().unwrap_or(x);
                return Err(format!("unknown option {name}\n{USAGE}"));
            }
            _ => a.plain.push(x),
        }
    }
    Ok(a)
}

/// The phrase from standard input or a file (its first line), if asked for.
fn phrase(a: &Args) -> Result<Option<Secret>, String> {
    let line = match (a.has("--phrase-stdin"), a.phrase_file) {
        (true, Some(_)) => return Err("give --phrase-stdin or --phrase-file, not both".into()),
        (true, None) => {
            let mut line = String::new();
            std::io::stdin()
                .lock()
                .read_line(&mut line)
                .map_err(|e| format!("could not read the phrase: {e}"))?;
            line
        }
        (false, Some(path)) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("could not read the phrase file: {e}"))?;
            text.lines().next().unwrap_or_default().to_string()
        }
        (false, None) => return Ok(None),
    };
    let line = line
        .trim_start_matches('\u{feff}')
        .trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return Err("the phrase is empty".into());
    }
    Ok(Some(Secret::new(line)))
}

fn same_file(a: &str, b: &str) -> bool {
    matches!(
        (std::fs::canonicalize(a), std::fs::canonicalize(b)),
        (Ok(x), Ok(y)) if x == y
    )
}

/// `patch create <old> <new> --key <col> -o <file.ndpatch>`.
pub fn create(args: &[&str], json_out: bool) -> Result<(), String> {
    let a = parse(
        args,
        &[
            "--match-case",
            "--no-trim",
            "--leave-out",
            "--plain",
            "--phrase-stdin",
            "--overwrite",
        ],
    )?;
    let ([old, new], Some(out)) = (a.plain.as_slice(), a.out) else {
        return Err(USAGE.to_string());
    };
    if a.keys.is_empty() {
        return Err("give the column(s) that identify a row with --key".into());
    }
    if !Path::new(out)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ndpatch"))
    {
        return Err("a patch file's name ends in .ndpatch".into());
    }
    if same_file(old, out) || same_file(new, out) {
        return Err("write to a different file than the input".into());
    }
    if Path::new(out).exists() && !a.has("--overwrite") {
        return Err(format!(
            "{out} already exists; add --overwrite to replace it"
        ));
    }
    let phrase = phrase(&a)?;
    match (&phrase, a.has("--plain")) {
        (None, false) => return Err(
            "a patch holds data: protect it with --phrase-stdin or --phrase-file, or say --plain"
                .into(),
        ),
        (Some(_), true) => return Err("--plain goes without a phrase".into()),
        _ => {}
    }
    let older = load(old, a.sheet)?;
    let newer = load(new, a.sheet)?;
    let titles = older.titles(&(0..older.col_count()).collect::<Vec<_>>(), true);
    let keys = a
        .keys
        .iter()
        .map(|name| {
            titles
                .iter()
                .position(|t| t.trim().eq_ignore_ascii_case(name.trim()))
                .or_else(|| {
                    let n: usize = name.parse().ok()?;
                    (1..=titles.len()).contains(&n).then(|| n - 1)
                })
                .map(|i| i as u32)
                .ok_or_else(|| format!("there is no column \"{name}\" in the older file"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let spec = PatchSpec {
        keys,
        match_case: a.has("--match-case"),
        trim: !a.has("--no-trim"),
        leave_out: a.has("--leave-out"),
    };
    let made = older
        .make_patch(&newer, &spec, &Progress::new())
        .map_err(|e| e.to_string())?;
    let p = &made.patch;
    p.save(Path::new(out), phrase.as_ref())
        .map_err(|e| e.to_string())?;
    let cells: usize = p.changes.iter().map(|c| c.cells.len()).sum();
    let protection = if phrase.is_some() {
        "protected with the phrase"
    } else {
        "NOT protected: it holds data from both files in the clear"
    };
    let mut msg = format!(
        "{} rows changed ({cells} cells), {} added, {} removed; wrote {out}, {protection}",
        p.changes.len(),
        p.adds.len(),
        p.removes.len()
    );
    if !made.left_out.is_empty() {
        msg.push_str(&format!("\nleft out: {}", made.left_out.join(", ")));
    }
    if json_out {
        print_json(&json!({
            "message": msg,
            "changes": p.changes.len(),
            "cells": cells,
            "adds": p.adds.len(),
            "removes": p.removes.len(),
            "left_out": made.left_out,
            "protected": phrase.is_some(),
        }));
    } else {
        println!("{msg}");
    }
    Ok(())
}

fn words(r: &PatchReport, dry: bool) -> String {
    let mut s = format!(
        "{} {}: {} cells, {} rows added, {} rows removed\n\
         {} already applied, {} conflicts, {} keys not found, {} keys found more than once, \
         {} in protected columns, {} in missing columns",
        r.will_apply,
        if dry { "would apply" } else { "applied" },
        r.cells,
        r.added,
        r.removed,
        r.already,
        r.conflicts,
        r.not_found,
        r.twice,
        r.protected,
        r.missing_column,
    );
    if !r.missing.is_empty() {
        s.push_str(&format!("\nnot in this file: {}", r.missing.join(", ")));
    }
    if !r.extra.is_empty() {
        s.push_str(&format!(
            "\nnot compared (blank in new rows): {}",
            r.extra.join(", ")
        ));
    }
    s
}

/// `patch apply <file.ndpatch> <data> -o <out>`. `Ok(false)` when something
/// could not be applied.
pub fn apply(args: &[&str], json_out: bool) -> Result<bool, String> {
    let a = parse(args, &["--dry-run", "--phrase-stdin", "--overwrite"])?;
    let [patch_path, data] = a.plain.as_slice() else {
        return Err(USAGE.to_string());
    };
    let dry = a.has("--dry-run");
    if !dry {
        let out = a
            .out
            .ok_or("give the output file with -o, or use --dry-run")?;
        if same_file(patch_path, out) || same_file(data, out) {
            return Err("write to a different file than the input".into());
        }
    }
    let phrase = phrase(&a)?;
    let patch = Patch::load(Path::new(patch_path), phrase.as_ref())
        .map_err(|e| format!("{patch_path}: {e}"))?;
    let mut d = load(data, a.sheet)?;
    let (plan, report) = d
        .plan_patch(&patch, &Progress::new())
        .map_err(|e| e.to_string())?;
    let mut msg = words(&report, dry);
    if let (false, Some(out)) = (dry, a.out) {
        if report.will_apply > 0 {
            d.apply_rows(plan).map_err(|e| e.to_string())?;
        }
        let wrote = write_out(
            &mut d,
            data,
            &Out {
                path: out,
                to: a.to,
                header: true,
                landscape: false,
                overwrite: a.has("--overwrite"),
            },
        )?;
        msg.push('\n');
        msg.push_str(&wrote);
    }
    if json_out {
        print_json(&json!({ "message": msg, "report": report }));
    } else {
        println!("{msg}");
    }
    Ok(report.unapplied() == 0)
}
