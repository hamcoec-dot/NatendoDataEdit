//! Log files: each line is cut into columns by a pattern (a regular
//! expression with groups). Lines the pattern does not fit, such as the
//! rest of a stack trace, can join the row above, become rows of their
//! own, or be left out. Patterns are compiled under find's size limits.

use serde::{Deserialize, Serialize};

use crate::fixed::lines;
use crate::table::Sink;
use crate::Error;

/// Most groups (columns) in a pattern.
pub const MAX_GROUPS: usize = 1_000;
/// Lines looked at when choosing a preset.
const GUESS_LINES: usize = 200;

/// Ready-made patterns: name and pattern.
pub const PRESETS: &[(&str, &str)] = &[
    (
        "Web server access log",
        r#"^(?P<host>\S+) (?P<ident>\S+) (?P<user>\S+) \[(?P<time>[^\]]+)\] "(?P<request>[^"]*)" (?P<status>\d{3}) (?P<size>\S+)(?: "(?P<referrer>[^"]*)" "(?P<agent>[^"]*)")?"#,
    ),
    (
        "System log (syslog)",
        r"^(?P<time>[A-Z][a-z]{2} [ \d]\d \d\d:\d\d:\d\d) (?P<host>\S+) (?P<process>[^:\[\s]+)(?:\[(?P<pid>\d+)\])?: (?P<message>.*)$",
    ),
    (
        "Date and time, level, message",
        r"^(?P<time>\d{4}-\d\d-\d\d[T ]\d\d:\d\d:\d\d(?:[.,]\d+)?(?:Z|[+-]\d\d:?\d\d)?)\s+\[?(?P<level>TRACE|DEBUG|INFO|NOTICE|WARN|WARNING|ERROR|FATAL|CRITICAL|trace|debug|info|warn|warning|error|fatal|critical)\]?:?\s+(?P<message>.*)$",
    ),
    (
        "Date and time, message",
        r"^(?P<time>\d{4}-\d\d-\d\d[T ]\d\d:\d\d:\d\d\S*)\s+(?P<message>.*)$",
    ),
    ("Whole line", r"^(?P<line>.*)$"),
];

/// What to do with a line the pattern does not fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unmatched {
    /// Add it to the last column of the row above, on a new line.
    #[default]
    Join,
    /// Put it in a row of its own, in the first column.
    Row,
    Skip,
}

/// Compile a pattern and name its columns.
pub fn compile(pattern: &str) -> Result<(regex::Regex, Vec<String>), Error> {
    if pattern.trim().is_empty() {
        return Err(Error::Invalid("write a pattern for the lines".into()));
    }
    if pattern.chars().count() > crate::find::MAX_PATTERN_CHARS {
        return Err(Error::Invalid("the pattern is too long".into()));
    }
    let re = crate::find::build(pattern, true)?;
    let n = re.captures_len() - 1;
    if n == 0 {
        return Err(Error::Invalid(
            "the pattern needs groups in brackets, one for each column, like (\\S+) or (?P<name>\\S+)".into(),
        ));
    }
    if n > MAX_GROUPS {
        return Err(Error::Invalid(format!("at most {MAX_GROUPS} groups")));
    }
    let names = re
        .capture_names()
        .skip(1)
        .enumerate()
        .map(|(i, n)| n.map_or_else(|| format!("Column {}", i + 1), str::to_string))
        .collect();
    Ok((re, names))
}

/// The preset that fits the most of the first lines (the whole line when
/// none fits half of them). Returns its number.
pub fn guess(text: &str) -> usize {
    let sample: Vec<&str> = lines(text)
        .filter(|l| !l.trim().is_empty())
        .take(GUESS_LINES)
        .collect();
    let last = PRESETS.len() - 1;
    if sample.is_empty() {
        return last;
    }
    let mut best = (last, 0usize);
    for (i, (_, p)) in PRESETS[..last].iter().enumerate() {
        let re = crate::find::build(p, true).expect("presets compile");
        let n = sample.iter().filter(|l| re.is_match(l)).count();
        if n > best.1 {
            best = (i, n);
        }
    }
    if best.1 * 2 >= sample.len() {
        best.0
    } else {
        last
    }
}

