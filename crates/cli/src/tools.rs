//! The command-line toolkit: `headers`, `select`, `filter`, `sort`,
//! `dedupe`, `stats`, `count`, `sample`, `slice`, `split` and `cat`. Each
//! is a thin wrapper over what the app does from its menus, with the same
//! limits. Columns are named by title or by number from 1.

use std::io::{BufWriter, Write};
use std::path::Path;

use natendo_engine::counts::CountsSpec;
use natendo_engine::expr::ComputeSpec;
use natendo_engine::gaps::{DateStep, GapsSpec, Pattern};
use natendo_engine::profile::ProfileSpec;
use natendo_engine::recipe::RecipeOp;
use natendo_engine::reshape::Duplicates;
use natendo_engine::sets::{compare_lists, slice_name, SetList, SetsSpec, MAX_LISTS};
use natendo_engine::sort::{
    SortKey, SortKind, SortSpec, BUILTIN_LISTS, MAX_ITEM_CHARS, MAX_LISTS as MAX_SORT_LISTS,
    MAX_LIST_ITEMS,
};
use natendo_engine::tools::{Sample, SampleSize, TitleUnion};
use natendo_engine::{Document, Progress};
use serde_json::{json, Value};

use crate::more::{load, print_json, write_out, Out};
use crate::USAGE;

/// Files `cat` stacks at most.
const MAX_CAT_FILES: usize = 1_000;
/// Columns `cat --by-name` writes at most.
const MAX_CAT_COLS: usize = 16_384;
/// Input bytes `cat --by-name` keeps open between its two passes.
const MAX_CAT_HELD: u64 = 1 << 30;
/// Parts `split` writes at most.
const MAX_PARTS: u64 = 10_000;
/// Lines `count` prints at most (the counts file has them all).
const MAX_PRINTED: usize = 1_000;

/// The options of one command: values by name, flags, and the rest.
pub(crate) struct Opts<'a> {
    pub(crate) plain: Vec<&'a str>,
    values: Vec<(&'a str, &'a str)>,
    flags: Vec<&'a str>,
}

impl<'a> Opts<'a> {
    pub(crate) fn get(&self, k: &str) -> Option<&'a str> {
        self.values
            .iter()
            .rev()
            .find(|(n, _)| *n == k)
            .map(|(_, v)| *v)
    }
    pub(crate) fn has(&self, k: &str) -> bool {
        self.flags.contains(&k)
    }
    /// Every value given for `k`, in order.
    pub(crate) fn all(&self, k: &str) -> Vec<&'a str> {
        self.values
            .iter()
            .filter(|(n, _)| *n == k)
            .map(|(_, v)| *v)
            .collect()
    }
    pub(crate) fn header(&self) -> bool {
        !self.has("--no-header")
    }
}

fn parse<'a>(args: &[&'a str], values: &[&str], flags: &[&str]) -> Result<Opts<'a>, String> {
    let mut o = Opts {
        plain: Vec::new(),
        values: Vec::new(),
        flags: Vec::new(),
    };
    let mut it = args.iter();
    while let Some(&x) = it.next() {
        let x = if x == "--out" { "-o" } else { x };
        if values.contains(&x) {
            let v = it.next().ok_or_else(|| format!("{x} needs a value"))?;
            o.values.push((x, v));
        } else if flags.contains(&x) {
            o.flags.push(x);
        } else if x.starts_with('-') && x.len() > 1 {
            return Err(format!("unknown option {x}\n{USAGE}"));
        } else {
            o.plain.push(x);
        }
    }
    Ok(o)
}

/// Column `name` (a title, any case, or a number from 1).
pub(crate) fn col_of(d: &Document, header: bool, name: &str) -> Result<u32, String> {
    let name = name.trim();
    let n = d.col_count();
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) {
        let k: u32 = name.parse().map_err(|_| format!("no column {name}"))?;
        return if k >= 1 && k <= n {
            Ok(k - 1)
        } else {
            Err(format!("there is no column {k} (the file has {n})"))
        };
    }
    if header {
        let cols: Vec<u32> = (0..n).collect();
        let titles = d.titles(&cols, true);
        if let Some(i) = titles
            .iter()
            .position(|t| t.trim().eq_ignore_ascii_case(name))
        {
            return Ok(i as u32);
        }
    }
    Err(format!("there is no column \"{name}\""))
}

pub(crate) fn cols_of(d: &Document, header: bool, list: &str) -> Result<Vec<u32>, String> {
    list.split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| col_of(d, header, s))
        .collect()
}

pub(crate) fn out_of<'a>(o: &Opts<'a>) -> Result<Out<'a>, String> {
    Ok(Out {
        path: o.get("-o").ok_or("give the output file with -o")?,
        to: o.get("--to"),
        header: o.header(),
        landscape: false,
        overwrite: o.has("--overwrite"),
        encoding: None,
    })
}

pub(crate) fn one_input<'a>(o: &Opts<'a>) -> Result<&'a str, String> {
    match o.plain.as_slice() {
        [x] => Ok(x),
        _ => Err(USAGE.to_string()),
    }
}

fn step(d: &mut Document, op: RecipeOp) -> Result<(), String> {
    d.run_recipe_step(&op, natendo_engine::clean::now_utc(), 1, &Progress::new())
        .map_err(|e| e.to_string())
}

const OUT_OPTS: [&str; 3] = ["-o", "--to", "--sheet"];
const OUT_FLAGS: [&str; 3] = ["--no-header", "--overwrite", "--json"];

pub(crate) fn opts<'a>(
    args: &[&'a str],
    more: &[&str],
    flags: &[&str],
) -> Result<Opts<'a>, String> {
    let values: Vec<&str> = OUT_OPTS.iter().chain(more).copied().collect();
    let fl: Vec<&str> = OUT_FLAGS.iter().chain(flags).copied().collect();
    parse(args, &values, &fl)
}

pub(crate) fn sheet(o: &Opts) -> Result<u32, String> {
    match o.get("--sheet") {
        None => Ok(0),
        Some(s) => s
            .parse::<u32>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .ok_or_else(|| "--sheet takes a sheet number from 1".to_string()),
    }
}

pub(crate) fn done(o: &Opts, msg: String, extra: Value) {
    if o.has("--json") {
        let mut v = extra;
        v["message"] = json!(msg);
        print_json(&v);
    } else {
        let _ = crate::outln!("{msg}");
    }
}

/// `headers <in>`: the column numbers and titles.
pub fn headers(args: &[&str]) -> Result<(), String> {
    let o = opts(args, &[], &[])?;
    let d = load(one_input(&o)?, sheet(&o)?)?;
    let cols: Vec<u32> = (0..d.col_count()).collect();
    let titles = d.titles(&cols, o.header());
    if o.has("--json") {
        print_json(&json!(titles
            .iter()
            .enumerate()
            .map(|(i, t)| json!({"column": i + 1, "title": t}))
            .collect::<Vec<_>>()));
    } else {
        for (i, t) in titles.iter().enumerate() {
            let _ = crate::outln!("{:>4}  {t}", i + 1);
        }
    }
    Ok(())
}

