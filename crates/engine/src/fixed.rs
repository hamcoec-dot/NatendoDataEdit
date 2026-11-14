//! Fixed-width text: columns sit at character positions, padded with
//! spaces. Column starts are guessed from the positions where every line
//! has a space, and can be set by hand. Tabs count as moving to the next
//! multiple of 8.

use std::borrow::Cow;

use crate::table::Sink;
use crate::Error;

/// Lines looked at when guessing.
pub const GUESS_LINES: usize = 2_000;
/// Widest line position considered when guessing, in characters.
pub const GUESS_WIDTH: usize = 10_000;
/// Lines shown on the ruler.
pub const RULER_LINES: usize = 40;
/// Characters of each ruler line.
pub const RULER_CHARS: usize = 400;

/// The line with tabs turned into spaces.
pub fn expand_tabs(line: &str) -> Cow<'_, str> {
    if !line.contains('\t') {
        return Cow::Borrowed(line);
    }
    let mut out = String::with_capacity(line.len() + 16);
    let mut n = 0usize;
    for c in line.chars() {
        if c == '\t' {
            let to = (n / 8 + 1) * 8;
            out.extend(std::iter::repeat_n(' ', to - n));
            n = to;
        } else {
            out.push(c);
            n += 1;
        }
    }
    Cow::Owned(out)
}

/// Lines of the text without their line breaks (`\n`, `\r\n` or `\r`).
pub fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l))
}

/// Guess where columns start: a column starts where a run of positions
/// that are spaces in every line ends. The first column starts at 0.
pub fn guess_starts<'a>(sample: impl Iterator<Item = &'a str>) -> Vec<u32> {
    let mut seen = false;
    let mut width = 0usize;
    let mut gap: Vec<bool> = Vec::new();
    for line in sample.filter(|l| !l.trim().is_empty()).take(GUESS_LINES) {
        let line = expand_tabs(line);
        let mut n = 0usize;
        for (i, c) in line.chars().take(GUESS_WIDTH).enumerate() {
            if i >= gap.len() {
                // Positions this line is the first to reach: every earlier
                // line was shorter, so they count as gaps for those.
                gap.push(true);
            }
            if c != ' ' {
                gap[i] = false;
            }
            n = i + 1;
        }
        width = width.max(n);
        seen = true;
    }
    let mut starts = vec![0u32];
    if !seen {
        return starts;
    }
    for i in 1..width {
        if !gap[i] && gap[i - 1] {
            starts.push(i as u32);
        }
    }
    starts.truncate(crate::import::MAX_IMPORT_COLS);
    starts
}

/// Check column starts: increasing, the first one 0 (added when missing).
pub fn normalize(starts: &[u32]) -> Result<Vec<u32>, Error> {
    let mut s: Vec<u32> = starts.to_vec();
    s.sort_unstable();
    s.dedup();
    if s.first() != Some(&0) {
        s.insert(0, 0);
    }
    if s.len() > crate::import::MAX_IMPORT_COLS {
        return Err(Error::Invalid(format!(
            "at most {} columns",
            crate::import::MAX_IMPORT_COLS
        )));
    }
    Ok(s)
}

/// Cut one line at the column starts.
pub fn split_line(line: &str, starts: &[u32], keep_spaces: bool, out: &mut Vec<String>) {
    out.clear();
    let line = expand_tabs(line);
    // Byte offset of each start (or the end of the line).
    let offs: Vec<usize> = line.char_indices().map(|(i, _)| i).collect();
    let at = |c: u32| offs.get(c as usize).copied().unwrap_or(line.len());
    let mut bytes: Vec<usize> = starts.iter().map(|&s| at(s)).collect();
    bytes.push(line.len());
    for w in bytes.windows(2) {
        let (a, b) = (w[0].min(line.len()), w[1].min(line.len()));
        let cell = &line[a..b.max(a)];
        out.push(if keep_spaces {
            cell.to_string()
        } else {
            cell.trim().to_string()
        });
    }
}

