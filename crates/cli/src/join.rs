//! `natendo-cli join`: the app's Join on the command line, with the same
//! options (see `USAGE`). Columns are named by their title (first row) or
//! by number from 1.

use natendo_engine::join::{JoinSource, JoinSpec, JoinWrite, KeepRows, KeyMatch, Place, Repeats};
use natendo_engine::{Document, Progress};

use crate::more::{self, Out};
use crate::USAGE;

/// Most bytes of a `--list` file of keys.
const MAX_LIST_BYTES: u64 = 256 << 20;

/// A column of `d` by title (first row, any case) or number from 1.
fn column(d: &Document, name: &str, header: bool) -> Result<u32, String> {
    let titles = d.expr_names(header);
    let want = name.trim().to_lowercase();
    if header {
        if let Some(c) = titles.iter().position(|t| t.trim().to_lowercase() == want) {
            return Ok(c as u32);
        }
    }
    match name.trim().parse::<u32>() {
        Ok(n) if n >= 1 && n <= d.col_count() => Ok(n - 1),
        _ => Err(format!("no column \"{name}\"")),
    }
}

fn columns(d: &Document, list: &str, header: bool) -> Result<Vec<u32>, String> {
    list.split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| column(d, s, header))
        .collect()
}

#[derive(Default)]
struct Opts<'a> {
    plain: Vec<&'a str>,
    out: Option<&'a str>,
    to: Option<&'a str>,
    keys: Vec<&'a str>,
    columns: Option<&'a str>,
    into: Option<&'a str>,
    write: Option<&'a str>,
    keep: Option<&'a str>,
    repeats: Option<&'a str>,
    separator: Option<&'a str>,
    nearest: Option<&'a str>,
    not_found: Option<&'a str>,
    prefix: Option<&'a str>,
    list: Option<&'a str>,
    loose: bool,
    match_case: bool,
    keep_spaces: bool,
    after_key: bool,
    add_unmatched: bool,
    no_header: bool,
    other_no_header: bool,
    overwrite: bool,
}

fn parse<'a>(args: &[&'a str]) -> Result<Opts<'a>, String> {
    let mut o = Opts::default();
    let mut it = args.iter();
    while let Some(&x) = it.next() {
        let mut value = || {
            it.next()
                .copied()
                .ok_or_else(|| format!("{x} needs a value"))
        };
        match x {
            "-o" | "--out" => o.out = Some(value()?),
            "--to" => o.to = Some(value()?),
            "--key" => o.keys.push(value()?),
            "--columns" => o.columns = Some(value()?),
            "--into" => o.into = Some(value()?),
            "--write" => o.write = Some(value()?),
            "--keep" => o.keep = Some(value()?),
            "--repeats" => o.repeats = Some(value()?),
            "--separator" => o.separator = Some(value()?),
            "--nearest" => o.nearest = Some(value()?),
            "--not-found" => o.not_found = Some(value()?),
            "--prefix" => o.prefix = Some(value()?),
            "--list" => o.list = Some(value()?),
            "--loose" => o.loose = true,
            "--match-case" => o.match_case = true,
            "--keep-spaces" => o.keep_spaces = true,
            "--after-key" => o.after_key = true,
            "--add-unmatched" => o.add_unmatched = true,
            "--no-header" => o.no_header = true,
            "--other-no-header" => o.other_no_header = true,
            "--overwrite" => o.overwrite = true,
            _ if x.starts_with('-') => return Err(format!("unknown option {x}\n{USAGE}")),
            _ => o.plain.push(x),
        }
    }
    Ok(o)
}

fn pick<T: Copy>(v: Option<&str>, what: &str, all: &[(&str, T)]) -> Result<Option<T>, String> {
    let Some(v) = v else { return Ok(None) };
    all.iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(v))
        .map(|x| Some(x.1))
        .ok_or_else(|| {
            let names: Vec<&str> = all.iter().map(|x| x.0).collect();
            format!("{what} is one of: {}", names.join(", "))
        })
}