/// `select <in> -o <out> --columns a,b`: keep these columns, in this order.
pub fn select(args: &[&str]) -> Result<(), String> {
    let o = opts(args, &["--columns"], &[])?;
    let input = one_input(&o)?;
    let mut d = load(input, sheet(&o)?)?;
    let order = cols_of(&d, o.header(), o.get("--columns").ok_or("give --columns")?)?;
    if order.is_empty() {
        return Err("give at least one column".into());
    }
    let names = vec![String::new(); order.len()];
    step(
        &mut d,
        RecipeOp::Arrange {
            spec: natendo_engine::editing::Arrange {
                order,
                names,
                keep_first: o.header(),
            },
        },
    )?;
    let msg = write_out(&mut d, input, &out_of(&o)?)?;
    done(&o, msg, json!({"columns": d.col_count()}));
    Ok(())
}

/// `filter <in> -o <out> --expr <formula>`: keep the rows where it is TRUE.
pub fn filter(args: &[&str]) -> Result<(), String> {
    let o = opts(args, &["--expr"], &[])?;
    let input = one_input(&o)?;
    let mut d = load(input, sheet(&o)?)?;
    let spec = ComputeSpec {
        expr: o.get("--expr").ok_or("give --expr")?.to_string(),
        keep_first: o.header(),
        now: natendo_engine::clean::now_utc(),
        ..Default::default()
    };
    let (plan, errors) = d
        .plan_formula_filter(&spec, &Progress::new())
        .map_err(|e| e.to_string())?;
    let kept = plan.rows;
    d.apply_rows(plan).map_err(|e| e.to_string())?;
    let keep = d.plan_keep_shown().map_err(|e| e.to_string())?;
    d.apply_rows(keep).map_err(|e| e.to_string())?;
    let msg = write_out(&mut d, input, &out_of(&o)?)?;
    if errors > 0 && !o.has("--json") {
        let _ = crate::outln!("{errors} rows gave an error and were left out");
    }
    done(&o, msg, json!({"kept": kept, "errors": errors}));
    Ok(())
}

/// Split at `sep` where it is not escaped with a backslash: `\<sep>` is the
/// separator itself and, with `last` (the innermost split), `\\` one
/// backslash. Other escapes are kept for the next split.
fn split_escaped(s: &str, sep: char, last: bool) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(n) if n == sep || (last && n == '\\') => out.last_mut().unwrap().push(n),
                Some(n) => {
                    let last = out.last_mut().unwrap();
                    last.push('\\');
                    last.push(n);
                }
                None => out.last_mut().unwrap().push('\\'),
            },
            c if c == sep => out.push(String::new()),
            c => out.last_mut().unwrap().push(c),
        }
    }
    out
}

/// A custom order's values as typed: blanks and repeats (case and spaces
/// aside) dropped, within the engine's limits.
fn order_items(items: impl IntoIterator<Item = String>) -> Result<Vec<String>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for i in items {
        let t = i.trim().to_string();
        if t.is_empty() || !seen.insert(t.to_lowercase()) {
            continue;
        }
        if t.chars().count() > MAX_ITEM_CHARS {
            return Err(format!(
                "a value of a custom order is longer than {MAX_ITEM_CHARS} characters"
            ));
        }
        out.push(t);
        if out.len() > MAX_LIST_ITEMS {
            return Err(format!(
                "a custom order holds at most {MAX_LIST_ITEMS} values"
            ));
        }
    }
    if out.is_empty() {
        return Err("the custom order is empty: give its values as order=A|B|C".into());
    }
    Ok(out)
}

/// One `--by` key: `col[:asc|:desc][:<ready-made order>][:order=A|B|C]`.
/// `order=` takes the rest of the key (so values may hold `:`), or
/// `order=@file` reads one value per line.
fn sort_key(
    d: &Document,
    header: bool,
    k: &str,
    lists: &mut Vec<Vec<String>>,
) -> Result<SortKey, String> {
    let (head, order) = match k.find(":order=") {
        Some(i) => (&k[..i], Some(&k[i + ":order=".len()..])),
        None => (k, None),
    };
    let mut name = head;
    let mut descending = false;
    let mut builtin: Option<&[&str]> = None;
    loop {
        let Some((rest, last)) = name.rsplit_once(':') else {
            break;
        };
        match last {
            "desc" => descending = true,
            "asc" => descending = false,
            _ => match BUILTIN_LISTS.iter().find(|b| b.0 == last) {
                Some(b) if builtin.is_none() => builtin = Some(b.2),
                _ => break,
            },
        }
        name = rest;
    }
    let col =
        col_of(d, header, name).map_err(|e| match name.rsplit_once(':') {
            Some((_, w)) if !w.is_empty() => {
                format!(
            "{e} (or \"{w}\" is not an order: the ready-made ones are {}; or give order=A|B|C)",
            BUILTIN_LISTS.iter().map(|b| b.0).collect::<Vec<_>>().join(", ")
        )
            }
            _ => e,
        })?;
    let items = match (order, builtin) {
        (Some(_), Some(_)) => {
            return Err(format!("{k}: give a ready-made order or order=, not both"))
        }
        (Some(o), None) => Some(match o.strip_prefix('@') {
            Some(path) => {
                let meta =
                    std::fs::metadata(path).map_err(|e| format!("cannot read {path}: {e}"))?;
                if meta.len() > (MAX_LIST_ITEMS * (MAX_ITEM_CHARS + 2)) as u64 {
                    return Err(format!("{path} is too big for a custom order"));
                }
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("cannot read {path}: {e}"))?;
                order_items(text.lines().map(str::to_string))?
            }
            None => order_items(split_escaped(o, '|', true))?,
        }),
        (None, Some(b)) => Some(b.iter().map(|s| s.to_string()).collect()),
        (None, None) => None,
    };
    let kind = match items {
        Some(items) => {
            if lists.len() >= MAX_SORT_LISTS {
                return Err(format!(
                    "at most {MAX_SORT_LISTS} custom orders in one sort"
                ));
            }
            lists.push(items);
            SortKind::List((lists.len() - 1) as u16)
        }
        None => SortKind::Auto,
    };
    Ok(SortKey {
        col,
        descending,
        kind,
        match_case: false,
    })
}

/// `sort <in> -o <out> --by a[:desc],b[:desc][:order=X|Y|Z]`.
pub fn sort(args: &[&str]) -> Result<(), String> {
    let o = opts(args, &["--by"], &[])?;
    let input = one_input(&o)?;
    let mut d = load(input, sheet(&o)?)?;
    let mut keys = Vec::new();
    let mut lists = Vec::new();
    for k in split_escaped(o.get("--by").ok_or("give --by")?, ',', false) {
        keys.push(sort_key(&d, o.header(), &k, &mut lists)?);
    }
    step(
        &mut d,
        RecipeOp::Sort {
            spec: SortSpec {
                keys,
                keep_first: o.header(),
                lists,
                ..Default::default()
            },
        },
    )?;
    let msg = write_out(&mut d, input, &out_of(&o)?)?;
    done(&o, msg, json!({}));
    Ok(())
}