/// Read fixed-width text into `sink` (trailing blank lines dropped by it).
pub(crate) fn read(
    text: &str,
    starts: &[u32],
    keep_spaces: bool,
    sink: &mut Sink,
) -> Result<(), Error> {
    let starts = if starts.is_empty() {
        guess_starts(lines(text))
    } else {
        normalize(starts)?
    };
    let mut row = Vec::new();
    for line in lines(text) {
        if sink.full() {
            break;
        }
        if line.is_empty() {
            sink.empty_rows(1);
            continue;
        }
        split_line(line, &starts, keep_spaces, &mut row);
        sink.row(&row)?;
    }
    Ok(())
}

/// The first lines for the ruler (tabs expanded, cut to 400 characters).
pub fn ruler_lines(text: &str) -> Vec<String> {
    lines(text)
        .take(RULER_LINES)
        .map(|l| expand_tabs(l).chars().take(RULER_CHARS).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(line: &str, starts: &[u32]) -> Vec<String> {
        let mut out = Vec::new();
        split_line(line, starts, false, &mut out);
        out
    }

    #[test]
    fn guesses_columns_with_blank_cells() {
        let text = "Name       City        Amount\n\
                    Ada        London         120\n\
                    Grace                    1500\n\
                    Linus      Helsinki          \n\
                    Ken        New York        33\n";
        let starts = guess_starts(lines(text));
        assert_eq!(starts, vec![0, 11, 23]);
        assert_eq!(
            split("Grace                    1500", &starts),
            ["Grace", "", "1500"]
        );
        assert_eq!(
            split("Ken        New York        33", &starts),
            ["Ken", "New York", "33"]
        );
        assert_eq!(
            split("Linus      Helsinki", &starts),
            ["Linus", "Helsinki", ""]
        );
        assert_eq!(split("", &starts), ["", "", ""]);
    }

    #[test]
    fn splits_by_characters_and_tabs() {
        assert_eq!(split("héllo wörld", &[0, 6]), ["héllo", "wörld"]);
        assert_eq!(split("日本語abc", &[0, 3]), ["日本語", "abc"]);
        assert_eq!(split("a\tb", &[0, 8]), ["a", "b"]);
        assert_eq!(split("abc", &[0, 1, 2, 3, 9]), ["a", "b", "c", "", ""]);
        let mut out = Vec::new();
        split_line(" a  b ", &[0, 3], true, &mut out);
        assert_eq!(out, [" a ", " b "]);
        assert_eq!(normalize(&[5, 2, 5]).unwrap(), vec![0, 2, 5]);
    }

    #[test]
    fn reads_into_a_table() {
        let mut sink = Sink::new(None);
        read("id  name\n1   x\n22  yy\n\n", &[], false, &mut sink).unwrap();
        let t = sink.finish();
        let d = t.into_document().unwrap();
        assert_eq!(d.row_count(), 3);
        assert_eq!(d.row_values(2).unwrap(), ["22", "yy"]);
    }

    #[test]
    fn export_then_import_gives_the_same_table() {
        use crate::export::{ExportFormat, ExportOptions};
        let d = crate::Document::from_bytes(
            "name,city,n\r\nAda,London,1\r\nGrace,,22\r\nLinus,\"New\nYork\",\r\n"
                .as_bytes()
                .to_vec(),
            crate::OpenOptions::default(),
            Some("csv"),
        )
        .unwrap();
        let mut out = Vec::new();
        let o = ExportOptions {
            format: ExportFormat::FixedWidth,
            ..Default::default()
        };
        d.export(&mut out, &o, None, &crate::Progress::new())
            .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().next().unwrap(), "name  city     n");
        let mut sink = Sink::new(None);
        read(&text, &[], false, &mut sink).unwrap();
        let back = sink.finish().into_document().unwrap();
        assert_eq!(back.row_values(2).unwrap(), ["Grace", "", "22"]);
        assert_eq!(back.row_values(3).unwrap()[..2], ["Linus", "New York"]);
    }

    #[test]
    fn odd_input_never_panics() {
        for text in [
            "",
            "\n\n",
            "   ",
            "a",
            "\t\t\tx",
            "é\u{0}\r\r\n z",
            &"x ".repeat(20_000),
        ] {
            let s = guess_starts(lines(text));
            let mut out = Vec::new();
            for l in lines(text) {
                split_line(l, &s, false, &mut out);
                split_line(l, &[0, 1, 1000, 3], false, &mut out);
            }
        }
    }
}
