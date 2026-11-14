//! Guessing how a file is laid out: the separator, the quote and escape
//! style, and any lines above the table (a title, comments, a `sep=` line).
//!
//! How it decides:
//!
//! 1. **Samples.** The first 64 KB, plus 16 KB from the middle and from the
//!    end of larger files, each cut to whole lines.
//! 2. **Candidates.** The usual separators; every punctuation or control
//!    byte found on most lines; runs of 2 to 4 punctuation bytes found on
//!    most lines (`||`, `~|~`); spaces, single or in lined-up runs. Each is
//!    tried with the double quote and, where the sample suggests them, the
//!    single quote, no quoting, and backslash escapes.
//! 3. **Two passes.** A quick pass keeps the candidates whose rows agree on
//!    a column count. A full pass then finds lines above the table and
//!    judges the cells: numbers (also with decimal commas), dates and times,
//!    yes/no values, empties, cells that still hold another likely
//!    separator, stray or broken quotes, and whether each column keeps one
//!    kind of value. The middle and end samples must agree as well.
//! 4. **Choice.** Splits that come out identical count once. The best wins;
//!    how far ahead of the runner-up it is sets the confidence, and the
//!    runners-up are kept so the user can pick another.
//!
//! The work is bounded whatever the input: at most about 100 KB of samples,
//! a fixed number of candidates, and a fixed number of lines, rows and cells
//! per candidate, each read with the linear scanner.

use std::hash::{DefaultHasher, Hash, Hasher};

use crate::dialect::{self, Dialect, MAX_DELIM};
use crate::scan::{self, Mode};

/// Bytes read from the start of the file.
pub const HEAD_BYTES: usize = 64 * 1024;
/// Bytes read from the middle and from the end of a larger file.
pub const PART_BYTES: usize = 16 * 1024;
/// Most lines above a table that can be set aside.
pub const MAX_PREAMBLE: usize = 20;
/// Below this confidence the app offers the other choices.
pub const LOW_CONFIDENCE: f32 = 0.6;

/// Lines of the head looked at for candidates.
const MAX_LINES: usize = 1000;
/// Lines counted per byte when looking for candidates.
const COUNT_LINES: usize = 400;
/// Rows split per candidate.
const MAX_ROWS: usize = 400;
/// Rows and columns whose cells are judged.
const JUDGED_ROWS: usize = 150;
const JUDGED_COLS: usize = 48;
/// Candidates kept after the quick pass.
const FULL_PASS: usize = 24;
/// Choices reported.
const MAX_CHOICES: usize = 5;
/// Score of "one column" (no separator): a split has to beat it.
const SINGLE_COLUMN: f32 = 0.3;

/// One way to read the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub dialect: Dialect,
    /// Lines above the table, kept unchanged and not shown as rows.
    pub skip_lines: u32,
    /// Most common number of columns.
    pub columns: u32,
    /// Higher is better; only comparable within one detection.
    pub score: f32,
}

/// The result of [`detect`].
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    /// Best first; never empty.
    pub choices: Vec<Choice>,
    /// 0 to 1: how clearly the first choice beat the others.
    pub confidence: f32,
    /// The file named its separator on a `sep=` line.
    pub sep_line: bool,
}

impl Detection {
    pub fn best(&self) -> &Choice {
        &self.choices[0]
    }

    /// True when the app need not offer the other choices.
    pub fn is_confident(&self) -> bool {
        self.confidence >= LOW_CONFIDENCE
    }
}

/// What the caller already knows.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hints<'a> {
    /// File extension; nudges ties towards its usual separator.
    pub ext: Option<&'a str>,
    /// `Some(None)` means "no quoting"; `None` lets detection decide.
    pub quote: Option<Option<u8>>,
}

/// Guess the layout of `data` (working bytes, after any byte-order mark).
pub fn detect(data: &[u8], hints: Hints) -> Detection {
    let head = head_of(data);
    let parts = parts_of(data);
    let lines = lines_of(head, MAX_LINES);
    let ext_delim = hints.ext.and_then(dialect::delimiter_for_extension);
    let fallback = || Choice {
        dialect: Dialect::new(ext_delim.unwrap_or(b','), hints.quote.unwrap_or(Some(b'"')))
            .unwrap_or(Dialect::CSV),
        skip_lines: 0,
        columns: 1,
        score: SINGLE_COLUMN,
    };

    // A `sep=X` first line names the separator.
    let sep = lines.first().and_then(|&(s, e, _)| sep_line(&head[s..e]));
    let pre = usize::from(sep.is_some());

    let counts = ByteCounts::of(head, &lines[pre.min(lines.len())..]);
    let others = counts.strong_separators();
    let quotes = quote_options(head, hints.quote);

    let mut seps: Vec<(Sep, f32)> = match &sep {
        Some(s) => vec![(*s, 1.0)],
        None => candidates(head, &lines, &counts),
    };
    if let Some(e) = ext_delim {
        let s = Sep::one(e);
        match seps.iter_mut().find(|(x, _)| *x == s) {
            Some((_, p)) => *p *= 1.05,
            None if sep.is_none() => seps.push((s, 1.05)),
            None => {}
        }
    }

    // Quick pass: column-count agreement only.
    let mut quick: Vec<(Dialect, f32, f32)> = Vec::new();
    for &(s, sep_prior) in &seps {
        for &(q, e, q_prior) in &quotes {
            let Ok(d) = Dialect::custom(s.bytes(), q, e, s.merge) else {
                continue;
            };
            let start = lines.get(pre).map_or(head.len(), |l| l.0);
            let (m, c) = agreement(head, start, d);
            if m >= 2 {
                let prior = sep_prior * q_prior;
                quick.push((d, c * c * prior, prior));
            }
        }
    }
    quick.sort_by(|a, b| b.1.total_cmp(&a.1));
    quick.truncate(FULL_PASS);

    // Full pass.
    let mut evals: Vec<Eval> = quick
        .iter()
        .map(|&(d, _, prior)| evaluate(head, &lines, &parts, d, pre, &others, prior))
        .filter(|e| e.columns >= 2)
        .collect();
    evals.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut seen = Vec::new();
    evals.retain(|e| {
        let fresh = !seen.contains(&e.sig);
        seen.push(e.sig);
        fresh
    });

    let mut choices: Vec<Choice> = evals
        .iter()
        .map(|e| Choice {
            dialect: e.dialect,
            skip_lines: e.skip as u32,
            columns: e.columns,
            score: e.score,
        })
        .collect();
    let mut single = Choice {
        skip_lines: pre as u32,
        ..fallback()
    };
    if let Some(s) = &sep {
        let q = hints.quote.unwrap_or(Some(b'"'));
        if let Ok(d) = Dialect::custom(s.bytes(), q, None, false) {
            single.dialect = d;
        }
    }
    // One column is the answer when no split does better.
    let at = choices
        .iter()
        .position(|c| c.score < SINGLE_COLUMN)
        .unwrap_or(choices.len());
    if sep.is_none() || choices.is_empty() {
        choices.insert(at, single);
    }
    choices.truncate(MAX_CHOICES);

    let rows = lines.iter().filter(|&&(s, e, _)| e > s).count();
    let confidence = if sep.is_some() {
        1.0
    } else {
        let best = &choices[0];
        let ratio = choices
            .get(1)
            .map_or(0.0, |c| c.score / best.score.max(1e-6));
        let winner = evals.iter().find(|e| {
            e.dialect == best.dialect && e.skip as u32 == best.skip_lines && e.score == best.score
        });
        let mut conf = if best.columns <= 1 {
            1.0 - ratio * ratio
        } else {
            let quality = winner.map_or(0.0, |e| e.quality);
            (0.5 + 0.5 * quality.min(1.0)) * (1.0 - ratio.powi(4))
        };
        // Too few rows to be sure of anything, or a doubtful winner.
        if rows < 3 || winner.is_some_and(|e| e.doubt) {
            conf = conf.min(LOW_CONFIDENCE - 0.05);
        }
        conf.clamp(0.0, 1.0)
    };
    Detection {
        choices,
        confidence,
        sep_line: sep.is_some(),
    }
}