/// `dedupe <in> -o <out> [--columns a,b] [--match-case]`: remove repeats
/// (the first copy stays).
pub fn dedupe(args: &[&str]) -> Result<(), String> {
    let o = opts(args, &["--columns"], &["--match-case"])?;
    let input = one_input(&o)?;
    let mut d = load(input, sheet(&o)?)?;
    let cols = match o.get("--columns") {
        Some(c) => cols_of(&d, o.header(), c)?,
        None => Vec::new(),
    };
    let before = d.row_count();
    step(
        &mut d,
        RecipeOp::RemoveDuplicates {
            spec: Duplicates {
                cols,
                match_case: o.has("--match-case"),
                trim: true,
                keep_first: o.header(),
                ..Default::default()
            },
        },
    )?;
    let removed = before - d.row_count();
    let msg = write_out(&mut d, input, &out_of(&o)?)?;
    if !o.has("--json") {
        let _ = crate::outln!("{removed} repeated rows removed");
    }
    done(&o, msg, json!({"removed": removed}));
    Ok(())
}

/// `stats <in> [--columns a,b]`: each column's profile.
pub fn stats(args: &[&str]) -> Result<(), String> {
    let o = opts(args, &["--columns"], &[])?;
    let d = load(one_input(&o)?, sheet(&o)?)?;
    let cols = match o.get("--columns") {
        Some(c) => cols_of(&d, o.header(), c)?,
        None => (0..d.col_count()).collect(),
    };
    let titles = d.titles(&cols, o.header());
    let mut all = Vec::new();
    for (&c, title) in cols.iter().zip(&titles) {
        let p = d
            .column_profile(
                &ProfileSpec {
                    col: c,
                    keep_first: o.header(),
                    ..Default::default()
                },
                &Progress::new(),
            )
            .map_err(|e| e.to_string())?;
        if o.has("--json") {
            let mut v = serde_json::to_value(&p).map_err(|e| e.to_string())?;
            v["title"] = json!(title);
            all.push(v);
            continue;
        }
        let filled = p.rows - p.blanks;
        let _ = crate::outln!("{} ({})", title, c + 1);
        let _ = crate::outln!("  {} rows, {} filled, {} blank", p.rows, filled, p.blanks);
        let shapes: Vec<String> = p
            .shapes
            .iter()
            .take(3)
            .map(|s| {
                format!(
                    "{} {:.1}%",
                    s.shape,
                    100.0 * s.count as f64 / filled.max(1) as f64
                )
            })
            .collect();
        if !shapes.is_empty() {
            let _ = crate::outln!("  shapes: {}", shapes.join(", "));
        }
        if let (Some(m), Some(sd)) = (p.mean, p.sd) {
            let _ = crate::outln!(
                "  numbers: mean {m:.4}, standard deviation {sd:.4}, {} far out",
                p.outliers
            );
        }
        if let (Some(a), Some(b)) = (&p.shortest, &p.longest) {
            let cut = |s: &str| -> String {
                if s.chars().count() > 40 {
                    format!("{}…", s.chars().take(40).collect::<String>())
                } else {
                    s.to_string()
                }
            };
            let _ = crate::outln!(
                "  shortest \"{}\" ({}), longest \"{}\" ({})",
                cut(a),
                p.min_len,
                cut(b),
                p.max_len
            );
        }
    }
    if o.has("--json") {
        print_json(&json!(all));
    }
    Ok(())
}

/// `count <in> --columns a[,b] [-o <out>]`: each value (or combination)
/// with its count, most common first.
pub fn count(args: &[&str]) -> Result<(), String> {
    let o = opts(args, &["--columns"], &[])?;
    let input = one_input(&o)?;
    let d = load(input, sheet(&o)?)?;
    let cols = cols_of(&d, o.header(), o.get("--columns").ok_or("give --columns")?)?;
    let spec = CountsSpec {
        cols,
        keep_first: o.header(),
    };
    if o.get("-o").is_some() {
        let (t, c) = d
            .counts_table(&spec, &Progress::new())
            .map_err(|e| e.to_string())?;
        let mut t = t.into_document().map_err(|e| e.to_string())?;
        let msg = write_out(&mut t, input, &out_of(&o)?)?;
        done(&o, msg, json!({"values": c.lines.len(), "rows": c.rows}));
        return Ok(());
    }
    let c = d
        .value_counts(&spec, &Progress::new())
        .map_err(|e| e.to_string())?;
    if o.has("--json") {
        print_json(&serde_json::to_value(&c).map_err(|e| e.to_string())?);
        return Ok(());
    }
    for l in c.lines.iter().take(MAX_PRINTED) {
        let pct = 100.0 * l.count as f64 / c.rows.max(1) as f64;
        let v = l
            .values
            .iter()
            .map(|v| if v.is_empty() { "(blank)" } else { v.as_str() })
            .collect::<Vec<_>>()
            .join(" | ");
        let _ = crate::outln!("{:>10}  {pct:5.1}%  {v}", l.count);
    }
    if c.lines.len() > MAX_PRINTED {
        let _ = crate::outln!(
            "… {} more (write them all with -o)",
            c.lines.len() - MAX_PRINTED
        );
    }
    Ok(())
}

/// A shape typed as its label (`INV-#####`): the last run of `#` is the
/// number, padded with zeros when it is longer than one.
fn shape_from(label: &str) -> Result<Pattern, String> {
    let end = label
        .rfind('#')
        .ok_or("--shape takes a shape with # for the number, like INV-#####")?
        + 1;
    let from = label[..end].trim_end_matches('#').len();
    let width = (end - from) as u32;
    Ok(Pattern {
        prefix: label[..from].to_string(),
        suffix: label[end..].to_string(),
        width: if width > 1 { width } else { 0 },
    })
}

/// `--step day|business|week|month|N` (days).
fn date_step(text: &str) -> Result<DateStep, String> {
    Ok(match text {
        "day" => DateStep::Day,
        "business" => DateStep::Business,
        "week" => DateStep::Week,
        "month" => DateStep::Month,
        n => DateStep::Days {
            n: n.parse()
                .map_err(|_| "--step takes day, business, week, month or a number of days")?,
        },
    })
}

/// `gaps <in> --column <c>`: the numbers missing from a column. `Ok(false)`
/// with `--fail-on-gap` and at least one gap.
pub fn gaps(args: &[&str], json_out: bool) -> Result<bool, String> {
    let o = opts(
        args,
        &[
            "--column", "--series", "--digits", "--shape", "--from", "--to", "--step",
        ],
        &["--each-shape", "--fail-on-gap", "--dates"],
    )?;
    let input = one_input(&o)?;
    let d = load(input, sheet(&o)?)?;
    let header = o.header();
    let col = col_of(&d, header, o.get("--column").ok_or("give --column")?)?;
    let series = o
        .values
        .iter()
        .filter(|(n, _)| *n == "--series")
        .map(|(_, v)| col_of(&d, header, v))
        .collect::<Result<Vec<_>, _>>()?;
    let run = match o.get("--digits") {
        Some(v) => Some(
            v.parse::<u32>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .ok_or("--digits takes which run of digits, counted from the right, from 1")?,
        ),
        None => None,
    };
    let spec = GapsSpec {
        col,
        series: series.clone(),
        keep_first: header,
        run,
        pattern: o.get("--shape").map(shape_from).transpose()?,
        each_shape: o.has("--each-shape"),
        dates: match (o.has("--dates"), o.get("--step")) {
            (false, None) => None,
            (false, Some(_)) => return Err("--step goes with --dates".into()),
            (true, step) => Some(date_step(step.unwrap_or("day"))?),
        },
        start: o.get("--from").unwrap_or_default().to_string(),
        end: o.get("--to").unwrap_or_default().to_string(),
    };
    let r = d
        .gap_audit(&spec, &Progress::new())
        .map_err(|e| e.to_string())?;
    if json_out {
        print_json(&serde_json::to_value(&r).map_err(|e| e.to_string())?);
    } else {
        if let Some(p) = &r.pattern {
            let _ = crate::outln!("shape {}", p.label());
        }
        let _ = crate::outln!(
            "{} rows, {} different numbers, {} missing in {} ranges",
            r.rows,
            r.distinct,
            r.missing,
            r.range_total
        );
        let _ = crate::outln!(
            "{} repeated numbers, {} drops in file order, {} of another shape, {} blank",
            r.duplicate_values,
            r.drops,
            r.other_shape,
            r.blank
        );
        if let Some(d) = &r.dates {
            for (n, what) in [
                (
                    d.ambiguous,
                    "ambiguous dates not read (some gaps may be these)",
                ),
                (d.two_digit, "two-digit years"),
                (d.weekend, "on a weekend"),
                (d.off_step, "off the step"),
            ] {
                if n > 0 {
                    let _ = crate::outln!("{n} {what}");
                }
            }
        }
        if r.looks_like_amounts {
            let _ = crate::outln!("note: this column looks like amounts, not IDs");
        }
        let _ = crate::out!("{}", r.copy_text());
    }
    Ok(!(o.has("--fail-on-gap") && r.range_total > 0))
}