/// Read log text into `sink`: a title row, then one row per matching line.
pub(crate) fn read(
    text: &str,
    pattern: &str,
    unmatched: Unmatched,
    sink: &mut Sink,
) -> Result<(), Error> {
    let (re, names) = compile(pattern)?;
    sink.row(&names)?;
    let mut pending: Option<Vec<String>> = None;
    let mut locs = re.capture_locations();
    for line in lines(text) {
        if sink.full() {
            pending = None;
            break;
        }
        if re.captures_read(&mut locs, line).is_some() {
            if let Some(p) = pending.take() {
                flush(p, sink)?;
            }
            let row = (1..=names.len())
                .map(|g| {
                    locs.get(g)
                        .map_or(String::new(), |(a, b)| line[a..b].to_string())
                })
                .collect();
            pending = Some(row);
            continue;
        }
        if line.is_empty() && unmatched != Unmatched::Join {
            continue;
        }
        match unmatched {
            Unmatched::Skip => {}
            Unmatched::Row => {
                if let Some(p) = pending.take() {
                    flush(p, sink)?;
                }
                sink.row(&[line])?;
            }
            Unmatched::Join => match pending.as_mut().and_then(|p| p.last_mut()) {
                Some(cell) if cell.len() + line.len() < crate::table::MAX_CELL => {
                    cell.push('\n');
                    cell.push_str(line);
                }
                _ => {
                    if let Some(p) = pending.take() {
                        flush(p, sink)?;
                    }
                    if !line.is_empty() {
                        sink.row(&[line])?;
                    }
                }
            },
        }
    }
    if let Some(p) = pending {
        flush(p, sink)?;
    }
    Ok(())
}

/// Write a row, without the blank lines a join may have left at its end.
fn flush(mut p: Vec<String>, sink: &mut Sink) -> Result<(), Error> {
    if let Some(last) = p.last_mut() {
        let t = last.trim_end_matches('\n').len();
        last.truncate(t);
    }
    sink.row(&p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str, pattern: &str, u: Unmatched) -> Vec<Vec<String>> {
        let mut sink = Sink::new(None);
        read(text, pattern, u, &mut sink).unwrap();
        let d = sink.finish().into_document().unwrap();
        (0..d.row_count())
            .map(|r| d.row_values(r).unwrap())
            .collect()
    }

    const APP: &str = "2024-05-01 10:00:01 INFO started\n\
                       2024-05-01 10:00:02 ERROR failed: boom\n\
                       \tat a.b(c.java:1)\n\
                       \tat d.e(f.java:2)\n\
                       2024-05-01 10:00:03 WARN slow\n";

    #[test]
    fn presets_are_guessed() {
        assert_eq!(PRESETS[guess(APP)].0, "Date and time, level, message");
        let web = "127.0.0.1 - frank [10/Oct/2000:13:55:36 -0700] \"GET /a.gif HTTP/1.0\" 200 2326 \"http://x/\" \"Mozilla/4.08\"\n";
        assert_eq!(PRESETS[guess(web)].0, "Web server access log");
        let sys = "Jun  3 12:01:02 host sshd[42]: Accepted key\nJun  3 12:01:03 host cron: ran\n";
        assert_eq!(PRESETS[guess(sys)].0, "System log (syslog)");
        assert_eq!(PRESETS[guess("hello\nworld\n")].0, "Whole line");
        assert_eq!(PRESETS[guess("")].0, "Whole line");
    }

    #[test]
    fn continuation_lines_join_the_row_above() {
        let p = PRESETS[2].1;
        let t = table(APP, p, Unmatched::Join);
        assert_eq!(t[0], ["time", "level", "message"]);
        assert_eq!(t.len(), 4);
        assert_eq!(
            t[2][2],
            "failed: boom\n\tat a.b(c.java:1)\n\tat d.e(f.java:2)"
        );
        let t = table(APP, p, Unmatched::Row);
        assert_eq!(t.len(), 6);
        assert_eq!(t[3][0], "\tat a.b(c.java:1)");
        let t = table(APP, p, Unmatched::Skip);
        assert_eq!(t.len(), 4);
        assert_eq!(t[2][2], "failed: boom");
    }

    #[test]
    fn unnamed_groups_and_bad_patterns() {
        let t = table("a=1\nb=2\n", r"(\w+)=(\d+)", Unmatched::Skip);
        assert_eq!(t[0], ["Column 1", "Column 2"]);
        assert_eq!(t[2], ["b", "2"]);
        assert!(compile("no groups").is_err());
        assert!(compile("(").is_err());
        assert!(compile("").is_err());
        let wide = "(a)".repeat(MAX_GROUPS + 1);
        assert!(compile(&wide).is_err());
        for (_, p) in PRESETS {
            compile(p).unwrap();
        }
    }
}
