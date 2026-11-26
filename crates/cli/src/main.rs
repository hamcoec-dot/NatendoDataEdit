//! `natendo-cli`: Natendo Data Edit without a window, for scripts and
//! agents. Every command that reads a table can print JSON (`--json`).
//!
//! `--version`, `sniff`, `info`, `view`, `clean` (run a saved cleaner),
//! `convert` (any importable file to any export format or delimited text),
//! `compute` (add a computed column), `join` (bring in values from another
//! table by key, or keep or drop rows on a list), `validate` (run data checks), `app`
//! (talk to the running app's agent link), and the toolkit: `headers`,
//! `select`, `filter`, `sort`, `dedupe`, `stats`, `count`, `sample`,
//! `slice`, `split` and `cat`.

mod app;
mod join;
mod more;
mod tools;

use std::path::Path;
use std::process::ExitCode;

use natendo_engine::{Dialect, Document, OpenOptions};

const USAGE: &str = "usage: natendo-cli --version
       natendo-cli sniff <file> [--json]
       natendo-cli info <file> [--json]
       natendo-cli view <file> [first-row] [row-count] [--json]
       natendo-cli clean <cleaner> <in> -o <out> [--columns 1,3] [--skip-header] [--overwrite]
       natendo-cli convert <in> -o <out> [--to csv|tsv|psv|json|xml|html|md|xlsx|pdf|fixed] [--sheet N] [--no-header] [--landscape] [--overwrite]
       natendo-cli compute <in> -o <out> --expr <expression> [--title <name>] [--no-header] [--overwrite]
       natendo-cli validate <in> --checks <checks.json> [--no-header] [--json]
       natendo-cli run <recipe.json> <in> -o <out> [--set <question>=<answer> ...] [--to <format>] [--force] [--dry-run] [--no-header] [--overwrite]
       natendo-cli headers <in> [--json]
       natendo-cli select <in> -o <out> --columns <a,b,...>
       natendo-cli filter <in> -o <out> --expr <formula>
       natendo-cli sort <in> -o <out> --by <col>[:desc][,<col>...]
       natendo-cli dedupe <in> -o <out> [--columns <a,b>] [--match-case]
       natendo-cli stats <in> [--columns <a,b>] [--json]
       natendo-cli count <in> --columns <a[,b]> [-o <out>] [--json]
       natendo-cli gaps <in> --column <c> [--series <c> ...] [--digits N] [--shape INV-#####]
           [--each-shape] [--from X] [--to Y] [--fail-on-gap] [--no-header] [--sheet N] [--json]
       natendo-cli sets <a.csv:ID> <b.csv:ID> [... up to 5; keys a.csv:ID+Ward or #1+#4] [--match-case]
           [--no-trim] [--loose] [--slice a,!b [--rows-of a] [--keys-only] -o <out>] [--json]
       natendo-cli sample <in> -o <out> (--rows N | --percent P | --every N) [--seed S]
       natendo-cli slice <in> -o <out> --rows <first>-<last>
       natendo-cli split <in> -o <folder> (--rows N | --by <col>) [--to csv|tsv]
       natendo-cli cat <in> <in> ... -o <out> [--by-name]
           (the toolkit also takes --no-header, --sheet N, --to <format>, --overwrite and --json;
            columns go by title or by number from 1)
       natendo-cli pivot <layout.ndpivot> <in> -o <out> [--to <format>] [--no-header] [--overwrite]
       natendo-cli join <in> <other> -o <out> --key <col>[=<other col>] [--key ...] [--columns a,b]
           [--write new|overwrite|fill-blanks|none] [--into x,y] [--keep all|matched|unmatched]
           [--repeats first|last|every|joined|count|sum|min|max] [--separator \"; \"] [--loose]
           [--match-case] [--keep-spaces] [--nearest below|above] [--not-found <text>] [--after-key]
           [--prefix <text>] [--add-unmatched] [--no-header] [--other-no-header] [--to <format>] [--overwrite] [--json]
       natendo-cli join <in> --list <keys.txt> -o <out> --key <col> [--keep matched|unmatched] [--write new] ...
       natendo-cli app tabs | info <tab> | read <tab> [first] [count] | open <file> | goto <tab> <row> <col>

The command line gives no private data warnings: check what you write before sharing it.
`app read` is refused for tabs in private file mode.";

/// Problems found (validate), as opposed to a failure (2).
const PROBLEMS: u8 = 1;

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let json = match args.iter().position(|a| a == "--json") {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    };
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match argv.as_slice() {
        ["--version"] | ["-V"] => {
            println!("natendo-cli {}", natendo_engine::version());
            Ok(())
        }
        ["sniff", path] if json => open(path).map(|d| more::print_json(&more::sniff_json(&d))),
        ["sniff", path] => open(path).map(|d| sniff(&d)),
        ["info", path] if json => open(path).map(|d| more::print_json(&more::info_json(&d))),
        ["info", path] => open(path).map(|d| info(&d)),
        ["view", path, rest @ ..] if rest.len() <= 2 => {
            let num =
                |i: usize, default: u64| rest.get(i).map_or(Ok(default), |s| s.parse::<u64>());
            match (num(0, 0), num(1, 20)) {
                (Ok(first), Ok(count)) if json => {
                    open(path).and_then(|d| more::view_json(&d, first, count))
                }
                (Ok(first), Ok(count)) => open(path).and_then(|d| view(&d, first, count)),
                _ => Err(USAGE.to_string()),
            }
        }
        ["clean", rest @ ..] => clean(rest, json),
        ["convert", rest @ ..] => more::convert(rest, json),
        ["compute", rest @ ..] => more::compute(rest, json),
        ["run", rest @ ..] => more::run(rest, json),
        ["pivot", rest @ ..] => more::pivot(rest, json),
        ["join", rest @ ..] => join::join(rest, json),
        ["validate", rest @ ..] => match more::validate(rest, json) {
            Ok(true) => Ok(()),
            Ok(false) => return ExitCode::from(PROBLEMS),
            Err(e) => Err(e),
        },
        ["app", rest @ ..] => app::run(rest),
        ["headers", rest @ ..] => tools::headers(&with_json(rest, json)),
        ["select", rest @ ..] => tools::select(&with_json(rest, json)),
        ["filter", rest @ ..] => tools::filter(&with_json(rest, json)),
        ["sort", rest @ ..] => tools::sort(&with_json(rest, json)),
        ["dedupe", rest @ ..] => tools::dedupe(&with_json(rest, json)),
        ["stats", rest @ ..] => tools::stats(&with_json(rest, json)),
        ["count", rest @ ..] => tools::count(&with_json(rest, json)),
        ["gaps", rest @ ..] => match tools::gaps(rest, json) {
            Ok(true) => Ok(()),
            Ok(false) => return ExitCode::from(PROBLEMS),
            Err(e) => Err(e),
        },
        ["sets", rest @ ..] => tools::sets(&with_json(rest, json)),
        ["sample", rest @ ..] => tools::sample(&with_json(rest, json)),
        ["slice", rest @ ..] => tools::slice(&with_json(rest, json)),
        ["split", rest @ ..] => tools::split(&with_json(rest, json)),
        ["cat", rest @ ..] => tools::cat(&with_json(rest, json)),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if json {
                more::print_json(&serde_json::json!({ "error": e }));
            } else {
                eprintln!("{e}");
            }
            ExitCode::from(2)
        }
    }
}

/// The toolkit commands read `--json` themselves: give it back to them.
fn with_json<'a>(rest: &[&'a str], json: bool) -> Vec<&'a str> {
    let mut v = rest.to_vec();
    if json {
        v.push("--json");
    }
    v
}