/// `file.csv:column` (several joined with `+`; a number as `#3`).
fn list_arg(arg: &str) -> Result<(&str, Vec<&str>), String> {
    let bad = || {
        format!("give each list as file:column, like voters.csv:ID or voters.csv:#1+#4 (not {arg})")
    };
    // The last ":" splits, so a drive letter stays with the path.
    let (path, cols) = arg.rsplit_once(':').ok_or_else(bad)?;
    if path.len() < 2 || cols.is_empty() || cols.contains(['\\', '/']) {
        return Err(bad());
    }
    Ok((path, cols.split('+').collect()))
}

fn list_col(d: &Document, header: bool, name: &str) -> Result<u32, String> {
    match name.strip_prefix('#') {
        Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => col_of(d, header, n),
        _ => col_of(d, header, name).map_err(|e| {
            format!("{e} (a title with \":\" or \"+\" in it can be given by number, like #3)")
        }),
    }
}

/// `a,b,!c`: which lists a slice is in and out of; every list is named.
fn slice_mask(text: &str, lists: usize) -> Result<u8, String> {
    let bad = || format!("--slice names each of the {lists} lists, in or out, like a,!b");
    let mut named = 0u8;
    let mut mask = 0u8;
    for part in text.split(',') {
        let part = part.trim();
        let (out, letter) = match part.strip_prefix('!') {
            Some(l) => (true, l),
            None => (false, part),
        };
        let i = list_letter(letter, lists).ok_or_else(bad)?;
        if named & (1 << i) != 0 {
            return Err(bad());
        }
        named |= 1 << i;
        if !out {
            mask |= 1 << i;
        }
    }
    if named.count_ones() as usize != lists || mask == 0 {
        return Err(bad());
    }
    Ok(mask)
}

/// `a` is the first list.
fn list_letter(text: &str, lists: usize) -> Option<usize> {
    let [c] = text.as_bytes() else {
        return None;
    };
    let i = usize::from(c.to_ascii_lowercase().checked_sub(b'a')?);
    (i < lists).then_some(i)
}

/// `sets a.csv:ID b.csv:ID ...`: how many keys the lists share, slice by
/// slice; `--slice` with `-o` writes one slice's rows or keys.
pub fn sets(args: &[&str]) -> Result<(), String> {
    let o = opts(
        args,
        &["--slice", "--rows-of"],
        &["--match-case", "--no-trim", "--loose", "--keys-only"],
    )?;
    if !(2..=MAX_LISTS).contains(&o.plain.len()) {
        return Err(format!("give 2 to {MAX_LISTS} lists\n{USAGE}"));
    }
    let header = o.header();
    let mut docs = Vec::new();
    let mut spec = SetsSpec {
        lists: Vec::new(),
        match_case: o.has("--match-case"),
        trim: !o.has("--no-trim"),
        loose: o.has("--loose"),
    };
    let mut names = Vec::new();
    let mut paths = Vec::new();
    for arg in &o.plain {
        let (path, cols) = list_arg(arg)?;
        let d = load(path, sheet(&o)?)?;
        let cols = cols
            .iter()
            .map(|c| list_col(&d, header, c))
            .collect::<Result<Vec<_>, _>>()?;
        spec.lists.push(SetList { cols, header });
        names.push(
            Path::new(path)
                .file_stem()
                .map_or_else(|| path.to_string(), |s| s.to_string_lossy().into_owned()),
        );
        paths.push(path);
        docs.push(d);
    }
    // Lists that would read the same are told apart by their letter.
    let names: Vec<String> = names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            if names.iter().filter(|x| *x == n).count() > 1 {
                format!("{n} ({})", char::from(b'a' + i as u8))
            } else {
                n.clone()
            }
        })
        .collect();
    let slice = o
        .get("--slice")
        .map(|s| slice_mask(s, docs.len()))
        .transpose()?;
    if slice.is_none() && (o.get("-o").is_some() || o.get("--rows-of").is_some()) {
        return Err("say which slice to write with --slice, like a,!b".into());
    }
    let refs: Vec<&Document> = docs.iter().collect();
    let r = compare_lists(&refs, &spec, &Progress::new()).map_err(|e| e.to_string())?;
    if let Some(mask) = slice {
        let list = match o.get("--rows-of") {
            Some(l) => list_letter(l, docs.len())
                .filter(|i| mask & (1 << i) != 0)
                .ok_or("--rows-of takes the letter of a list in the slice")?,
            None => mask.trailing_zeros() as usize,
        };
        let out = out_of(&o)?;
        // Never over one of the lists ([`write_out`] guards the one written).
        let target = std::fs::canonicalize(out.path).ok();
        if target.is_some()
            && paths
                .iter()
                .any(|p| std::fs::canonicalize(p).ok() == target)
        {
            return Err("write to a different file than the lists".into());
        }
        let d = &docs[list];
        let keys_only = o.has("--keys-only");
        let table = if keys_only {
            let mut exact = vec![false; spec.lists[0].cols.len()];
            for (d, l) in docs.iter().zip(&spec.lists) {
                for (a, b) in exact.iter_mut().zip(d.exact_key_parts(&l.cols)) {
                    *a |= b;
                }
            }
            d.sets_keys_table(
                &spec,
                &exact,
                list,
                &r.row_masks[list],
                mask,
                &Progress::new(),
            )
        } else {
            d.sets_slice_table(
                &spec.lists[list],
                &r.row_masks[list],
                mask,
                &Progress::new(),
            )
        }
        .map_err(|e| e.to_string())?;
        let rows = table.rows.saturating_sub(u64::from(header || keys_only));
        if table.rows == 0 {
            return Err("that slice is empty: nothing to write".into());
        }
        let mut t = table.into_document().map_err(|e| e.to_string())?;
        let msg = write_out(&mut t, paths[list], &out)?;
        done(
            &o,
            msg,
            json!({
                "slice": slice_name(mask, &names),
                "rows": rows,
                "keys": r.report.slices[usize::from(mask) - 1].keys,
            }),
        );
        return Ok(());
    }
    if o.has("--json") {
        let mut v = serde_json::to_value(&r.report).map_err(|e| e.to_string())?;
        v["names"] = json!(names);
        print_json(&v);
        return Ok(());
    }
    for s in r.report.slices.iter().filter(|s| s.keys > 0) {
        let rows: Vec<String> = s.rows.iter().map(u64::to_string).collect();
        let _ = crate::outln!(
            "{:>10} keys  {}  (rows: {})",
            s.keys,
            slice_name(s.mask, &names),
            rows.join(", ")
        );
    }
    for (l, name) in r.report.lists.iter().zip(&names) {
        let _ = crate::outln!(
            "{name}: {} rows, {} different keys, {} blank",
            l.rows,
            l.keys,
            l.blank
        );
    }
    Ok(())
}