/// Bytes taken up by the first `n` lines of `data` (each with its line
/// break; fewer if the data ends first).
pub fn skip_lines_len(data: &[u8], n: u32) -> usize {
    let mut pos = 0;
    for _ in 0..n {
        match memchr::memchr2(b'\r', b'\n', &data[pos..]) {
            Some(i) => {
                pos += i;
                pos += if data[pos] == b'\r' && data.get(pos + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
            }
            None => return data.len(),
        }
    }
    pos
}

/// A separator candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Sep {
    bytes: [u8; MAX_DELIM],
    len: u8,
    merge: bool,
}

impl Sep {
    fn one(b: u8) -> Sep {
        Sep {
            bytes: [b, 0, 0, 0],
            len: 1,
            merge: false,
        }
    }

    fn many(s: &[u8]) -> Sep {
        let mut bytes = [0; MAX_DELIM];
        bytes[..s.len()].copy_from_slice(s);
        Sep {
            bytes,
            len: s.len() as u8,
            merge: false,
        }
    }

    fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

/// The first 64 KB, cut after the last line break when the file goes on.
fn head_of(data: &[u8]) -> &[u8] {
    if data.len() <= HEAD_BYTES {
        return data;
    }
    let head = &data[..HEAD_BYTES];
    match memchr::memrchr2(b'\r', b'\n', head) {
        Some(p) => &head[..=p],
        None => head,
    }
}

/// Middle and end samples of a file much larger than the head, each cut to
/// whole lines.
fn parts_of(data: &[u8]) -> Vec<&[u8]> {
    if data.len() < HEAD_BYTES * 3 {
        return Vec::new();
    }
    let mid = data.len() / 2;
    [
        &data[mid..mid + PART_BYTES],
        &data[data.len() - PART_BYTES..],
    ]
    .into_iter()
    .filter_map(|part| {
        let first = memchr::memchr2(b'\r', b'\n', part)?;
        let mut start = first + 1;
        if part[first] == b'\r' && part.get(start) == Some(&b'\n') {
            start += 1;
        }
        let last = memchr::memrchr2(b'\r', b'\n', part)?;
        (start <= last).then(|| &part[start..=last])
    })
    .collect()
}

/// Physical lines: (start, content end, next start).
fn lines_of(buf: &[u8], max: usize) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < buf.len() && out.len() < max {
        match memchr::memchr2(b'\r', b'\n', &buf[pos..]) {
            Some(i) => {
                let end = pos + i;
                let next = if buf[end] == b'\r' && buf.get(end + 1) == Some(&b'\n') {
                    end + 2
                } else {
                    end + 1
                };
                out.push((pos, end, next));
                pos = next;
            }
            None => {
                out.push((pos, buf.len(), buf.len()));
                break;
            }
        }
    }
    out
}

/// The separator named by a `sep=X` line (Excel's convention).
fn sep_line(line: &[u8]) -> Option<Sep> {
    let line = line
        .strip_suffix(b"\"")
        .map_or(line, |l| l.strip_prefix(b"\"").unwrap_or(line));
    if line.len() < 5 || !line[..4].eq_ignore_ascii_case(b"sep=") {
        return None;
    }
    let s = &line[4..];
    (s.len() <= MAX_DELIM && Dialect::custom(s, None, None, false).is_ok()).then(|| Sep::many(s))
}

/// Per-byte counts over the first lines.
struct ByteCounts {
    /// Non-blank lines counted.
    lines: usize,
    /// Lines containing the byte.
    with: [u32; 128],
    /// Lines where the byte appears exactly as often as on the most lines.
    steady: [u32; 128],
    /// Lines with two or more spaces in a row.
    space_runs: u32,
}