fn open(path: &str) -> Result<Document, String> {
    // A dBase table is read through the importer.
    if Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("dbf"))
    {
        return more::load(path, 0);
    }
    Document::open_indexed(Path::new(path), OpenOptions::default(), None)
        .map_err(|e| format!("{path}: {e}"))
}

/// The separator in words: `tab`, `space`, `spaces (lined up)`, `0x1F`, `,`.
fn delimiter_name(d: &Dialect) -> String {
    let name = match d.delimiter() {
        b"\t" => "tab".into(),
        b" " => "space".into(),
        s if s.len() == 1 && s[0] < 0x20 => format!("0x{:02X}", s[0]),
        _ => d.delimiter_str().to_string(),
    };
    if d.merges() {
        format!("{name}s (lined up)")
    } else {
        name
    }
}

fn quote_name(d: &Dialect) -> String {
    let q = d.quote().map_or("none".into(), |q| (q as char).to_string());
    match d.escape() {
        Some(e) => format!("{q} (escape {})", e as char),
        None => q,
    }
}

/// The separator, then what detection weighed.
fn sniff(d: &Document) {
    println!("{}", delimiter_name(&d.format().dialect));
    if let Some(found) = d.detection() {
        println!("confidence: {:.2}", found.confidence);
        for (i, c) in found.choices.iter().enumerate() {
            println!(
                "  {}. separator {:<18} quote {:<14} columns {:<4} lines above {:<3} score {:.3}",
                i + 1,
                delimiter_name(&c.dialect),
                quote_name(&c.dialect),
                c.columns,
                c.skip_lines,
                c.score
            );
        }
    }
}