#[cfg(test)]
mod sets_args {
    use super::*;

    #[test]
    fn lists_and_slices_are_read_or_refused() {
        assert_eq!(
            list_arg(r"C:\x\a.csv:ID").unwrap(),
            (r"C:\x\a.csv", vec!["ID"])
        );
        assert_eq!(
            list_arg("a.csv:#3+#5").unwrap(),
            ("a.csv", vec!["#3", "#5"])
        );
        for bad in ["a.csv", r"C:\x\a.csv", "a.csv:", ":ID"] {
            assert!(list_arg(bad).is_err(), "{bad}");
        }
        assert_eq!(slice_mask("a,b,!c", 3).unwrap(), 3);
        assert_eq!(slice_mask("!a, B", 2).unwrap(), 2);
        for bad in ["a", "a,a", "a,b,c,d", "!a,!b", "a,x", ""] {
            assert!(slice_mask(bad, 2).is_err(), "{bad}");
        }
    }
}

/// `sample <in> -o <out> (--rows N | --percent P | --every N) [--seed S]`.
pub fn sample(args: &[&str]) -> Result<(), String> {
    let o = opts(args, &["--rows", "--percent", "--every", "--seed"], &[])?;
    let input = one_input(&o)?;
    let d = load(input, sheet(&o)?)?;
    let num = |k: &str| -> Result<Option<u64>, String> {
        o.get(k)
            .map(|v| {
                v.parse::<u64>()
                    .map_err(|_| format!("{k} takes a whole number"))
            })
            .transpose()
    };
    let size = match (num("--rows")?, o.get("--percent"), num("--every")?) {
        (Some(n), None, None) => SampleSize::Rows { n },
        (None, Some(p), None) => SampleSize::Percent {
            p: p.parse().map_err(|_| "--percent takes a number")?,
        },
        (None, None, Some(n)) if n > 0 => SampleSize::Every { n },
        _ => return Err("give one of --rows, --percent or --every".into()),
    };
    let seed = num("--seed")?.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |t| t.as_nanos() as u64)
    });
    let t = d
        .sample(
            &Sample {
                size,
                seed,
                keep_first: o.header(),
                ..Default::default()
            },
            &Progress::new(),
        )
        .map_err(|e| e.to_string())?;
    let mut t = t.into_document().map_err(|e| e.to_string())?;
    let msg = write_out(&mut t, input, &out_of(&o)?)?;
    done(&o, format!("{msg} (seed {seed})"), json!({"seed": seed}));
    Ok(())
}

/// A delimited text file written row by row (for `slice`, `split`, `cat`).
struct Rows {
    w: BufWriter<std::fs::File>,
    sep: u8,
    rows: u64,
}

impl Rows {
    fn create(path: &Path, sep: u8, overwrite: bool) -> Result<Rows, String> {
        Rows::create_with(path, sep, overwrite, 1 << 20)
    }

    /// `buffer`: bytes kept before writing (smaller when many are open).
    fn create_with(path: &Path, sep: u8, overwrite: bool, buffer: usize) -> Result<Rows, String> {
        if path.exists() && !overwrite {
            return Err(format!(
                "{} already exists; add --overwrite to replace it",
                path.display()
            ));
        }
        let f = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Rows {
            w: BufWriter::with_capacity(buffer, f),
            sep,
            rows: 0,
        })
    }

    fn row(&mut self, cells: &[String]) -> Result<(), String> {
        let sep = self.sep as char;
        let mut line = String::new();
        for (i, c) in cells.iter().enumerate() {
            if i > 0 {
                line.push(sep);
            }
            if c.contains(sep) || c.contains('"') || c.contains('\r') || c.contains('\n') {
                line.push('"');
                line.push_str(&c.replace('"', "\"\""));
                line.push('"');
            } else {
                line.push_str(c);
            }
        }
        line.push_str("\r\n");
        self.w
            .write_all(line.as_bytes())
            .map_err(|e| e.to_string())?;
        self.rows += 1;
        Ok(())
    }

    fn finish(mut self) -> Result<u64, String> {
        self.w.flush().map_err(|e| e.to_string())?;
        Ok(self.rows)
    }
}

fn sep_of(o: &Opts, path: &str) -> Result<u8, String> {
    let kind = o
        .get("--to")
        .map(str::to_ascii_lowercase)
        .or_else(|| {
            Path::new(path)
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
        })
        .unwrap_or_else(|| "csv".into());
    match kind.as_str() {
        "csv" => Ok(b','),
        "tsv" | "tab" | "txt" => Ok(b'\t'),
        "psv" => Ok(b'|'),
        "ssv" => Ok(b';'),
        k => Err(format!(
            "{k}: slice, split and cat write delimited text (csv, tsv, psv)"
        )),
    }
}

fn same_file(a: &str, b: &Path) -> bool {
    matches!(
        (std::fs::canonicalize(a), std::fs::canonicalize(b)),
        (Ok(x), Ok(y)) if x == y
    )
}

/// `slice <in> -o <out> --rows 100-200`: data rows 100 to 200 (from 1),
/// with the title row.
pub fn slice(args: &[&str]) -> Result<(), String> {
    let o = opts(args, &["--rows"], &[])?;
    let input = one_input(&o)?;
    let d = load(input, sheet(&o)?)?;
    let range = o.get("--rows").ok_or("give --rows, like 100-200")?;
    let (a, b) = range
        .split_once('-')
        .ok_or("--rows takes a range like 100-200")?;
    let first: u64 = a
        .trim()
        .parse()
        .map_err(|_| "--rows takes a range like 100-200")?;
    let last: u64 = if b.trim().is_empty() {
        u64::MAX
    } else {
        b.trim()
            .parse()
            .map_err(|_| "--rows takes a range like 100-200")?
    };
    if first == 0 || last < first {
        return Err("--rows counts from 1, the first number no bigger than the second".into());
    }
    let out = out_of(&o)?;
    let path = Path::new(out.path);
    if same_file(input, path) {
        return Err("write to a different file than the input".into());
    }
    let mut w = Rows::create(path, sep_of(&o, out.path)?, out.overwrite)?;
    let head = u64::from(o.header() && d.row_count() > 0);
    if head == 1 {
        w.row(&d.row_values(0).map_err(|e| e.to_string())?)?;
    }
    let end = d
        .row_count()
        .min(head + last.saturating_sub(1).saturating_add(1));
    for r in (head + first - 1)..end {
        w.row(&d.row_values(r).map_err(|e| e.to_string())?)?;
    }
    let n = w.finish()?;
    done(
        &o,
        format!("saved {n} rows to {}", out.path),
        json!({"rows": n}),
    );
    Ok(())
}