impl ByteCounts {
    fn of(head: &[u8], lines: &[(usize, usize, usize)]) -> ByteCounts {
        let mut with = [0u32; 128];
        let mut per_line: Vec<[u16; 128]> = Vec::new();
        let mut space_runs = 0;
        for &(s, e, _) in lines.iter().filter(|l| l.1 > l.0).take(COUNT_LINES) {
            let mut c = [0u16; 128];
            for &b in &head[s..e] {
                if b < 128 {
                    c[b as usize] = c[b as usize].saturating_add(1);
                }
            }
            for (b, &n) in c.iter().enumerate() {
                if n > 0 {
                    with[b] += 1;
                }
            }
            if memchr::memmem::find(&head[s..e], b"  ").is_some() {
                space_runs += 1;
            }
            per_line.push(c);
        }
        let mut steady = [0u32; 128];
        for b in 0..128 {
            if with[b] == 0 {
                continue;
            }
            let mut v: Vec<u16> = per_line.iter().map(|c| c[b]).filter(|&n| n > 0).collect();
            v.sort_unstable();
            steady[b] = v
                .chunk_by(|a, b| a == b)
                .map(|r| r.len() as u32)
                .max()
                .unwrap_or(0);
        }
        ByteCounts {
            lines: per_line.len(),
            with,
            steady,
            space_runs,
        }
    }

    /// Share of lines that contain `b`, and that contain it a steady
    /// number of times.
    fn presence(&self, b: u8) -> (f32, f32) {
        if self.lines == 0 || b >= 128 {
            return (0.0, 0.0);
        }
        let n = self.lines as f32;
        (
            self.with[b as usize] as f32 / n,
            self.steady[b as usize] as f32 / n,
        )
    }

    /// Bytes that look like separators wherever they are (used to spot
    /// cells that still hold one).
    fn strong_separators(&self) -> Vec<u8> {
        (1u8..128)
            .filter(|&b| {
                (matches!(b, b',' | b';' | b'\t' | b'|' | b'^' | b'~') || is_control(b))
                    && self.presence(b).0 >= 0.6
            })
            .collect()
    }
}

fn is_control(b: u8) -> bool {
    b < 0x20 && !matches!(b, b'\t' | b'\r' | b'\n' | 0)
}

/// Separator candidates with their prior weight.
fn candidates(
    head: &[u8],
    lines: &[(usize, usize, usize)],
    counts: &ByteCounts,
) -> Vec<(Sep, f32)> {
    let mut out: Vec<(Sep, f32)> = b",\t;|".iter().map(|&b| (Sep::one(b), 1.0)).collect();
    // Other bytes on most lines, best first.
    let mut found: Vec<(u8, f32)> = (1u8..128)
        .filter(|&b| {
            !b",\t;| \"'".contains(&b)
                && Dialect::new(b, None).is_ok()
                && counts.presence(b).0 >= 0.3
        })
        .map(|b| {
            let (with, steady) = counts.presence(b);
            (b, with * steady)
        })
        .collect();
    found.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (b, _) in found.into_iter().take(8) {
        let prior = match b {
            _ if is_control(b) => 1.0,
            b':' | b'^' | b'~' => 0.9,
            b'.' | b'-' | b'/' | b'_' => 0.6,
            _ => 0.85,
        };
        out.push((Sep::one(b), prior));
    }
    let (spaces, _) = counts.presence(b' ');
    if spaces >= 0.8 {
        out.push((Sep::one(b' '), 0.75));
    }
    if counts.lines > 0 && counts.space_runs as f32 / counts.lines as f32 >= 0.5 {
        out.push((
            Sep {
                merge: true,
                ..Sep::one(b' ')
            },
            0.85,
        ));
    }
    for run in repeated_runs(head, lines, counts.lines) {
        out.push((Sep::many(&run), 0.95));
    }
    out
}

/// Runs of 2 to 4 punctuation bytes that appear on most lines.
fn repeated_runs(head: &[u8], lines: &[(usize, usize, usize)], n: usize) -> Vec<Vec<u8>> {
    // (run, lines seen on, last line index)
    let mut runs: Vec<(Vec<u8>, u32, usize)> = Vec::new();
    let part = |b: u8| b.is_ascii_punctuation() && b != b'"' && b != b'\'';
    for (li, &(s, e, _)) in lines
        .iter()
        .filter(|l| l.1 > l.0)
        .take(COUNT_LINES)
        .enumerate()
    {
        let line = &head[s..e];
        let mut i = 0;
        while i < line.len() {
            if !part(line[i]) {
                i += 1;
                continue;
            }
            let start = i;
            while i < line.len() && part(line[i]) {
                i += 1;
            }
            let run = &line[start..i];
            if (2..=MAX_DELIM).contains(&run.len()) {
                match runs.iter().position(|r| r.0 == run) {
                    Some(k) if runs[k].2 != li => {
                        runs[k].1 += 1;
                        runs[k].2 = li;
                    }
                    Some(_) => {}
                    None if runs.len() < 64 => runs.push((run.to_vec(), 1, li)),
                    None => {}
                }
            }
        }
    }
    runs.sort_by_key(|r| std::cmp::Reverse(r.1));
    // Most uses of the run's bytes must be inside the run (in `a:/b:/c:d`,
    // `:/` is not the separator: most colons stand alone).
    let mostly_whole = |run: &[u8]| {
        let (mut inside, mut all) = (0usize, 0usize);
        for &(s, e, _) in lines.iter().filter(|l| l.1 > l.0).take(COUNT_LINES) {
            let line = &head[s..e];
            inside += memchr::memmem::find_iter(line, run).count() * run.len();
            all += line.iter().filter(|b| run.contains(b)).count();
        }
        all > 0 && inside * 4 >= all * 3
    };
    runs.into_iter()
        .filter(|r| n > 0 && r.1 as f32 / n as f32 >= 0.6 && mostly_whole(&r.0))
        .take(3)
        .map(|r| r.0)
        .collect()
}

/// Quote and escape options to try, with prior weights.
fn quote_options(head: &[u8], fixed: Option<Option<u8>>) -> Vec<(Option<u8>, Option<u8>, f32)> {
    if let Some(q) = fixed {
        return vec![(q, None, 1.0)];
    }
    let mut out = vec![(Some(b'"'), None, 1.0)];
    if memchr::memmem::find(head, b"\\\"").is_some() {
        out.push((Some(b'"'), Some(b'\\'), 0.99));
    }
    if memchr::memchr(b'"', head).is_some() {
        out.push((None, None, 0.98));
    }
    if memchr::memchr(b'\'', head).is_some() {
        out.push((Some(b'\''), None, 0.93));
    }
    out
}