fn info(d: &Document) {
    let f = d.format();
    let s = d.index().stats();
    println!(
        "encoding:    {}{}",
        f.encoding,
        if f.bom { " (with BOM)" } else { "" }
    );
    println!("delimiter:   {}", delimiter_name(&f.dialect));
    println!("quote:       {}", quote_name(&f.dialect));
    if d.preamble_lines() > 0 {
        println!("above table: {} lines kept as they are", d.preamble_lines());
    }
    println!(
        "line breaks: {:?}{}",
        f.line_break,
        if s.mixed_line_breaks() {
            " (mixed)"
        } else {
            ""
        }
    );
    println!("rows:        {}", d.row_count());
    println!(
        "columns:     {} (usual {})",
        d.col_count(),
        s.usual_fields()
    );
    println!("repaired:    {} rows", s.repair_count);
    if let Some(r) = d.index().line_bound_from() {
        println!(
            "note:        unterminated quote; rows from {} read line by line",
            r + 1
        );
    }
    if d.is_lossy() {
        println!(
            "note:        some bytes were not valid {} and were replaced",
            f.encoding
        );
    }
}

fn view(d: &Document, first: u64, count: u64) -> Result<(), String> {
    let v = d
        .viewport(
            first..first.saturating_add(count),
            0..d.col_count().min(50),
            60,
        )
        .map_err(|e| e.to_string())?;
    for r in 0..v.rows {
        let cells: Vec<&str> = (0..v.cols).map(|c| v.cell(r, c).unwrap_or("")).collect();
        println!(
            "{}\t{}",
            v.first_row + r as u64 + 1,
            cells.join(" | ").escape_debug()
        );
    }
    Ok(())
}

/// Run a saved cleaner on a file and save the result with the same layout.
fn clean(args: &[&str], json: bool) -> Result<(), String> {
    use natendo_engine::clean::{CleanSpec, Cleaner, MAX_CLEANER_BYTES};
    use natendo_engine::find::Scope;
    use natendo_engine::{Progress, SaveOptions};
    let mut plain: Vec<&str> = Vec::new();
    let (mut out, mut cols, mut skip, mut overwrite) = (None, None, false, false);
    let mut it = args.iter();
    while let Some(&a) = it.next() {
        match a {
            "-o" | "--out" => out = it.next().copied(),
            "--columns" => {
                let list = it.next().ok_or(USAGE)?;
                let parsed: Option<Vec<u32>> = list
                    .split(',')
                    .map(|c| c.trim().parse::<u32>().ok().and_then(|n| n.checked_sub(1)))
                    .collect();
                cols = Some(parsed.ok_or("--columns takes column numbers from 1, like 1,3")?);
            }
            "--skip-header" => skip = true,
            "--overwrite" => overwrite = true,
            a if a.starts_with('-') => {
                return Err(format!(
                    "unknown option {a}
{USAGE}"
                ))
            }
            a => plain.push(a),
        }
    }
    let ([cleaner_path, input], Some(output)) = (plain.as_slice(), out) else {
        return Err(USAGE.to_string());
    };
    let size = std::fs::metadata(cleaner_path)
        .map_err(|e| format!("{cleaner_path}: {e}"))?
        .len();
    if size > MAX_CLEANER_BYTES as u64 {
        return Err(format!(
            "{cleaner_path}: a cleaner file can be at most 1 MB"
        ));
    }
    let bytes = std::fs::read(cleaner_path).map_err(|e| format!("{cleaner_path}: {e}"))?;
    let cleaner = Cleaner::from_json(&bytes).map_err(|e| format!("{cleaner_path}: {e}"))?;
    let out_path = Path::new(output);
    let same = match (
        std::fs::canonicalize(input),
        std::fs::canonicalize(out_path),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if (same || out_path.exists()) && !overwrite {
        return Err(format!(
            "{output} already exists; add --overwrite to replace it"
        ));
    }
    let mut doc = open(input)?;
    let spec = CleanSpec {
        cleaner,
        scope: match cols {
            Some(cols) => Scope::Columns { cols },
            None => Scope::All,
        },
        skip_first: skip,
        read: Default::default(),
    };
    let now = natendo_engine::clean::now_utc();
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    let plan = doc
        .plan_clean(&spec, now, seed, 0, &Progress::new())
        .map_err(|e| e.to_string())?;
    let (changed, left) = (plan.changed, plan.left);
    doc.apply_clean(plan).map_err(|e| e.to_string())?;
    let report = doc
        .save(SaveOptions {
            path: if same {
                None
            } else {
                Some(out_path.to_path_buf())
            },
            rewrite: true,
            overwrite_changed: false,
            ..Default::default()
        })
        .map_err(|e| format!("{output}: {e}"))?;
    let msg = format!(
        "{changed} cells cleaned{}; saved {} rows to {}",
        if left > 0 {
            format!(", {left} left as they were")
        } else {
            String::new()
        },
        report.rows,
        report.path.display()
    );
    if json {
        more::print_json(&serde_json::json!({
            "message": msg,
            "changed": changed,
            "left": left,
            "rows": report.rows,
            "file": report.path.display().to_string(),
        }));
    } else {
        println!("{msg}");
    }
    Ok(())
}