/// `split <in> -o <folder> (--rows N | --by <column>)`: parts of N data
/// rows, or one file per value of a column; each keeps the title row.
/// `base.ext`, or `base (2).ext` and so on when that name (any case) is
/// already taken; the name is then taken.
fn part_name(base: &str, ext: &str, taken: &mut std::collections::HashSet<String>) -> String {
    let mut name = format!("{base}.{ext}");
    let mut n = 2;
    while !taken.insert(name.to_lowercase()) {
        name = format!("{base} ({n}).{ext}");
        n += 1;
    }
    name
}

pub fn split(args: &[&str]) -> Result<(), String> {
    let o = opts(args, &["--rows", "--by"], &[])?;
    let input = one_input(&o)?;
    let d = load(input, sheet(&o)?)?;
    let dir = Path::new(o.get("-o").ok_or("give the output folder with -o")?);
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stem = Path::new(input)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "part".into());
    let ext = match o.get("--to") {
        Some(t) => t.to_ascii_lowercase(),
        None => Path::new(input)
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .filter(|e| matches!(e.as_str(), "csv" | "tsv" | "txt" | "psv"))
            .unwrap_or_else(|| "csv".into()),
    };
    let sep = sep_of(&o, &format!("x.{ext}"))?;
    let head = u64::from(o.header() && d.row_count() > 0);
    let title = if head == 1 {
        Some(d.row_values(0).map_err(|e| e.to_string())?)
    } else {
        None
    };
    let overwrite = o.has("--overwrite");
    let mut written: Vec<(String, u64)> = Vec::new();
    match (o.get("--rows"), o.get("--by")) {
        (Some(n), None) => {
            let n: u64 = n
                .parse()
                .ok()
                .filter(|&n| n > 0)
                .ok_or("--rows takes a whole number")?;
            let rows = d.row_count() - head;
            let parts = rows.div_ceil(n);
            if parts > MAX_PARTS {
                return Err(format!("that makes {parts} files; at most {MAX_PARTS}"));
            }
            for p in 0..parts {
                let name = format!("{stem} part {}.{ext}", p + 1);
                let path = dir.join(&name);
                if same_file(input, &path) {
                    return Err("a part would land on the input file".into());
                }
                let mut w = Rows::create(&path, sep, overwrite)?;
                if let Some(t) = &title {
                    w.row(t)?;
                }
                for r in (head + p * n)..(head + ((p + 1) * n).min(rows)) {
                    w.row(&d.row_values(r).map_err(|e| e.to_string())?)?;
                }
                written.push((name, w.finish()?));
            }
        }
        (None, Some(by)) => {
            let c = col_of(&d, o.header(), by)?;
            let mut files: std::collections::HashMap<String, (String, Rows)> =
                std::collections::HashMap::new();
            let mut order = Vec::new();
            // File names given so far, compared as Windows does (any case);
            // two values that clean to one name get (2), (3) and so on.
            let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
            for r in head..d.row_count() {
                let row = d.row_values(r).map_err(|e| e.to_string())?;
                let v = row.get(c as usize).cloned().unwrap_or_default();
                let key = v.trim().to_lowercase();
                if !files.contains_key(&key) {
                    if files.len() as u64 >= MAX_PARTS {
                        return Err(format!("more than {MAX_PARTS} different values"));
                    }
                    let safe: String = v
                        .trim()
                        .chars()
                        .map(|ch| {
                            if r#"<>:"/\|?*"#.contains(ch) || ch.is_control() {
                                '_'
                            } else {
                                ch
                            }
                        })
                        .take(80)
                        .collect();
                    // Windows drops trailing dots and spaces from a name.
                    let safe = safe.trim_end_matches(['.', ' ']).to_string();
                    let base = format!(
                        "{stem} {}",
                        if safe.is_empty() {
                            "(blank)".to_string()
                        } else {
                            safe
                        }
                    );
                    let name = part_name(&base, &ext, &mut taken);
                    let path = dir.join(&name);
                    if same_file(input, &path) {
                        return Err("a part would land on the input file".into());
                    }
                    let mut w = Rows::create_with(&path, sep, overwrite, 64 << 10)?;
                    if let Some(t) = &title {
                        w.row(t)?;
                    }
                    order.push(key.clone());
                    files.insert(key.clone(), (name, w));
                }
                if let Some((_, w)) = files.get_mut(&key) {
                    w.row(&row)?;
                }
            }
            for k in order {
                if let Some((name, w)) = files.remove(&k) {
                    written.push((name, w.finish()?));
                }
            }
        }
        _ => return Err("give --rows N or --by <column>".into()),
    }
    if o.has("--json") {
        print_json(&json!(written
            .iter()
            .map(|(n, r)| json!({"file": n, "rows": r}))
            .collect::<Vec<_>>()));
    } else {
        for (n, r) in &written {
            let _ = crate::outln!("{r:>10}  {n}");
        }
        let _ = crate::outln!("{} files written to {}", written.len(), dir.display());
    }
    Ok(())
}

/// The data rows of one file of `cat --by-name`, each value moved to its
/// output column (`map`) among `width`.
#[inline(never)]
fn write_by_name(
    w: &mut Rows,
    d: &Document,
    head: u64,
    map: &[Option<u32>],
    width: usize,
    input: &str,
) -> Result<(), String> {
    // A file whose columns already are the output's needs no moving.
    let as_is = map.len() == width && map.iter().enumerate().all(|(i, j)| *j == Some(i as u32));
    for r in head..d.row_count() {
        let row = d.row_values(r).map_err(|e| e.to_string())?;
        if as_is && row.len() == width {
            w.row(&row)?;
            continue;
        }
        let mut cells = vec![String::new(); width];
        for (i, v) in row.into_iter().enumerate() {
            if let Some(j) = map.get(i) {
                // A column the map leaves out.
                if let Some(j) = j {
                    cells[*j as usize] = v;
                }
            } else if !v.is_empty() {
                return Err(format!(
                    "{input}: row {} has a value beyond its last column",
                    r + 1
                ));
            }
        }
        w.row(&cells)?;
    }
    Ok(())
}

/// `cat <in> <in> … -o <out>`: stack files with the same columns (the
/// title row once). With `--by-name` columns are lined up by title and every
/// file's columns are kept.
pub fn cat(args: &[&str]) -> Result<(), String> {
    cat_holding(args, MAX_CAT_HELD, &|| {})
}

/// A file's title row, one cell for every column it has.
fn title_row(d: &Document) -> Result<Vec<String>, String> {
    if d.row_count() == 0 {
        return Ok(Vec::new());
    }
    let mut t = d.row_values(0).map_err(|e| e.to_string())?;
    t.resize(d.col_count() as usize, String::new());
    Ok(t)
}