/// Most common column count of the rows from `start`, and the share of
/// rows that have it.
fn agreement(head: &[u8], start: usize, d: Dialect) -> (u32, f32) {
    let rows = split(head, start, d, MAX_ROWS, 0);
    let m = mode_of(rows.iter().map(|r| r.fields));
    if rows.is_empty() {
        return (m, 0.0);
    }
    let same = rows.iter().filter(|r| r.fields == m).count();
    (m, same as f32 / rows.len() as f32)
}

/// A split row: field count, repair flags, and the first fields' ranges.
struct Row {
    /// Start and content end of the row in the buffer.
    content: (usize, usize),
    fields: u32,
    repairs: u8,
    spans: Vec<(usize, usize)>,
}

/// Split up to `max` non-blank rows from `start`, keeping the ranges of
/// the first [`JUDGED_COLS`] fields of the first `keep_spans` rows.
fn split(buf: &[u8], start: usize, d: Dialect, max: usize, keep_spans: usize) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut pos = start;
    let mut mode = Mode::Normal;
    while pos < buf.len() && rows.len() < max {
        let keep = rows.len() < keep_spans;
        let start_of_row = pos;
        let mut spans = Vec::new();
        let mut empty = true;
        let end = scan::scan_row(buf, pos, d, mode, |s, e| {
            empty &= s == e;
            if keep && spans.len() < JUDGED_COLS {
                spans.push((s, e));
            }
            true
        });
        if end.unterminated {
            mode = Mode::LineBound;
            continue;
        }
        pos = end.next;
        if end.fields == 1 && empty {
            continue; // blank line
        }
        rows.push(Row {
            content: (start_of_row, end.content_end),
            fields: end.fields,
            repairs: end.repairs,
            spans,
        });
    }
    rows
}

fn mode_of(values: impl Iterator<Item = u32>) -> u32 {
    let mut v: Vec<u32> = values.collect();
    v.sort_unstable();
    let (mut best, mut run) = (0, 0);
    for r in v.chunk_by(|a, b| a == b) {
        // Ties go to more columns.
        if r.len() >= run {
            best = r[0];
            run = r.len();
        }
    }
    best
}

/// A fully judged candidate.
struct Eval {
    dialect: Dialect,
    skip: usize,
    columns: u32,
    /// How good the split looks, 0 to 1.
    quality: f32,
    /// Quality times the candidate's prior.
    score: f32,
    /// Something makes this reading doubtful even when it wins (a title
    /// set aside unmarked, values cut apart, columns that do not line up,
    /// sentences rather than a table): keep the confidence below the
    /// warning line.
    doubt: bool,
    /// Identifies the split, so identical splits count once.
    sig: u64,
}

fn evaluate(
    head: &[u8],
    lines: &[(usize, usize, usize)],
    parts: &[&[u8]],
    d: Dialect,
    pre: usize,
    others: &[u8],
    prior: f32,
) -> Eval {
    let line_start = |i: usize| lines.get(i).map_or(head.len(), |l| l.0);
    let mut skip = pre;
    let mut unmarked_skip = false;
    let mut rows = split(head, line_start(skip), d, MAX_ROWS, JUDGED_ROWS);
    let m = mode_of(rows.iter().map(|r| r.fields));

    // Lines above the table, before the first line with the table's column
    // count: blank lines, comments, lines with at most half the columns, and
    // lines with fewer columns that a blank line separates from the table.
    // (A header one column short, right above its rows, stays in the table.)
    if m >= 2 {
        let mut table = None;
        // (line, columns, comment) of the non-blank lines above it.
        let mut above: Vec<(usize, u32, bool)> = Vec::new();
        let mut last_blank = None;
        let end = lines.len().min(pre + MAX_PREAMBLE + 1);
        for (i, &(s, e, _)) in lines.iter().enumerate().take(end).skip(pre) {
            if e == s {
                last_blank = Some(i);
                continue;
            }
            let c = scan::scan_row(head, s, d, Mode::LineBound, |_, _| true).fields;
            if c == m {
                table = Some(i);
                break;
            }
            let line = &head[s..e];
            let comment = line.starts_with(b"#") || line.starts_with(b"//");
            if c > m && !comment {
                break;
            }
            above.push((i, c, comment));
        }
        let marked =
            |&(i, _, comment): &(usize, u32, bool)| comment || last_blank.is_some_and(|b| b > i);
        let fits = |l: &(usize, u32, bool)| marked(l) || l.1 == 1 || l.1 * 2 <= m;
        if let Some(t) = table.filter(|&t| t > pre && above.iter().all(|l| fits(l) && (l.1 < m))) {
            let again = split(head, line_start(t), d, MAX_ROWS, JUDGED_ROWS);
            if again.len() >= 2 {
                // Setting aside a line that nothing marks as a title (no
                // blank line after it, not a comment) is less certain: it
                // may be the header of another reading.
                unmarked_skip = !above.iter().all(marked);
                skip = t;
                rows = again;
            }
        }
    }
    let base = judge_split(head, parts, d, skip, rows, unmarked_skip, others, prior);
    if skip != pre || base.columns < 2 {
        return base;
    }

    // A short title with as many columns as the table, then a blank line,
    // then a table without blank lines: compare reading it as a title.
    let nonblank = |i: usize| lines.get(i).is_some_and(|l| l.1 > l.0);
    let Some(first) = (pre..lines.len().min(pre + 4)).find(|&i| nonblank(i)) else {
        return base;
    };
    let Some(blank) = (first + 1..lines.len().min(first + 4)).find(|&i| !nonblank(i)) else {
        return base;
    };
    let Some(t) = (blank..lines.len().min(blank + 4)).find(|&i| nonblank(i)) else {
        return base;
    };
    let table_has_blanks =
        (t..lines.len().min(t + 12)).any(|i| !nonblank(i) && i + 1 < lines.len());
    if table_has_blanks {
        return base;
    }
    let again = split(head, line_start(t), d, MAX_ROWS, JUDGED_ROWS);
    if again.len() < 3 {
        return base;
    }
    let alt = judge_split(head, parts, d, t, again, false, others, prior);
    if alt.columns == base.columns && alt.quality > base.quality * 1.03 {
        alt
    } else {
        base
    }
}

