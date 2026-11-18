//! The command-line toolkit: `headers`, `select`, `filter`, `sort`,
//! `dedupe`, `stats`, `count`, `sample`, `slice`, `split` and `cat`. Each
//! is a thin wrapper over what the app does from its menus, with the same
//! limits. Columns are named by title or by number from 1.

use std::io::{BufWriter, Write};
use std::path::Path;

use natendo_engine::counts::CountsSpec;
use natendo_engine::expr::ComputeSpec;
use natendo_engine::profile::ProfileSpec;
use natendo_engine::recipe::RecipeOp;
use natendo_engine::reshape::Duplicates;
use natendo_engine::sort::{SortKey, SortSpec};
use natendo_engine::tools::{Sample, SampleSize};
use natendo_engine::{Document, Progress};
use serde_json::{json, Value};

use crate::more::{load, print_json, write_out, Out};
use crate::USAGE;

/// Files `cat` stacks at most.
const MAX_CAT_FILES: usize = 1_000;
/// Parts `split` writes at most.
const MAX_PARTS: u64 = 10_000;
/// Lines `count` prints at most (the counts file has them all).
const MAX_PRINTED: usize = 1_000;

/// The options of one command: values by name, flags, and the rest.
struct Opts<'a> {
    plain: Vec<&'a str>,
    values: Vec<(&'a str, &'a str)>,
    flags: Vec<&'a str>,
}

impl<'a> Opts<'a> {
    fn get(&self, k: &str) -> Option<&'a str> {
        self.values
            .iter()
            .rev()
            .find(|(n, _)| *n == k)
            .map(|(_, v)| *v)
    }
    fn has(&self, k: &str) -> bool {
        self.flags.contains(&k)
    }
    fn header(&self) -> bool {
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
fn col_of(d: &Document, header: bool, name: &str) -> Result<u32, String> {
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

fn cols_of(d: &Document, header: bool, list: &str) -> Result<Vec<u32>, String> {
    list.split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| col_of(d, header, s))
        .collect()
}

fn out_of<'a>(o: &Opts<'a>) -> Result<Out<'a>, String> {
    Ok(Out {
        path: o.get("-o").ok_or("give the output file with -o")?,
        to: o.get("--to"),
        header: o.header(),
        landscape: false,
        overwrite: o.has("--overwrite"),
    })
}

fn one_input<'a>(o: &Opts<'a>) -> Result<&'a str, String> {
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

fn opts<'a>(args: &[&'a str], more: &[&str], flags: &[&str]) -> Result<Opts<'a>, String> {
    let values: Vec<&str> = OUT_OPTS.iter().chain(more).copied().collect();
    let fl: Vec<&str> = OUT_FLAGS.iter().chain(flags).copied().collect();
    parse(args, &values, &fl)
}

fn sheet(o: &Opts) -> Result<u32, String> {
    match o.get("--sheet") {
        None => Ok(0),
        Some(s) => s
            .parse::<u32>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .ok_or_else(|| "--sheet takes a sheet number from 1".to_string()),
    }
}

fn done(o: &Opts, msg: String, extra: Value) {
    if o.has("--json") {
        let mut v = extra;
        v["message"] = json!(msg);
        print_json(&v);
    } else {
        println!("{msg}");
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
            println!("{:>4}  {t}", i + 1);
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
        println!("{errors} rows gave an error and were left out");
    }
    done(&o, msg, json!({"kept": kept, "errors": errors}));
    Ok(())
}