/// [`cat`], keeping at most `most_held` input bytes open between the two
/// passes of `--by-name`; `between` runs between them (tests).
fn cat_holding(args: &[&str], most_held: u64, between: &dyn Fn()) -> Result<(), String> {
    let o = opts(args, &["--mapping"], &["--by-name", "--strict", "--plan"])?;
    if o.plain.len() < 2 {
        return Err("give two or more files to stack".into());
    }
    if o.plain.len() > MAX_CAT_FILES {
        return Err(format!("stack at most {MAX_CAT_FILES} files at once"));
    }
    let mapping = match o.get("--mapping") {
        Some(m) => {
            if !o.header() {
                return Err("a column map is for files that start with a row of titles".into());
            }
            let bytes = crate::safe::read_small(
                m,
                natendo_engine::combine::MAX_MAPPING_BYTES as u64,
                "a column map",
            )?;
            Some(
                natendo_engine::combine::SavedMapping::from_json(&bytes)
                    .map_err(|e| format!("{m}: {e}"))?,
            )
        }
        None => None,
    };
    if o.has("--strict") && mapping.is_none() {
        return Err("--strict goes with --mapping".into());
    }
    let plan_only = o.has("--plan");
    if plan_only && !(o.has("--by-name") || mapping.is_some()) {
        return Err("--plan shows how columns line up by name: add --by-name or --mapping".into());
    }
    let out = if plan_only { None } else { Some(out_of(&o)?) };
    if let Some(out) = &out {
        let path = Path::new(out.path);
        if o.plain.iter().any(|i| same_file(i, path)) {
            return Err("write to a different file than the inputs".into());
        }
    }
    let by_name = (o.has("--by-name") || mapping.is_some()) && o.header();
    let sheet = sheet(&o)?;
    // By name, the output has every file's columns, so all title rows are
    // read first. Files are kept open up to a size; larger sets are read again.
    let mut union = TitleUnion::default();
    let mut maps: Vec<Vec<Option<u32>>> = Vec::new();
    let mut heads: Vec<natendo_engine::combine::FileHead> = Vec::new();
    let mut kept: Vec<Option<Document>> = Vec::new();
    // Each file's titles as the column maps were made from them.
    let mut seen: Vec<Vec<String>> = Vec::new();
    if by_name {
        let mut held = 0u64;
        for input in &o.plain {
            let d = load(input, sheet)?;
            let t = title_row(&d)?;
            maps.push(union.add(&t).into_iter().map(|i| Some(i as u32)).collect());
            if plan_only {
                heads.push(head_of_doc(input, &d, &t)?);
            }
            if union.len() > MAX_CAT_COLS {
                return Err(format!(
                    "{input}: more than {MAX_CAT_COLS} different columns"
                ));
            }
            seen.push(t);
            held = held.saturating_add(std::fs::metadata(input).map_or(u64::MAX, |m| m.len()));
            kept.push((held <= most_held).then_some(d));
        }
    }
    between();
    let mut titles = union.into_titles();
    let mut fit_report = json!(null);
    if let Some(m) = &mapping {
        // The titles come from the files being stacked, so the map cannot drift from them.
        let mut fit = m.apply(&seen);
        fit.plan.compact();
        if let Some(p) = fit.problems.first() {
            return Err(format!(
                "{}: {}; the column map cannot decide (nothing was written)",
                o.plain[p.file], p.what
            ));
        }
        let name = |f: usize| o.plain[f];
        let missing: Vec<String> = fit
            .missing
            .iter()
            .map(|&(f, c)| format!("{} has no \"{}\"", name(f), m.columns[c as usize].title))
            .collect();
        let extras: Vec<String> = fit
            .extras
            .iter()
            .map(|&(f, c)| format!("{}: \"{}\" is not in the map", name(f), seen[f][c as usize]))
            .collect();
        if o.has("--strict") && !(missing.is_empty() && extras.is_empty()) {
            return Err(format!(
                "the files do not fit the column map exactly (--strict; nothing was written): {}",
                missing
                    .iter()
                    .chain(&extras)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        if !o.has("--json") {
            for line in missing.iter().chain(&extras) {
                crate::errln!("note: {line}");
            }
        }
        fit_report = json!({ "missing": missing, "extras": extras });
        titles = fit.plan.titles;
        maps = fit.plan.files;
    }
    if plan_only {
        let plan = natendo_engine::combine::CombinePlan {
            titles: titles.clone(),
            files: maps.clone(),
        };
        let suggestions = natendo_engine::combine::suggest(&plan, &heads);
        let mut text = format!("{} columns: {}", titles.len(), titles.join(", "));
        for (f, map) in maps.iter().enumerate() {
            let left = map.iter().filter(|m| m.is_none()).count();
            text += &format!(
                "\n{}: {} columns{}",
                o.plain[f],
                map.len(),
                if left > 0 {
                    format!(", {left} left out")
                } else {
                    String::new()
                }
            );
        }
        for s in &suggestions {
            text += &format!(
                "\nsuggestion: \"{}\" and \"{}\" may be one column",
                titles[s.keep as usize], titles[s.merge as usize]
            );
        }
        done(
            &o,
            text,
            json!({
                "titles": titles,
                "files": o.plain.iter().zip(&maps).map(|(n, m)| json!({ "file": n, "map": m })).collect::<Vec<_>>(),
                "suggestions": suggestions,
                "fit": fit_report,
            }),
        );
        return Ok(());
    }
    let out = out.ok_or("give the output file with -o")?;
    let path = Path::new(out.path);
    let sep = sep_of(&o, out.path)?;
    let mut w = Rows::create(path, sep, out.overwrite)?;
    if by_name && !titles.is_empty() {
        w.row(&titles)?;
    }
    let mut first_titles: Option<Vec<String>> = None;
    for (at, input) in o.plain.iter().enumerate() {
        let d = match kept.get_mut(at).and_then(Option::take) {
            Some(d) => d,
            None => {
                let d = load(input, sheet)?;
                // Saved in between, its values could land under wrong titles.
                if by_name && title_row(&d)? != seen[at] {
                    return Err(format!(
                        "{input} changed while it was being combined: run again"
                    ));
                }
                d
            }
        };
        let head = u64::from(o.header() && d.row_count() > 0);
        if head == 1 && !by_name {
            let t = d.row_values(0).map_err(|e| e.to_string())?;
            match &first_titles {
                None => {
                    w.row(&t)?;
                    first_titles = Some(t);
                }
                Some(f) => {
                    let norm = |s: &str| s.trim().to_lowercase();
                    if f.len() != t.len() || f.iter().zip(&t).any(|(a, b)| norm(a) != norm(b)) {
                        return Err(format!(
                            "{input}: its columns differ from the first file's (add --by-name to line them up by title)"
                        ));
                    }
                }
            }
        }
        if by_name {
            write_by_name(&mut w, &d, head, &maps[at], titles.len(), input)?;
        } else {
            for r in head..d.row_count() {
                w.row(&d.row_values(r).map_err(|e| e.to_string())?)?;
            }
        }
    }
    let n = w.finish()?;
    let mut extra = json!({"rows": n});
    if mapping.is_some() {
        extra["fit"] = fit_report;
    }
    done(&o, format!("saved {n} rows to {}", out.path), extra);
    Ok(())
}

/// The head `--plan` suggests from: the file's titles and first rows.
fn head_of_doc(
    input: &str,
    d: &Document,
    titles: &[String],
) -> Result<natendo_engine::combine::FileHead, String> {
    let rows = d.row_count();
    let preview = (1..rows.min(1 + natendo_engine::combine::PREVIEW_ROWS as u64))
        .map(|r| d.row_values(r).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    Ok(natendo_engine::combine::FileHead {
        name: Path::new(input)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        titles: titles.to_vec(),
        cols: d.col_count(),
        rows_est: rows.saturating_sub(1),
        whole: true,
        preview,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("nde-cli-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn s(p: &std::path::Path) -> String {
        p.display().to_string()
    }

    #[test]
    fn split_by_keeps_every_row_when_names_clash() {
        let d = dir("clash");
        let a = d.join("p.csv");
        let long = "x".repeat(90);
        std::fs::write(
            &a,
            format!(
                "k,n
a/b,1
a_b,2
a/b,3
{long}1,4
{long}2,5
end.,6
end,7
"
            ),
        )
        .unwrap();
        let out = d.join("parts");
        let (a, o) = (s(&a), s(&out));
        split(&[&a, "-o", &o, "--by", "k", "--overwrite"]).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names.len(), 6, "{names:?}");
        assert!(names.contains(&"p a_b.csv".to_string()));
        assert!(names.contains(&"p a_b (2).csv".to_string()));
        let rows: usize = names
            .iter()
            .map(|n| {
                std::fs::read_to_string(out.join(n))
                    .unwrap()
                    .lines()
                    .count()
                    - 1
            })
            .sum();
        assert_eq!(rows, 7, "every row is in a file");
        let first = std::fs::read_to_string(out.join("p a_b.csv")).unwrap();
        assert!(first.contains("a/b,1") && first.contains("a/b,3"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn cat_by_name_notices_a_file_changed_between_passes() {
        let d = dir("cat-changed");
        let (a, b, o) = (d.join("a.csv"), d.join("b.csv"), d.join("o.csv"));
        let write = || {
            std::fs::write(&a, "id,name\r\n1,Ann\r\n").unwrap();
            std::fs::write(&b, "name,id\r\nBo,2\r\n").unwrap();
        };
        let (a2, b2, o2) = (s(&a), s(&b), s(&o));
        let args = [&a2[..], &b2, "-o", &o2, "--by-name", "--overwrite"];
        // Nothing held: both files are read again, unchanged.
        write();
        cat_holding(&args, 0, &|| {}).unwrap();
        assert_eq!(
            std::fs::read_to_string(&o).unwrap(),
            "id,name\r\n1,Ann\r\n2,Bo\r\n"
        );
        for changed in ["id,name\r\n2,Bo\r\n", "name,id,more\r\nBo,2,x\r\n", ""] {
            write();
            let e = cat_holding(&args, 0, &|| std::fs::write(&b, changed).unwrap()).unwrap_err();
            assert!(e.contains("changed while it was being combined"), "{e}");
        }
        // A file held open is the one the map was made from.
        write();
        cat_holding(&args, MAX_CAT_HELD, &|| {}).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The column map is fitted to the titles of the files being stacked, and a
    /// file read again must still have them (it cannot drift from the map).
    #[test]
    fn cat_with_a_column_map_uses_the_streamed_files_titles() {
        let d = dir("cat-map");
        let (a, b, o, m) = (
            d.join("a.csv"),
            d.join("b.csv"),
            d.join("o.csv"),
            d.join("m.ndcombine"),
        );
        std::fs::write(&a, "Cust_ID,Name\r\n1,Ann\r\n").unwrap();
        std::fs::write(&b, "Name,CustID\r\nBo,2\r\n").unwrap();
        let map = natendo_engine::combine::SavedMapping {
            kind: natendo_engine::combine::MAPPING_KIND.into(),
            version: 1,
            tool: "t".into(),
            header: true,
            file_name_column: false,
            extras: natendo_engine::combine::Extras::Add,
            columns: vec![
                natendo_engine::combine::SavedColumn {
                    title: "Cust_ID".into(),
                    from: vec!["CustID".into()],
                },
                natendo_engine::combine::SavedColumn {
                    title: "Name".into(),
                    from: vec![],
                },
            ],
        };
        std::fs::write(&m, map.to_json()).unwrap();
        let (a2, b2, o2, m2) = (s(&a), s(&b), s(&o), s(&m));
        let args = [&a2[..], &b2, "-o", &o2, "--mapping", &m2, "--overwrite"];
        cat_holding(&args, 0, &|| {}).unwrap();
        assert_eq!(
            std::fs::read_to_string(&o).unwrap(),
            "Cust_ID,Name\r\n1,Ann\r\n2,Bo\r\n"
        );
        let e = cat_holding(&args, 0, &|| {
            std::fs::write(&b, "Full name,CustID\r\nBo,2\r\n").unwrap()
        })
        .unwrap_err();
        assert!(e.contains("changed while it was being combined"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn toolkit_commands() {
        let d = dir("tools");
        let a = d.join("a.csv");
        std::fs::write(
            &a,
            "name,party,amount\r\nAnn,D,10\r\nBo,R,5\r\nCy,D,7\r\nAnn,D,10\r\n",
        )
        .unwrap();
        let b = d.join("b.csv");
        std::fs::write(&b, "amount,name,party\r\n1,Ed,I\r\n").unwrap();
        let (a, b) = (s(&a), s(&b));
        let out = |n: &str| s(&d.join(n));
        let read = |n: &str| std::fs::read_to_string(d.join(n)).unwrap();
        select(&[&a, "-o", &out("s.csv"), "--columns", "amount,1"]).unwrap();
        assert_eq!(
            read("s.csv"),
            "amount,name\r\n10,Ann\r\n5,Bo\r\n7,Cy\r\n10,Ann\r\n"
        );
        filter(&[&a, "-o", &out("f.csv"), "--expr", "[amount] > 6"]).unwrap();
        assert_eq!(read("f.csv").lines().count(), 4);
        sort(&[&a, "-o", &out("o.csv"), "--by", "amount:desc,name"]).unwrap();
        assert!(read("o.csv").ends_with("Bo,R,5\r\n"));
        dedupe(&[&a, "-o", &out("d.csv")]).unwrap();
        assert_eq!(read("d.csv").lines().count(), 4);
        slice(&[&a, "-o", &out("l.csv"), "--rows", "2-3"]).unwrap();
        assert_eq!(read("l.csv"), "name,party,amount\r\nBo,R,5\r\nCy,D,7\r\n");
        split(&[&a, "-o", &out("parts"), "--rows", "3"]).unwrap();
        assert_eq!(std::fs::read_dir(d.join("parts")).unwrap().count(), 2);
        // Stacked by title: b's columns are in another order.
        assert!(cat(&[&a, &b, "-o", &out("c.csv")]).is_err());
        cat(&[&a, &b, "-o", &out("c.csv"), "--by-name", "--overwrite"]).unwrap();
        assert!(read("c.csv").ends_with("Ed,I,1\r\n"));
        count(&[&a, "--columns", "party", "-o", &out("n.csv")]).unwrap();
        assert!(read("n.csv").starts_with("party,Count,Percent\r\nD,3,"));
        sample(&[&a, "-o", &out("m.csv"), "--rows", "2", "--seed", "3"]).unwrap();
        assert_eq!(read("m.csv").lines().count(), 3);
        // Never over the input, never over a file without --overwrite.
        assert!(select(&[&a, "-o", &a, "--columns", "1"]).is_err());
        assert!(select(&[&a, "-o", &out("s.csv"), "--columns", "1"]).is_err());
        assert!(select(&[&a, "-o", &out("x.csv"), "--columns", "nope"]).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