pub fn join(args: &[&str], json: bool) -> Result<(), String> {
    let o = parse(args)?;
    let out = o.out.ok_or_else(|| USAGE.to_string())?;
    let (input, other_path) = match (o.plain.as_slice(), o.list) {
        ([input], Some(_)) => (*input, None),
        ([input, other], None) => (*input, Some(*other)),
        _ => return Err(USAGE.to_string()),
    };
    if o.keys.is_empty() {
        return Err("name the key column(s) with --key (here=there for different names)".into());
    }
    let header = !o.no_header;
    let other_header = !o.other_no_header;
    let mut d = more::load(input, 0)?;
    let other = match other_path {
        Some(p) => Some(more::load(p, 0)?),
        None => None,
    };
    let mut keys = Vec::new();
    let mut other_keys = Vec::new();
    for (i, k) in o.keys.iter().enumerate() {
        let (mine, theirs) = k.split_once('=').unwrap_or((k, k));
        keys.push(column(&d, mine, header)?);
        other_keys.push(match &other {
            Some(t) => column(t, theirs, other_header)?,
            None => i as u32,
        });
    }
    let write = pick(
        o.write,
        "--write",
        &[
            ("new", JoinWrite::New),
            ("overwrite", JoinWrite::Overwrite),
            ("fill-blanks", JoinWrite::FillBlanks),
            ("none", JoinWrite::Nothing),
        ],
    )?;
    let write = write.unwrap_or(if o.list.is_some() && o.repeats.is_none() {
        JoinWrite::Nothing
    } else {
        JoinWrite::New
    });
    let keep_rows = pick(
        o.keep,
        "--keep",
        &[
            ("all", KeepRows::All),
            ("matched", KeepRows::Matched),
            ("unmatched", KeepRows::Unmatched),
        ],
    )?
    .unwrap_or_default();
    let repeats = pick(
        o.repeats,
        "--repeats",
        &[
            ("first", Repeats::First),
            ("last", Repeats::Last),
            ("every", Repeats::Every),
            ("joined", Repeats::Joined),
            ("count", Repeats::Count),
            ("sum", Repeats::Sum),
            ("min", Repeats::Min),
            ("max", Repeats::Max),
        ],
    )?
    .unwrap_or_default();
    let key_match = pick(
        o.nearest,
        "--nearest",
        &[("below", KeyMatch::Below), ("above", KeyMatch::Above)],
    )?
    .unwrap_or_default();
    let cols = match (&other, o.columns) {
        (Some(t), Some(c)) => columns(t, c, other_header)?,
        _ => Vec::new(),
    };
    let targets = match o.into {
        Some(c) => columns(&d, c, header)?,
        None if matches!(write, JoinWrite::Overwrite | JoinWrite::FillBlanks) => {
            return Err("--write overwrite and fill-blanks need --into (columns here)".into())
        }
        None => Vec::new(),
    };
    let spec = JoinSpec {
        keys,
        other_keys,
        columns: cols,
        keep_first: header,
        other_header,
        match_case: o.match_case,
        trim: !o.keep_spaces,
        add_unmatched: o.add_unmatched,
        write,
        targets,
        keep_rows,
        repeats: if o.list.is_some() && write == JoinWrite::New {
            Repeats::Count
        } else {
            repeats
        },
        separator: o.separator.unwrap_or("; ").to_string(),
        loose: o.loose,
        not_found: o.not_found.unwrap_or("").to_string(),
        place: if o.after_key {
            Place::AfterKey
        } else {
            Place::End
        },
        prefix: o.prefix.unwrap_or("").to_string(),
        key_match,
    };
    let p = Progress::new();
    let src = match (&other, o.list) {
        (Some(t), _) => JoinSource::build(t, &spec, &p),
        (None, Some(path)) => {
            let len = std::fs::metadata(path)
                .map_err(|e| format!("{path}: {e}"))?
                .len();
            if len > MAX_LIST_BYTES {
                return Err(format!("{path}: the list is too long"));
            }
            // Read like any table (UTF-8, UTF-16 or a legacy code page), one
            // key per line, parts split by tabs.
            let opts = natendo_engine::OpenOptions {
                delimiter: Some(b'\t'),
                ..Default::default()
            };
            crate::refuse_encrypted(path)?;
            let keys = Document::open_indexed(std::path::Path::new(path), opts, None)
                .map_err(|e| format!("{path}: {e}"))?;
            let mut text = String::new();
            for r in 0..keys.row_count() {
                let row = keys.row_values(r).map_err(|e| format!("{path}: {e}"))?;
                text.push_str(&row.join("\t"));
                text.push('\n');
            }
            JoinSource::from_list(&text, &spec)
        }
        (None, None) => unreachable!("checked above"),
    }
    .map_err(|e| e.to_string())?;
    drop(other);
    let (plan, rep) = d.plan_join(&src, &spec, &p).map_err(|e| e.to_string())?;
    drop(src);
    d.apply_join(plan).map_err(|e| e.to_string())?;
    let msg = more::write_out(
        &mut d,
        input,
        &Out {
            path: out,
            to: o.to,
            header,
            landscape: false,
            overwrite: o.overwrite,
            encoding: None,
        },
    )?;
    if json {
        more::print_json(&serde_json::json!({ "report": rep, "result": msg }));
    } else {
        println!(
            "{} matched, {} found no match, {} blank keys; {} columns added, {} cells written, {} rows taken out, {} extra rows, {} rows added",
            rep.matched,
            rep.unmatched,
            rep.blank_keys,
            rep.cols,
            rep.changed,
            rep.removed,
            rep.extra_rows,
            rep.added
        );
        println!("{msg}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, body: &str) -> String {
        let dir = std::env::temp_dir().join(format!("nde-cli-join-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn joins_files_and_lists() {
        let a = tmp(
            "a.csv",
            "id,name,city\r\n001,Ann,\r\n2,Bob,Old\r\n3,Cy,\r\n",
        );
        let b = tmp("b.csv", "code,town\r\n1,Rome\r\n2,Oslo\r\n");
        let out = tmp("out.csv", "");
        join(
            &[
                &a,
                &b,
                "-o",
                &out,
                "--key",
                "id=code",
                "--loose",
                "--write",
                "fill-blanks",
                "--columns",
                "town",
                "--into",
                "city",
                "--overwrite",
            ],
            false,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "id,name,city\r\n001,Ann,Rome\r\n2,Bob,Old\r\n3,Cy,\r\n"
        );
        let list = tmp("keys.txt", "2\n3\n");
        join(
            &[
                &a,
                "--list",
                &list,
                "-o",
                &out,
                "--key",
                "1",
                "--keep",
                "unmatched",
                "--loose",
                "--overwrite",
            ],
            false,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "id,name,city\r\n001,Ann,\r\n"
        );
        // A list saved as UTF-16 (with its byte order mark) reads the same.
        let dir = std::path::Path::new(&list).parent().unwrap().to_path_buf();
        let wide = dir.join("keys16.txt");
        let mut bytes = vec![0xFF, 0xFE];
        for u in "2\r\n3\r\n".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        std::fs::write(&wide, bytes).unwrap();
        let wide = wide.to_string_lossy().into_owned();
        join(
            &[
                &a,
                "--list",
                &wide,
                "-o",
                &out,
                "--key",
                "1",
                "--keep",
                "unmatched",
                "--loose",
                "--overwrite",
            ],
            false,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "id,name,city\r\n001,Ann,\r\n"
        );
        // Mistakes are reported, not guessed.
        assert!(join(&[&a, &b, "-o", &out, "--overwrite"], false).is_err());
        assert!(join(&[&a, &b, "-o", &out, "--key", "nope", "--overwrite"], false).is_err());
        assert!(join(
            &[
                &a,
                &b,
                "-o",
                &out,
                "--key",
                "id=code",
                "--write",
                "overwrite",
                "--overwrite"
            ],
            false
        )
        .is_err());
        assert!(join(
            &[&a, &b, "-o", &out, "--key", "id=code", "--repeats", "most"],
            false
        )
        .is_err());
        assert!(join(&[&a, &b, "-o", &out, "--key", "id=code"], false).is_err());
    }
}