/// `sort <in> -o <out> --by a[:desc],b`.
pub fn sort(args: &[&str]) -> Result<(), String> {
    let o = opts(args, &["--by"], &[])?;
    let input = one_input(&o)?;
    let mut d = load(input, sheet(&o)?)?;
    let mut keys = Vec::new();
    for k in o.get("--by").ok_or("give --by")?.split(',') {
        let (name, desc) = match k.rsplit_once(':') {
            Some((n, "desc")) => (n, true),
            Some((n, "asc")) => (n, false),
            _ => (k, false),
        };
        keys.push(SortKey {
            col: col_of(&d, o.header(), name)?,
            descending: desc,
            kind: Default::default(),
            match_case: false,
        });
    }
    step(
        &mut d,
        RecipeOp::Sort {
            spec: SortSpec {
                keys,
                keep_first: o.header(),
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
        println!("{removed} repeated rows removed");
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
        println!("{} ({})", title, c + 1);
        println!("  {} rows, {} filled, {} blank", p.rows, filled, p.blanks);
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
            println!("  shapes: {}", shapes.join(", "));
        }
        if let (Some(m), Some(sd)) = (p.mean, p.sd) {
            println!(
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
            println!(
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
        println!("{:>10}  {pct:5.1}%  {v}", l.count);
    }
    if c.lines.len() > MAX_PRINTED {
        println!(
            "… {} more (write them all with -o)",
            c.lines.len() - MAX_PRINTED
        );
    }
    Ok(())
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
            println!("{r:>10}  {n}");
        }
        println!("{} files written to {}", written.len(), dir.display());
    }
    Ok(())
}

/// `cat <in> <in> … -o <out>`: stack files with the same columns (the
/// title row once).
pub fn cat(args: &[&str]) -> Result<(), String> {
    let o = opts(args, &[], &["--by-name"])?;
    if o.plain.len() < 2 {
        return Err("give two or more files to stack".into());
    }
    if o.plain.len() > MAX_CAT_FILES {
        return Err(format!("stack at most {MAX_CAT_FILES} files at once"));
    }
    let out = out_of(&o)?;
    let path = Path::new(out.path);
    if o.plain.iter().any(|i| same_file(i, path)) {
        return Err("write to a different file than the inputs".into());
    }
    let mut w = Rows::create(path, sep_of(&o, out.path)?, out.overwrite)?;
    let mut first_titles: Option<Vec<String>> = None;
    for input in &o.plain {
        let d = load(input, sheet(&o)?)?;
        let head = u64::from(o.header() && d.row_count() > 0);
        // Where each of this file's columns goes.
        let mut map: Option<Vec<Option<usize>>> = None;
        if head == 1 {
            let t = d.row_values(0).map_err(|e| e.to_string())?;
            match &first_titles {
                None => {
                    w.row(&t)?;
                    first_titles = Some(t);
                }
                Some(f) if o.has("--by-name") => {
                    let norm = |s: &str| s.trim().to_lowercase();
                    map = Some(
                        t.iter()
                            .map(|x| f.iter().position(|y| norm(y) == norm(x)))
                            .collect(),
                    );
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
        let width = first_titles.as_ref().map_or(0, Vec::len);
        for r in head..d.row_count() {
            let row = d.row_values(r).map_err(|e| e.to_string())?;
            match &map {
                Some(m) => {
                    let mut cells = vec![String::new(); width];
                    for (i, v) in row.into_iter().enumerate() {
                        if let Some(Some(j)) = m.get(i) {
                            cells[*j] = v;
                        }
                    }
                    w.row(&cells)?;
                }
                None => w.row(&row)?,
            }
        }
    }
    let n = w.finish()?;
    done(
        &o,
        format!("saved {n} rows to {}", out.path),
        json!({"rows": n}),
    );
    Ok(())
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
            format!("k,n
a/b,1
a_b,2
a/b,3
{long}1,4
{long}2,5
end.,6
end,7
"),
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
            .map(|n| std::fs::read_to_string(out.join(n)).unwrap().lines().count() - 1)
            .sum();
        assert_eq!(rows, 7, "every row is in a file");
        let first = std::fs::read_to_string(out.join("p a_b.csv")).unwrap();
        assert!(first.contains("a/b,1") && first.contains("a/b,3"));
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