/// Judge one split of the head (rows from line `skip`).
#[allow(clippy::too_many_arguments)]
fn judge_split(
    head: &[u8],
    parts: &[&[u8]],
    d: Dialect,
    skip: usize,
    rows: Vec<Row>,
    unmarked_skip: bool,
    others: &[u8],
    prior: f32,
) -> Eval {
    let m = mode_of(rows.iter().map(|r| r.fields));
    let n = rows.len().max(1) as f32;
    // Reasons to keep the confidence below the warning line even when this
    // reading wins.
    let mut doubt = unmarked_skip;
    // Rows one or more columns short (ragged files) count half.
    let fit = |r: &Row| match r.fields {
        f if f == m => 1.0,
        f if f < m && f >= 2 => 0.5,
        f if f < m => 0.25,
        _ => 0.0,
    };
    let head_agree = rows.iter().map(fit).sum::<f32>() / n;
    let mut agree = head_agree;
    let mut rest = (0usize, 0f32);
    for part in parts {
        for r in split(part, 0, d, 100, 0) {
            rest.0 += 1;
            rest.1 += fit(&r);
        }
    }
    if rest.0 >= 3 {
        agree = 0.75 * head_agree + 0.25 * rest.1 / rest.0 as f32;
    }

    // Whole lines that are one value: a separator that is also part of
    // values cuts them apart ("1.5" read with '.', "2024-01-31" or
    // "555-123-4567" with '-'). A comma may be a decimal comma ("1,2"):
    // then the reading is only doubtful.
    let sep = d.delimiter();
    let part_of_values = matches!(sep, b"." | b"-" | b"/" | b":" | b" ");
    let one_value = |r: &Row| {
        let t = head[r.content.0..r.content.1].trim_ascii();
        if part_of_values {
            is_number(t) || is_date_or_time(t) || (sep != b" " && is_code(t))
        } else {
            sep == b"," && is_number(t)
        }
    };
    let whole = rows.iter().filter(|r| one_value(r)).count() as f32 / n;
    if whole >= 1.0 && !part_of_values {
        doubt = true; // every line could be one number with a decimal comma
    }
    // Numbers cut apart inside rows: "10:00" read with ':' leaves "10" and
    // "00" in neighbouring cells.
    let with_spans = rows.iter().filter(|r| !r.spans.is_empty()).count().max(1);
    let cut_numbers = if matches!(sep, b":" | b"." | b"-" | b"/") {
        rows.iter()
            .filter(|r| {
                r.spans.windows(2).any(|w| {
                    let a = &head[w[0].0..w[0].1];
                    let b = &head[w[1].0..w[1].1];
                    let tail = a.iter().rev().take_while(|c| c.is_ascii_digit()).count();
                    let lead = b.iter().take_while(|c| c.is_ascii_digit()).count();
                    (1..=4).contains(&tail)
                        && lead == 2
                        && b.get(2)
                            .is_none_or(|c| !c.is_ascii_alphanumeric() || *c == b' ')
                })
            })
            .count() as f32
            / with_spans as f32
    } else {
        0.0
    };
    if cut_numbers >= 0.5 {
        doubt = true;
    }
    // Quotes off, yet a cell opens a quote that a later cell of the same
    // row closes: a quoted value was cut apart at a separator.
    let cut_quotes = b"\"'"
        .iter()
        .copied()
        .filter(|&q| Some(q) != d.quote())
        .map(|q| {
            rows.iter()
                .filter(|r| {
                    let cell = |&(s, e): &(usize, usize)| head[s..e].trim_ascii();
                    let open = r.spans.iter().position(|sp| {
                        let c = cell(sp);
                        c.len() > 1 && c[0] == q && c[c.len() - 1] != q
                    });
                    open.is_some_and(|i| {
                        r.spans[i + 1..].iter().any(|sp| {
                            let c = cell(sp);
                            c.last() == Some(&q) && c.first() != Some(&q)
                        })
                    })
                })
                .count() as f32
                / with_spans as f32
        })
        .fold(0f32, f32::max);

    let broken = rows
        .iter()
        .filter(|r| r.repairs & (scan::TEXT_AFTER_QUOTE | scan::UNCLOSED_QUOTE) != 0)
        .count() as f32
        / n;
    let long = rows.iter().any(|r| r.repairs & scan::LONG_QUOTED_CELL != 0);

    // Columns lined up with spaces must line up: each field starts or ends
    // where the others in its column do. A blank cell shifts the fields
    // after it, which merged spaces cannot see.
    let mut aligned = 1.0;
    if d.merges() {
        let with = rows.iter().filter(|r| !r.spans.is_empty());
        let mut starts: Vec<Vec<usize>> = vec![Vec::new(); (m as usize).min(JUDGED_COLS)];
        let mut ends = starts.clone();
        let mut offsets: Vec<Option<Vec<(usize, usize)>>> = Vec::new();
        for r in with {
            if r.fields != m {
                offsets.push(None);
                continue;
            }
            let row = &head[r.content.0..r.content.1];
            let col = |byte: usize| row[..byte].iter().filter(|&&b| b & 0xC0 != 0x80).count();
            let o: Vec<(usize, usize)> = r
                .spans
                .iter()
                .map(|&(s, e)| (col(s - r.content.0), col(e - r.content.0)))
                .collect();
            for (j, &(s, e)) in o.iter().enumerate().take(starts.len()) {
                starts[j].push(s);
                ends[j].push(e);
            }
            offsets.push(Some(o));
        }
        let most = |v: &mut Vec<usize>| {
            v.sort_unstable();
            v.chunk_by(|a, b| a == b)
                .max_by_key(|r| r.len())
                .map_or(usize::MAX, |r| r[0])
        };
        let s_mode: Vec<usize> = starts.iter_mut().map(most).collect();
        let e_mode: Vec<usize> = ends.iter_mut().map(most).collect();
        let total = offsets.len().max(1);
        let good = offsets
            .iter()
            .filter(|o| {
                o.as_ref().is_some_and(|o| {
                    o.iter()
                        .zip(s_mode.iter().zip(&e_mode))
                        // Overlaps the column's usual place (left- or
                        // right-aligned values both do).
                        .all(|(&(s, e), (&ms, &me))| s < me.max(ms + 1) && e > ms)
                })
            })
            .count();
        aligned = good as f32 / total as f32;
        if aligned < 0.9 {
            doubt = true;
        }
    }

    // Judge the cells below the first row (which may be titles).
    let width = (m as usize).min(JUDGED_COLS);
    let mut kinds = vec![[0u32; KINDS]; width];
    let mut empties = vec![0u32; width];
    let (mut plaus, mut cells) = (0f32, 0u32);
    let (mut prose, mut filled_cells, mut quoted_cells) = (0u32, 0u32, 0u32);
    let mut words = 0usize;
    let judged: Vec<&Row> = rows
        .iter()
        .skip(1)
        .filter(|r| !r.spans.is_empty())
        .collect();
    for r in &judged {
        for (c, &(s, e)) in r.spans.iter().enumerate() {
            let raw = &head[s..e];
            let (kind, p) = judge(raw, d, others);
            plaus += p;
            cells += 1;
            if kind != Kind::Empty {
                filled_cells += 1;
            }
            if d.quote().is_some_and(|q| raw.first() == Some(&q)) {
                quoted_cells += 1;
            } else if kind != Kind::Empty {
                let w = raw.split(|&b| b == b' ').filter(|w| !w.is_empty()).count();
                words += w;
                if kind == Kind::Text && w >= 3 {
                    prose += 1;
                }
            }
            if c < width {
                kinds[c][kind as usize] += 1;
                if kind == Kind::Empty {
                    empties[c] += 1;
                }
            }
        }
    }
    let plaus = if cells == 0 {
        0.7
    } else {
        plaus / cells as f32
    };
    let mut typed = (0f32, 0u32);
    for k in &kinds {
        let filled: u32 = k[1..].iter().sum();
        if filled >= 2 {
            typed.0 += *k[1..].iter().max().unwrap_or(&0) as f32 / filled as f32;
            typed.1 += 1;
        }
    }
    let typed = if typed.1 == 0 {
        0.7
    } else {
        typed.0 / typed.1 as f32
    };
    // Sentences split at their commas: mostly several-word text, no column
    // that holds another kind of value, nothing quoted.
    let all_text = kinds.iter().all(|k| {
        let best = (1..KINDS)
            .max_by_key(|&i| k[i])
            .unwrap_or(Kind::Text as usize);
        k[1..].iter().sum::<u32>() == 0 || best == Kind::Text as usize
    });
    let several_words = words >= 2 * filled_cells as usize && prose > 0;
    if filled_cells > 0
        && quoted_cells == 0
        && all_text
        && (prose * 2 >= filled_cells || several_words)
    {
        doubt = true;
    }
    // Columns that are (almost) always empty, not counting the last (a
    // trailing separator is common).
    let empty_cols = if judged.len() >= 2 && width >= 3 {
        let limit = (judged.len() as f32 * 0.9) as u32;
        empties[..width - 1]
            .iter()
            .filter(|&&n| n >= limit.max(1))
            .count() as f32
            / (width - 1) as f32
    } else {
        0.0
    };

    let quality = agree
        * agree
        * (0.35 + 0.65 * plaus)
        * (0.75 + 0.25 * typed)
        * (1.0 - 0.8 * empty_cols)
        * (1.0 - broken)
        * if long { 0.5 } else { 1.0 }
        * if whole >= 0.5 && part_of_values {
            0.2
        } else {
            1.0
        }
        * if unmarked_skip { 0.85 } else { 1.0 }
        * (1.0 - 0.8 * cut_quotes)
        * (1.0 - 0.6 * cut_numbers)
        * (0.5 + 0.5 * aligned);

    let mut h = DefaultHasher::new();
    skip.hash(&mut h);
    for r in rows.iter().take(JUDGED_ROWS) {
        r.fields.hash(&mut h);
        r.spans.hash(&mut h);
    }
    Eval {
        dialect: d,
        skip,
        columns: m,
        quality,
        score: quality * prior,
        doubt,
        sig: h.finish(),
    }
}

/// Digit groups joined by dashes, dots, slashes, spaces or brackets, such as
/// phone numbers, part codes or addresses ("555-123-4567", "10.0.0.1",
/// "(555) 123 4567", "+44 20 7946 0000").
fn is_code(t: &[u8]) -> bool {
    let groups = t
        .split(|b| !b.is_ascii_digit())
        .filter(|g| !g.is_empty())
        .count();
    groups >= 2
        && t.first()
            .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'(' | b'+'))
        && t.iter().all(|b| {
            b.is_ascii_digit() || matches!(b, b'-' | b'.' | b'/' | b' ' | b'(' | b')' | b'+')
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Empty = 0,
    Number,
    Date,
    Flag,
    Text,
}
const KINDS: usize = 5;

/// What a cell holds, and how plausible it is as a whole cell (0 to 1).
fn judge(raw: &[u8], d: Dialect, others: &[u8]) -> (Kind, f32) {
    let quoted = d.quote().is_some_and(|q| raw.first() == Some(&q));
    let value = scan::unquote(raw, d);
    let t = value.trim_ascii();
    if t.is_empty() {
        return (Kind::Empty, 0.75);
    }
    let mut p = 1.0f32;
    if !quoted && !d.merges() && t.len() != value.len() && d.delimiter() != b" " {
        p *= 0.8; // spaces around the value
    }
    // Quote marks this dialect does not treat as quotes.
    if !quoted && raw.len() >= 2 {
        for c in *b"\"'" {
            if Some(c) == d.quote() {
                continue;
            }
            let t = raw.trim_ascii();
            if t.len() >= 2 && t[0] == c && t[t.len() - 1] == c {
                p *= 0.3; // looks quoted, but not with this quote
            } else if t.iter().filter(|&&b| b == c).count() % 2 == 1 {
                // A piece of a quoted value cut at a separator (an
                // apostrophe may be just that, so it weighs less).
                p *= if c == b'"' { 0.5 } else { 0.75 };
            }
        }
    }
    let kind = if is_number(t) {
        Kind::Number
    } else if is_date_or_time(t) {
        Kind::Date
    } else if is_flag(t) {
        Kind::Flag
    } else {
        Kind::Text
    };
    if kind == Kind::Text {
        p *= 0.85;
        if !quoted {
            // (A comma followed by a space is ordinary writing.)
            let held = t.iter().enumerate().any(|(i, b)| {
                others.contains(b) && !d.delimiter().contains(b) && t.get(i + 1) != Some(&b' ')
            });
            if held {
                p *= 0.45; // still holds another likely separator
            }
            if let Some(q) = d.quote() {
                if t.contains(&q) {
                    p *= 0.7; // stray quote
                }
            }
            if odd_edge(t[0]) || odd_edge(t[t.len() - 1]) {
                p *= 0.5; // starts or ends with a leftover piece of a separator
            }
            if t.len() > 120 {
                p *= 0.7;
            }
        }
    }
    (kind, p)
}

/// Punctuation that rarely starts or ends a value but often is part of a
/// separator.
fn odd_edge(b: u8) -> bool {
    matches!(b, b'|' | b'^' | b'~' | b';' | b':' | b',' | b'\t' | b'\\') || is_control(b)
}

/// Numbers: optional sign, currency or brackets, digits with thousands
/// groups, a decimal point or comma, an exponent, a percent sign.
fn is_number(t: &[u8]) -> bool {
    let mut s = t;
    if s.len() >= 3 && s[0] == b'(' && s[s.len() - 1] == b')' {
        s = &s[1..s.len() - 1];
    }
    s = s
        .strip_prefix(b"+")
        .or_else(|| s.strip_prefix(b"-"))
        .unwrap_or(s);
    s = s.strip_prefix(b"$").unwrap_or(s);
    s = s.strip_prefix(b"-").unwrap_or(s);
    s = s.strip_suffix(b"%").unwrap_or(s);
    // Exponent.
    if let Some(i) = s.iter().position(|&b| b == b'e' || b == b'E') {
        let exp = &s[i + 1..];
        let exp = exp
            .strip_prefix(b"+")
            .or_else(|| exp.strip_prefix(b"-"))
            .unwrap_or(exp);
        if exp.is_empty() || !exp.iter().all(u8::is_ascii_digit) {
            return false;
        }
        s = &s[..i];
    }
    if s.is_empty() {
        return false;
    }
    // Digit groups split by '.' or ','.
    let groups: Vec<&[u8]> = s.split(|&b| b == b'.' || b == b',').collect();
    if !groups.iter().all(|g| g.iter().all(u8::is_ascii_digit)) {
        return false;
    }
    if !groups.iter().any(|g| !g.is_empty()) {
        return false;
    }
    match groups.len() {
        1 => true,
        // "12.5", "1,50", ".5", "5."
        2 => !groups[0].is_empty() || !groups[1].is_empty(),
        // Thousands groups, then maybe a decimal part: every inner group
        // is three digits and the first is one to three.
        _ => {
            let seps: Vec<u8> = s
                .iter()
                .copied()
                .filter(|&b| b == b'.' || b == b',')
                .collect();
            let last_differs = seps[seps.len() - 1] != seps[0];
            let inner_end = if last_differs {
                groups.len() - 1
            } else {
                groups.len()
            };
            let thousands_same = seps[..inner_end - 1].iter().all(|&b| b == seps[0]);
            (1..=3).contains(&groups[0].len())
                && thousands_same
                && groups[1..inner_end].iter().all(|g| g.len() == 3)
                && (!last_differs || !groups[groups.len() - 1].is_empty())
        }
    }
}

const MONTHS: [&[u8]; 12] = [
    b"jan", b"feb", b"mar", b"apr", b"may", b"jun", b"jul", b"aug", b"sep", b"oct", b"nov", b"dec",
];

fn is_month(w: &[u8]) -> bool {
    w.len() >= 3 && MONTHS.iter().any(|m| w[..3].eq_ignore_ascii_case(m)) && w.len() <= 9
}

/// Dates such as 2024-01-31, 31/01/2024, 31.01.24, 5-Jan-2024, Jan 5 2024,
/// optionally followed by a time; or a time on its own (10:30, 10:30:15.5 PM).
fn is_date_or_time(t: &[u8]) -> bool {
    let (date, rest) = date_part(t);
    if date {
        let rest = rest.trim_ascii_start();
        let rest = rest.strip_prefix(b"T").unwrap_or(rest);
        return rest.is_empty() || is_time(rest);
    }
    is_time(t)
}

/// Whether `t` starts with a date, and what follows it.
fn date_part(t: &[u8]) -> (bool, &[u8]) {
    let mut i = 0;
    let mut tokens: Vec<(&[u8], bool)> = Vec::new(); // (text, is digits)
    let mut sep = None;
    while tokens.len() < 3 && i < t.len() {
        let start = i;
        if t[i].is_ascii_digit() {
            while i < t.len() && t[i].is_ascii_digit() {
                i += 1;
            }
            tokens.push((&t[start..i], true));
        } else if t[i].is_ascii_alphabetic() {
            while i < t.len() && t[i].is_ascii_alphabetic() {
                i += 1;
            }
            tokens.push((&t[start..i], false));
        } else {
            return (false, t);
        }
        if tokens.len() < 3 {
            // One separator between parts; "Jan 5, 2024" allows ", ".
            let s = t.get(i).copied();
            if !matches!(s, Some(b'-' | b'/' | b'.' | b' ' | b',')) {
                return (false, t);
            }
            if s != Some(b',') && sep.is_some_and(|p| Some(p) != s) && s != Some(b' ') {
                return (false, t);
            }
            sep = sep.or(s);
            i += 1;
            if s == Some(b',') && t.get(i) == Some(&b' ') {
                i += 1;
            }
        }
    }
    if tokens.len() != 3 {
        return (false, t);
    }
    let len = |k: usize| tokens[k].0.len();
    let ok = match (tokens[0].1, tokens[1].1, tokens[2].1) {
        (true, true, true) => {
            (len(0) == 4 && len(1) <= 2 && len(2) <= 2)
                || (len(0) <= 2 && len(1) <= 2 && (len(2) == 2 || len(2) == 4))
        }
        (true, false, true) => len(0) <= 2 && is_month(tokens[1].0) && matches!(len(2), 2 | 4),
        (false, true, true) => is_month(tokens[0].0) && len(1) <= 2 && matches!(len(2), 2 | 4),
        _ => false,
    };
    // The year must end the date part (no "2024-01-05x").
    if ok
        && t.get(i)
            .is_some_and(|b| b.is_ascii_alphanumeric() && *b != b'T')
    {
        return (false, t);
    }
    (ok, &t[i..])
}

/// 10:30, 10:30:15, 10:30:15.123, with optional AM/PM and a zone (Z, +02:00).
fn is_time(t: &[u8]) -> bool {
    let mut i = 0;
    let digits = |i: &mut usize, max: usize| {
        let s = *i;
        while *i < t.len() && t[*i].is_ascii_digit() && *i - s < max {
            *i += 1;
        }
        *i - s
    };
    if !(1..=2).contains(&digits(&mut i, 2)) || t.get(i) != Some(&b':') {
        return false;
    }
    i += 1;
    if digits(&mut i, 2) != 2 {
        return false;
    }
    if t.get(i) == Some(&b':') {
        i += 1;
        if digits(&mut i, 2) != 2 {
            return false;
        }
        if t.get(i) == Some(&b'.') {
            i += 1;
            if digits(&mut i, 9) == 0 {
                return false;
            }
        }
    }
    let rest = t[i..].trim_ascii_start();
    rest.is_empty()
        || rest.eq_ignore_ascii_case(b"am")
        || rest.eq_ignore_ascii_case(b"pm")
        || rest == b"Z"
        || (rest.len() == 6
            && matches!(rest[0], b'+' | b'-')
            && rest[1..3].iter().all(u8::is_ascii_digit)
            && rest[3] == b':'
            && rest[4..].iter().all(u8::is_ascii_digit))
}

fn is_flag(t: &[u8]) -> bool {
    [
        &b"true"[..],
        b"false",
        b"yes",
        b"no",
        b"y",
        b"n",
        b"t",
        b"f",
        b"on",
        b"off",
    ]
    .iter()
    .any(|w| t.eq_ignore_ascii_case(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn best(text: &[u8]) -> Choice {
        detect(text, Hints::default()).best().clone()
    }

    #[test]
    fn numbers() {
        for ok in [
            &b"12"[..],
            b"-3.5",
            b"1,50",
            b"1,234.56",
            b"1.234,56",
            b"(1,234.00)",
            b"$12.00",
            b"45%",
            b"6.02e23",
            b".5",
            b"1,234,567",
        ] {
            assert!(is_number(ok), "{}", String::from_utf8_lossy(ok));
        }
        for bad in [
            &b"1.2.3"[..],
            b"abc",
            b"12a",
            b"10.0.0.1",
            b"1,23,4",
            b"",
            b"-",
            b"e5",
        ] {
            assert!(!is_number(bad), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn dates_and_times() {
        for ok in [
            &b"2024-01-31"[..],
            b"31/01/2024",
            b"31.01.24",
            b"5-Jan-2024",
            b"Jan 5, 2024",
            b"2024-01-31T10:30:00Z",
            b"2024-01-31 10:30",
            b"10:30",
            b"10:30:15.25 PM",
            b"2024-01-31T10:30:00+02:00",
        ] {
            assert!(is_date_or_time(ok), "{}", String::from_utf8_lossy(ok));
        }
        for bad in [
            &b"2024"[..],
            b"1-2",
            b"hello",
            b"10:3",
            b"2024-01-31x",
            b"12/2024/1",
        ] {
            assert!(!is_date_or_time(bad), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn plain_files() {
        assert_eq!(best(b"a,b,c\n1,2,3\n4,5,6\n").dialect, Dialect::CSV);
        assert_eq!(best(b"a\tb\r\n1\t2\r\n").dialect, Dialect::TSV);
        let c = best(b"name;price\nfoo;1,50\nbar;2,75\nbaz;3\n");
        assert_eq!(c.dialect.delimiter(), b";");
    }

    #[test]
    fn single_column() {
        let d = detect(b"alpha\nbeta\ngamma\n", Hints::default());
        assert_eq!(d.best().columns, 1);
        assert!(d.is_confident());
        let d = detect(
            b"",
            Hints {
                ext: Some("tsv"),
                quote: None,
            },
        );
        assert_eq!(d.best().dialect.delimiter(), b"\t");
    }

    #[test]
    fn sep_line_and_preamble() {
        let c = best(b"sep=;\na;b\n1;2\n");
        assert_eq!((c.dialect.delimiter(), c.skip_lines), (&b";"[..], 1));
        let c = best(b"Monthly report, all regions\n\nregion,units,price\nnorth,4,1.5\nsouth,7,2.25\neast,1,9\n");
        assert_eq!(
            (c.dialect.delimiter(), c.skip_lines, c.columns),
            (&b","[..], 2, 3)
        );
    }

    #[test]
    fn skip_lines_len_counts_breaks() {
        assert_eq!(skip_lines_len(b"a\r\nb\nc\rd", 3), 7);
        assert_eq!(skip_lines_len(b"a\nb", 5), 3);
        assert_eq!(skip_lines_len(b"a\nb", 0), 0);
    }

    #[test]
    fn hostile_inputs_stay_bounded() {
        // One huge line of separators, quotes everywhere, control bytes.
        let inputs: Vec<Vec<u8>> = vec![
            vec![b','; 3 << 20],
            vec![b'"'; 3 << 20],
            b"\"a".repeat(1 << 20),
            (0..=255u8).cycle().take(3 << 20).collect(),
            b"|~|".repeat(1 << 20),
            b"  \n".repeat(1 << 20),
        ];
        for data in inputs {
            let t = std::time::Instant::now();
            let d = detect(&data, Hints::default());
            assert!(!d.choices.is_empty());
            assert!(t.elapsed().as_millis() < 2000, "took {:?}", t.elapsed());
        }
    }
}
